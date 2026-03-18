//! Local HTTP API for IPC with authentication, CORS, and rate limiting

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        ConnectInfo, FromRequestParts, Multipart, Path, Query, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{any, get, post},
    Json, Router,
};
use chrono::{Datelike, Timelike};
use futures::{SinkExt, StreamExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as TungsteniteMessage};
use tower_http::cors::{AllowOrigin, CorsLayer};

mod actions;
mod auth;
mod integrations;
mod moltbook;
mod observability;
mod suggestions;
mod trace;
mod tunnel;
mod tunnel_auth;

pub(crate) use self::actions::import_action_from_url_shared;
use self::moltbook::MoltbookSettings;

use crate::core::config::{
    DeploymentMode, TelegramConfig, TunnelCloudflareConfig, TunnelConfig, TunnelNgrokConfig,
    TunnelProviderKind, TunnelTailscaleConfig,
};
use crate::core::{
    score_action_risk, Agent, AutonomySettings, AutopilotMode, ConversationScope, ExecutionTrace,
    LlmProvider, ModelRole, ModelSlot, RecommendedAction, RiskEnvelope, RiskLevel, Task,
    TaskApproval, TaskQueue, TaskStatus, TrustPolicy, UserProfile,
};
use crate::hooks;
use crate::memory::MemoryType;

type SharedAgent = Arc<RwLock<Agent>>;
const FRONTEND_DIST_DIR: &str = "frontend/dist";
const DEFAULT_RATE_LIMIT_MAX_TRACKED_IPS: usize = 4096;
const SERVER_BUSY_TRACE_WINDOW_SECS: i64 = 20 * 60;
const SERVER_BUSY_TASK_WINDOW_SECS: i64 = 20 * 60;
static MISSING_API_KEY_WARNED: AtomicBool = AtomicBool::new(false);
static CHAT_SUGGESTION_SCAN_ACTIVE: AtomicBool = AtomicBool::new(false);
static CODEX_OAUTH_RUNTIME: OnceLock<Arc<RwLock<CodexOAuthRuntimeState>>> = OnceLock::new();
const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpServerRole {
    ControlPlane,
    PublicApps,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Clone)]
enum NotificationControlCommand {
    Pause24h,
    Resume,
    Status,
}

#[derive(Debug, Clone)]
enum AutonomyQuickCommand {
    TriageInbox,
    Delegate {
        task: String,
        require_approval: bool,
    },
    Rollback {
        event_id: String,
        operation: Option<String>,
    },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum McpTransportRequest {
    Http {
        url: String,
    },
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum McpAuthRequest {
    None {},
    Bearer {
        #[serde(default)]
        header: Option<String>,
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        clear: bool,
    },
    Basic {
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        clear: bool,
    },
    Header {
        name: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        clear: bool,
    },
    Query {
        name: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        clear: bool,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct McpServerRequest {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    description: Option<String>,
    transport: McpTransportRequest,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    resources_enabled: bool,
    #[serde(default)]
    auth: Option<McpAuthRequest>,
    #[serde(default)]
    tool_allowlist: Vec<String>,
    #[serde(default)]
    resource_allowlist: Vec<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    max_response_bytes: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct CodexOAuthRuntimeState {
    active: bool,
    auth_url: Option<String>,
    device_code: Option<String>,
    device_auth_id: Option<String>,
    user_code: Option<String>,
    poll_interval_secs: u64,
    last_output: String,
    last_error: Option<String>,
}
const OPENAI_DEVICE_AUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const OPENAI_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const OPENAI_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
const OPENAI_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

fn codex_oauth_runtime() -> Arc<RwLock<CodexOAuthRuntimeState>> {
    CODEX_OAUTH_RUNTIME
        .get_or_init(|| Arc::new(RwLock::new(CodexOAuthRuntimeState::default())))
        .clone()
}

struct ChatSuggestionScanGuard;

impl Drop for ChatSuggestionScanGuard {
    fn drop(&mut self) {
        CHAT_SUGGESTION_SCAN_ACTIVE.store(false, Ordering::Release);
    }
}
fn try_start_chat_suggestion_scan() -> Option<ChatSuggestionScanGuard> {
    if CHAT_SUGGESTION_SCAN_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        Some(ChatSuggestionScanGuard)
    } else {
        None
    }
}

/// Manages the embedded WhatsApp bridge (Node.js) process lifecycle.
/// Started/stopped from Settings UI when user enables/disables WhatsApp.
pub struct WhatsAppBridgeState {
    /// Child process handle
    process: Option<tokio::process::Child>,
    /// Whether the bridge is actively running
    pub active: bool,
    /// Error message if bridge failed
    pub error: Option<String>,
}

impl WhatsAppBridgeState {
    pub fn new() -> Self {
        Self {
            process: None,
            active: false,
            error: None,
        }
    }
}

// - Rate Limiter -

/// Simple in-memory rate limiter: max requests per window per IP
#[derive(Clone)]
pub struct RateLimiter {
    /// Map of IP address -> list of request timestamps
    requests: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    /// Maximum requests allowed per window
    max_requests: usize,
    /// Window duration
    window: std::time::Duration,
    /// Maximum distinct IPs tracked to cap memory
    max_tracked_ips: usize,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: std::time::Duration) -> Self {
        let max_tracked_ips = std::env::var("AGENTARK_RATE_LIMITER_MAX_IPS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_RATE_LIMIT_MAX_TRACKED_IPS);
        Self {
            requests: Arc::new(RwLock::new(HashMap::new())),
            max_requests,
            window,
            max_tracked_ips,
        }
    }

    fn evict_if_needed(
        map: &mut HashMap<String, Vec<Instant>>,
        now: Instant,
        window: std::time::Duration,
        max_tracked_ips: usize,
    ) {
        if map.len() < max_tracked_ips {
            return;
        }

        // First try to free space by dropping fully expired buckets.
        map.retain(|_ip, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });

        if map.len() < max_tracked_ips {
            return;
        }

        // Still full: evict least-recently-seen IP.
        if let Some(oldest_ip) = map
            .iter()
            .min_by_key(|(_ip, timestamps)| timestamps.last().copied().unwrap_or(now))
            .map(|(ip, _)| ip.clone())
        {
            map.remove(&oldest_ip);
        }
    }

    /// Check whether the given IP is within the rate limit.
    /// Returns `true` if the request is allowed, `false` if rate-limited.
    pub async fn check_rate_limit(&self, ip: &str) -> bool {
        let now = Instant::now();
        let mut map = self.requests.write().await;

        if !map.contains_key(ip) && map.len() >= self.max_tracked_ips {
            Self::evict_if_needed(&mut map, now, self.window, self.max_tracked_ips);
        }

        let timestamps = map.entry(ip.to_string()).or_insert_with(Vec::new);

        // Remove timestamps outside the current window
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests {
            false
        } else {
            timestamps.push(now);
            true
        }
    }

    /// Remove expired entries to free memory
    pub async fn cleanup(&self) {
        let now = Instant::now();
        let mut map = self.requests.write().await;
        map.retain(|_ip, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            !timestamps.is_empty()
        });
        while map.len() > self.max_tracked_ips {
            if let Some(oldest_ip) = map
                .iter()
                .min_by_key(|(_ip, timestamps)| timestamps.last().copied().unwrap_or(now))
                .map(|(ip, _)| ip.clone())
            {
                map.remove(&oldest_ip);
            } else {
                break;
            }
        }
    }
}

/// Tiered rate limiter with different limits per route prefix
#[derive(Clone)]
pub struct TieredRateLimiter {
    chat_limiter: RateLimiter,
    approval_limiter: RateLimiter,
    settings_limiter: RateLimiter,
    action_limiter: RateLimiter,
    default_limiter: RateLimiter,
}

impl TieredRateLimiter {
    pub fn new() -> Self {
        let min = std::time::Duration::from_secs(60);
        Self {
            chat_limiter: RateLimiter::new(60, min),
            approval_limiter: RateLimiter::new(30, min),
            settings_limiter: RateLimiter::new(60, min), // Settings is a local config read - no need to throttle aggressively
            action_limiter: RateLimiter::new(30, min),
            default_limiter: RateLimiter::new(120, min),
        }
    }

    pub async fn cleanup_all(&self) {
        self.chat_limiter.cleanup().await;
        self.approval_limiter.cleanup().await;
        self.settings_limiter.cleanup().await;
        self.action_limiter.cleanup().await;
        self.default_limiter.cleanup().await;
    }

    pub fn select_for_path(&self, path: &str) -> &RateLimiter {
        if path.starts_with("/chat") {
            &self.chat_limiter
        } else if path.contains("/approve") || path.contains("/reject") {
            &self.approval_limiter
        } else if path.starts_with("/settings")
            || path.starts_with("/models")
            || path.starts_with("/mcp/servers")
        {
            &self.settings_limiter
        } else if path.starts_with("/skills") {
            &self.action_limiter
        } else {
            &self.default_limiter
        }
    }
}

/// Shared application state - allows accessing some data without locking the agent
#[derive(Clone)]
pub struct AppState {
    /// Full agent (requires lock for most operations)
    pub agent: SharedAgent,
    /// Trace history - can be read without locking agent
    pub trace_history: Arc<RwLock<Vec<ExecutionTrace>>>,
    /// Current trace - can be read without locking agent
    pub last_trace: Arc<RwLock<ExecutionTrace>>,
    /// Task queue - can be read without locking agent
    pub tasks: Arc<RwLock<TaskQueue>>,
    /// User profile - can be read without locking agent
    pub user_profile: Arc<RwLock<UserProfile>>,
    /// Tiered rate limiter for all endpoints
    pub tiered_rate_limiter: TieredRateLimiter,
    /// HTTP API key for authentication (None = blocked unless insecure override is enabled)
    pub api_key: Arc<RwLock<Option<String>>>,
    /// HTTP API key expiry (unix timestamp seconds)
    pub api_key_expires_at: Arc<RwLock<Option<i64>>>,
    /// Explicitly allow protected routes without auth when API key is missing (dangerous).
    pub allow_insecure_no_auth: bool,
    /// Session token for web UI cookie-based auth. Created lazily once API auth exists.
    pub session_token: Arc<RwLock<Option<String>>>,
    /// Whether public local UI bootstrap is allowed without an API key prompt.
    pub local_ui_bootstrap_enabled: bool,
    /// One-time bootstrap tokens for local UI session creation.
    pub local_ui_bootstrap_tokens: Arc<RwLock<HashMap<String, i64>>>,
    /// If true, UI session cookies are marked Secure by default (for HTTPS deployments).
    pub cookie_secure_default: bool,
    /// Short-lived OAuth state tokens for browser callbacks.
    oauth_states: Arc<RwLock<HashMap<String, PendingOAuthState>>>,
    /// Failed remote tunnel login attempts keyed by client IP.
    pub remote_login_attempts: Arc<RwLock<HashMap<String, (u32, Instant)>>>,
    tunnel: Arc<RwLock<tunnel::TunnelState>>,
    /// WhatsApp bridge state (embedded Node.js process, managed from Settings UI)
    pub whatsapp_bridge: Arc<RwLock<WhatsAppBridgeState>>,
    /// Security event counters (shared with Agent for cross-layer tracking)
    pub security_events: Arc<crate::core::SecurityEvents>,
    /// App registry for deployed apps (static + dynamic)
    pub app_registry: crate::actions::app::AppRegistry,
    /// Deployment posture of the control plane.
    pub deployment_mode: DeploymentMode,
    /// Which surface this listener serves.
    pub server_role: HttpServerRole,
    /// Optional dedicated bind address for public apps.
    pub public_app_bind_addr: Option<String>,
    /// Optional externally visible base URL for public apps.
    pub public_app_base_url: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingOAuthState {
    service_id: String,
    expires_at: i64,
}

/// Chat request
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    #[serde(default)]
    pub deep_research: bool,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub attachments_present: bool,
}

fn default_channel() -> String {
    "http".to_string()
}

fn is_openrouter_base_url(url: &str) -> bool {
    url.to_ascii_lowercase().contains("openrouter")
}

fn is_codex_cli_base_url(url: &str) -> bool {
    url.trim().eq_ignore_ascii_case("codex://cli")
}

fn effective_openai_base_url(base_url: Option<&str>) -> &str {
    match base_url {
        Some(url) if is_codex_cli_base_url(url) => "https://api.openai.com/v1",
        Some(url) => url,
        None => "https://api.openai.com/v1",
    }
}

fn codex_auth_file_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })?;
    Some(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn read_codex_cli_api_key() -> Option<String> {
    let path = codex_auth_file_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;

    // Native format: {"openai": {"type": "oauth", "access": "eyJ..."}}
    if let Some(openai) = parsed.get("openai") {
        if let Some(access) = openai.get("access").and_then(|v| v.as_str()) {
            if !access.is_empty() {
                // Check expiry if present
                if let Some(expires) = openai.get("expires").and_then(|v| v.as_u64()) {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    if now_ms >= expires {
                        // Token expired - try refresh
                        return None;
                    }
                }
                return Some(access.to_string());
            }
        }
    }

    // Legacy Codex CLI format: {"OPENAI_API_KEY": "sk-..."}
    let key = parsed
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

/// Request a device code from OpenAI and start background polling for authorization.
async fn spawn_codex_oauth_probe() -> std::result::Result<(), String> {
    let runtime = codex_oauth_runtime();
    {
        let state = runtime.read().await;
        if state.active {
            return Ok(());
        }
    }

    // Step 1: Request user code from OpenAI
    let client = reqwest::Client::new();
    let resp = client
        .post(OPENAI_DEVICE_USERCODE_URL)
        .json(&serde_json::json!({ "client_id": OPENAI_DEVICE_AUTH_CLIENT_ID }))
        .send()
        .await
        .map_err(|e| format!("Failed to request device code: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "OpenAI device code request failed ({}): {}",
            status, body
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse device code response: {}", e))?;

    let device_auth_id = body
        .get("device_auth_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let user_code = body
        .get("user_code")
        .or_else(|| body.get("usercode"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let interval: u64 = body
        .get("interval")
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_u64())
        })
        .unwrap_or(5);

    if device_auth_id.is_empty() || user_code.is_empty() {
        return Err(format!(
            "OpenAI returned incomplete device code response: {}",
            body
        ));
    }

    {
        let mut state = runtime.write().await;
        state.active = true;
        state.auth_url = Some(OPENAI_DEVICE_VERIFY_URL.to_string());
        state.device_code = Some(user_code.clone());
        state.device_auth_id = Some(device_auth_id.clone());
        state.user_code = Some(user_code.clone());
        state.poll_interval_secs = interval;
        state.last_output = format!(
            "Open {} and enter code: {}",
            OPENAI_DEVICE_VERIFY_URL, user_code
        );
        state.last_error = None;
    }

    // Step 2: Background task polls for authorization completion
    let runtime_bg = runtime.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let max_attempts = (15 * 60) / interval.max(1); // 15 min timeout

        for _ in 0..max_attempts {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

            let poll_resp = client
                .post(OPENAI_DEVICE_TOKEN_URL)
                .json(&serde_json::json!({
                    "device_auth_id": device_auth_id,
                    "user_code": user_code,
                }))
                .send()
                .await;

            match poll_resp {
                Ok(resp) if resp.status().is_success() => {
                    // User authorized - extract auth code and exchange for tokens
                    if let Ok(poll_body) = resp.json::<serde_json::Value>().await {
                        let auth_code = poll_body
                            .get("authorization_code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let code_verifier = poll_body
                            .get("code_verifier")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        if !auth_code.is_empty() {
                            // Step 3: Exchange authorization code for tokens
                            let token_resp = client
                                .post(OPENAI_OAUTH_TOKEN_URL)
                                .form(&[
                                    ("grant_type", "authorization_code"),
                                    ("client_id", OPENAI_DEVICE_AUTH_CLIENT_ID),
                                    ("code", auth_code),
                                    ("code_verifier", code_verifier),
                                    ("redirect_uri", OPENAI_DEVICE_REDIRECT_URI),
                                ])
                                .send()
                                .await;

                            match token_resp {
                                Ok(tr) if tr.status().is_success() => {
                                    if let Ok(tokens) = tr.json::<serde_json::Value>().await {
                                        let access = tokens
                                            .get("access_token")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let refresh = tokens
                                            .get("refresh_token")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let expires_in = tokens
                                            .get("expires_in")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(3600);
                                        let expires_ms = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis()
                                            as u64
                                            + (expires_in * 1000);

                                        // Save to ~/.codex/auth.json
                                        if let Some(path) = codex_auth_file_path() {
                                            if let Some(parent) = path.parent() {
                                                let _ = std::fs::create_dir_all(parent);
                                            }
                                            let auth_json = serde_json::json!({
                                                "openai": {
                                                    "type": "oauth",
                                                    "access": access,
                                                    "refresh": refresh,
                                                    "expires": expires_ms
                                                }
                                            });
                                            let _ = std::fs::write(
                                                &path,
                                                serde_json::to_string_pretty(&auth_json)
                                                    .unwrap_or_default(),
                                            );
                                        }

                                        let mut state = runtime_bg.write().await;
                                        state.active = false;
                                        state.last_output =
                                            "OpenAI OAuth connected successfully.".to_string();
                                        state.last_error = None;
                                        return;
                                    }
                                }
                                Ok(tr) => {
                                    let body = tr.text().await.unwrap_or_default();
                                    let mut state = runtime_bg.write().await;
                                    state.active = false;
                                    state.last_error =
                                        Some(format!("Token exchange failed: {}", body));
                                    return;
                                }
                                Err(e) => {
                                    let mut state = runtime_bg.write().await;
                                    state.active = false;
                                    state.last_error = Some(format!("Token exchange error: {}", e));
                                    return;
                                }
                            }
                        }
                    }
                }
                Ok(resp)
                    if resp.status() == reqwest::StatusCode::FORBIDDEN
                        || resp.status() == reqwest::StatusCode::NOT_FOUND =>
                {
                    // Still pending - continue polling
                    continue;
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    let mut state = runtime_bg.write().await;
                    state.active = false;
                    state.last_error = Some(format!("Poll failed ({}): {}", status, body));
                    return;
                }
                Err(e) => {
                    let mut state = runtime_bg.write().await;
                    state.active = false;
                    state.last_error = Some(format!("Poll request error: {}", e));
                    return;
                }
            }
        }

        // Timeout
        let mut state = runtime_bg.write().await;
        state.active = false;
        state.last_error = Some("Device code expired (15 min timeout). Try again.".to_string());
    });

    Ok(())
}

async fn open_url_in_default_browser(url: &str) -> std::result::Result<(), String> {
    let status = if cfg!(target_os = "windows") {
        tokio::process::Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .arg(url)
            .status()
            .await
            .map_err(|e| format!("Failed to launch browser: {}", e))?
    } else if cfg!(target_os = "macos") {
        tokio::process::Command::new("open")
            .arg(url)
            .status()
            .await
            .map_err(|e| format!("Failed to launch browser: {}", e))?
    } else {
        tokio::process::Command::new("xdg-open")
            .arg(url)
            .status()
            .await
            .map_err(|e| format!("Failed to launch browser: {}", e))?
    };

    if status.success() {
        Ok(())
    } else {
        Err(format!("Browser launcher exited with status {}", status))
    }
}

fn provider_label_for_openai(base_url: &Option<String>) -> &'static str {
    match base_url.as_deref() {
        Some(url) if is_codex_cli_base_url(url) => "openai-subscription",
        Some(url) if is_openrouter_base_url(url) => "openrouter",
        Some(_) => "openai-compatible",
        None => "openai",
    }
}

fn normalize_openai_base_url(
    provider: &str,
    base_url: Option<String>,
) -> std::result::Result<Option<String>, String> {
    let normalized = base_url.and_then(|u| {
        let trimmed = u.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    match provider {
        "codex-cli" | "openai-subscription" => Ok(Some("codex://cli".to_string())),
        "openrouter" => {
            Ok(Some(normalized.unwrap_or_else(|| {
                "https://openrouter.ai/api/v1".to_string()
            })))
        }
        "openai-compatible" => {
            if normalized.is_none() {
                Err("Base URL is required for OpenAI-Compatible providers".to_string())
            } else {
                Ok(normalized)
            }
        }
        _ => Ok(normalized),
    }
}

fn provider_from_model_slot_request(
    request: &ModelSlotRequest,
    existing_api_key: Option<String>,
) -> std::result::Result<LlmProvider, String> {
    let base_url = request.base_url.clone().and_then(|u| {
        let trimmed = u.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let compat_base_url = normalize_openai_base_url(request.provider.as_str(), base_url.clone())?;
    let mut api_key = request
        .api_key
        .clone()
        .filter(|k| !k.is_empty() && k != "[ENCRYPTED]")
        .or(existing_api_key.filter(|k| !k.is_empty() && k != "[ENCRYPTED]"))
        .unwrap_or_default();
    if (request.provider == "codex-cli" || request.provider == "openai-subscription")
        && api_key.is_empty()
    {
        api_key = read_codex_cli_api_key().unwrap_or_default();
    }

    let provider = match request.provider.as_str() {
        "ollama" => LlmProvider::Ollama {
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
            model: request.model.clone(),
        },
        "anthropic" => LlmProvider::Anthropic {
            api_key: api_key.clone(),
            model: request.model.clone(),
        },
        "openai" => LlmProvider::OpenAI {
            api_key: api_key.clone(),
            model: request.model.clone(),
            base_url: None,
        },
        "openai-compatible" | "openrouter" => LlmProvider::OpenAI {
            api_key: api_key.clone(),
            model: request.model.clone(),
            base_url: compat_base_url,
        },
        "codex-cli" | "openai-subscription" => {
            if api_key.trim().is_empty() {
                return Err(
                    "OpenAI Subscription is not connected yet. Click 'Connect via Browser' and complete OAuth first.".to_string(),
                );
            }
            LlmProvider::OpenAI {
                api_key: api_key.clone(),
                model: request.model.clone(),
                base_url: compat_base_url,
            }
        }
        _ => {
            return Err(format!("Unknown provider: {}", request.provider));
        }
    };

    Ok(provider)
}

fn is_env_var_style_key(key: &str) -> bool {
    // Conservative: env vars are typically uppercase, digits, and underscores.
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn parse_set_secret_command(message: &str) -> Option<(String, String)> {
    // Syntax:
    // - "set secret KEY=VALUE"
    // - "set secret KEY VALUE"
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("set secret ") {
        return None;
    }
    let rest = trimmed[10..].trim(); // len("set secret ") == 10
    if rest.is_empty() {
        return None;
    }

    let (key, value) = if let Some(eq) = rest.find('=') {
        let (k, v) = rest.split_at(eq);
        (k.trim(), v[1..].trim())
    } else {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let k = parts.next().unwrap_or("").trim();
        let v = parts.next().unwrap_or("").trim();
        (k, v)
    };

    if key.is_empty() || value.is_empty() {
        return None;
    }
    if key.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    // Avoid accidentally capturing multi-line pastes into the key name.
    if key.contains('\n') || key.contains('\r') {
        return None;
    }

    Some((key.to_string(), value.to_string()))
}

fn normalize_origin(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let uri: Uri = trimmed.parse().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let authority = uri.authority()?.as_str().to_ascii_lowercase();
    Some(format!("{}://{}", scheme, authority))
}

fn is_local_origin(value: &str) -> bool {
    let normalized = match normalize_origin(value) {
        Some(v) => v,
        None => return false,
    };
    normalized.starts_with("http://localhost")
        || normalized.starts_with("https://localhost")
        || normalized.starts_with("http://127.0.0.1")
        || normalized.starts_with("https://127.0.0.1")
        || normalized.starts_with("http://[::1]")
        || normalized.starts_with("https://[::1]")
}

fn generate_ephemeral_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn parse_env_truthy(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.eq_ignore_ascii_case("true") || trimmed == "1" {
            Some(true)
        } else if trimmed.eq_ignore_ascii_case("false") || trimmed == "0" {
            Some(false)
        } else {
            None
        }
    })
}

fn normalize_optional_url(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
}

fn deployment_mode_from_config(config: &crate::core::config::AgentConfig) -> DeploymentMode {
    if let Some(force_mode) = std::env::var("AGENTARK_DEPLOYMENT_MODE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
    {
        if force_mode == "internet_facing" || force_mode == "internet-facing" {
            return DeploymentMode::InternetFacing;
        }
        if force_mode == "trusted_local" || force_mode == "trusted-local" {
            return DeploymentMode::TrustedLocal;
        }
    }
    config.deployment_mode
}

fn public_app_bind_addr_from_config(
    config: &crate::core::config::AgentConfig,
    deployment_mode: DeploymentMode,
) -> Option<String> {
    normalize_optional_url(std::env::var("AGENTARK_PUBLIC_APP_BIND").ok().as_deref())
        .or_else(|| {
            config
                .public_apps
                .bind_addr
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            if deployment_mode == DeploymentMode::InternetFacing {
                Some("127.0.0.1:8992".to_string())
            } else {
                None
            }
        })
}

fn public_app_base_url_from_config(config: &crate::core::config::AgentConfig) -> Option<String> {
    normalize_optional_url(
        std::env::var("AGENTARK_PUBLIC_APP_BASE_URL")
            .ok()
            .as_deref(),
    )
    .or_else(|| normalize_optional_url(config.public_apps.base_url.as_deref()))
}

fn default_base_url_for_bind_addr(bind_addr: &str) -> Option<String> {
    let trimmed = bind_addr.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if trimmed.starts_with("0.0.0.0:") {
        format!("localhost:{}", trimmed.trim_start_matches("0.0.0.0:"))
    } else if trimmed == "0.0.0.0" {
        "localhost".to_string()
    } else if trimmed.starts_with("[::]:") {
        format!("localhost:{}", trimmed.trim_start_matches("[::]:"))
    } else if trimmed == "[::]" || trimmed == "::" {
        "localhost".to_string()
    } else if trimmed.starts_with("127.0.0.1:") || trimmed == "127.0.0.1" {
        trimmed.replacen("127.0.0.1", "localhost", 1)
    } else {
        trimmed.to_string()
    };
    Some(format!("http://{}", normalized.trim_end_matches('/')))
}

fn internet_facing_apps_should_be_isolated(
    deployment_mode: DeploymentMode,
    public_app_bind_addr: Option<&str>,
) -> bool {
    deployment_mode == DeploymentMode::InternetFacing
        && public_app_bind_addr
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
fn parse_autonomy_quick_command(message: &str) -> Option<AutonomyQuickCommand> {
    let trimmed = message.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if normalized == "/triage" || normalized == "/triage inbox" || normalized == "/inbox triage" {
        return Some(AutonomyQuickCommand::TriageInbox);
    }
    if normalized.starts_with("/delegate ") {
        let task = trimmed[10..].trim();
        if task.is_empty() {
            return None;
        }
        return Some(AutonomyQuickCommand::Delegate {
            task: task.to_string(),
            require_approval: false,
        });
    }
    if normalized.starts_with("/rollback ") {
        let rest = trimmed[10..].trim();
        if rest.is_empty() {
            return None;
        }
        let mut parts = rest.split_whitespace();
        let event_id = parts.next().unwrap_or("").trim();
        if event_id.is_empty() {
            return None;
        }
        let operation = parts.next().map(|raw| {
            let op = raw.trim().to_ascii_lowercase();
            match op.as_str() {
                "read" => "mark_read".to_string(),
                "unread" => "mark_unread".to_string(),
                _ => op,
            }
        });
        return Some(AutonomyQuickCommand::Rollback {
            event_id: event_id.to_string(),
            operation,
        });
    }
    None
}

fn parse_notification_control_command(message: &str) -> Option<NotificationControlCommand> {
    let normalized = message.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    let text = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.as_str() {
        "pause notifications"
        | "pause notification"
        | "mute notifications"
        | "mute notification"
        | "/notifications pause"
        | "/pause notifications" => Some(NotificationControlCommand::Pause24h),
        "resume notifications"
        | "resume notification"
        | "unmute notifications"
        | "unmute notification"
        | "/notifications resume"
        | "/resume notifications" => Some(NotificationControlCommand::Resume),
        "notification status"
        | "notifications status"
        | "status notifications"
        | "status notification"
        | "/notifications"
        | "/notifications status" => Some(NotificationControlCommand::Status),
        _ => None,
    }
}

async fn handle_notification_control_command(
    state: &AppState,
    cmd: NotificationControlCommand,
) -> std::result::Result<String, String> {
    let agent = state.agent.read().await;
    match cmd {
        NotificationControlCommand::Pause24h => {
            let until_ts = agent
                .pause_push_notifications_for_hours(24)
                .await
                .map_err(|e| format!("Failed to pause notifications: {}", e))?;
            let until = chrono::DateTime::<chrono::Utc>::from_timestamp(until_ts, 0)
                .unwrap_or_else(chrono::Utc::now);
            Ok(format!(
                "Push notifications paused until {}. Type 'resume notifications' anytime to re-enable.",
                until.format("%Y-%m-%d %H:%M:%S UTC")
            ))
        }
        NotificationControlCommand::Resume => {
            agent
                .resume_push_notifications()
                .await
                .map_err(|e| format!("Failed to resume notifications: {}", e))?;
            Ok("Push notifications resumed.".to_string())
        }
        NotificationControlCommand::Status => {
            if let Some(until_ts) = agent.push_notifications_muted_until_ts().await {
                let until = chrono::DateTime::<chrono::Utc>::from_timestamp(until_ts, 0)
                    .unwrap_or_else(chrono::Utc::now);
                Ok(format!(
                    "Push notifications are currently paused until {}.",
                    until.format("%Y-%m-%d %H:%M:%S UTC")
                ))
            } else {
                Ok("Push notifications are active.".to_string())
            }
        }
    }
}

async fn server_load_reasons(state: &AppState) -> Vec<String> {
    let now = chrono::Utc::now();
    let mut reasons = {
        let tasks = state.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::InProgress))
            .filter_map(|task| {
                let age_secs = (now - task.created_at).num_seconds().max(0);
                if age_secs > SERVER_BUSY_TASK_WINDOW_SECS {
                    None
                } else {
                    Some(format!(
                        "task '{}' in progress ({}s)",
                        task.action, age_secs
                    ))
                }
            })
            .take(3)
            .collect::<Vec<_>>()
    };

    let active_trace_reason = {
        let last_trace = state.last_trace.read().await;
        last_trace
            .started_at
            .filter(|_| last_trace.completed_at.is_none())
            .and_then(|started_at| {
                let age_secs = (now - started_at).num_seconds().max(0);
                if age_secs > SERVER_BUSY_TRACE_WINDOW_SECS {
                    None
                } else {
                    Some(format!("active trace ({}s)", age_secs))
                }
            })
    };
    if let Some(reason) = active_trace_reason {
        reasons.push(reason);
    }

    reasons
}

async fn server_under_load(state: &AppState) -> bool {
    !server_load_reasons(state).await.is_empty()
}

async fn handle_autonomy_quick_command(
    state: &AppState,
    cmd: AutonomyQuickCommand,
) -> std::result::Result<String, String> {
    match cmd {
        AutonomyQuickCommand::TriageInbox => {
            let labels = vec![
                "Act now".to_string(),
                "Delegate".to_string(),
                "Ignore".to_string(),
            ];
            let agent = state.agent.read().await;
            let fallback = agent
                .storage
                .list_notifications(30, 0, true)
                .await
                .unwrap_or_default();
            let messages: Vec<serde_json::Value> = fallback
                .into_iter()
                .map(|n| {
                    serde_json::json!({
                        "id": n.id,
                        "from": n.source,
                        "subject": n.title,
                        "snippet": n.body,
                    })
                })
                .collect();
            if messages.is_empty() {
                return Ok(
                    "No inbox items found to triage. Unread notifications are already clear."
                        .to_string(),
                );
            }

            let payload = serde_json::json!({ "messages": messages, "labels": labels });
            let llm_response = agent
                .llm
                .chat(
                    "You are an executive inbox triage assistant. Return strict JSON {\"triage\":[{\"message_id\":\"...\",\"label\":\"...\",\"reason\":\"...\",\"draft_reply\":\"...\"}]}.",
                    &payload.to_string(),
                    &[],
                    &[],
                )
                .await
                .ok();
            if let Some(ref r) = llm_response {
                agent.record_llm_usage("web", "inbox_triage", r).await;
            }

            let parsed = llm_response
                .as_ref()
                .and_then(|r| extract_json(&r.content))
                .unwrap_or_else(|| {
                    let triage: Vec<serde_json::Value> = payload
                        .get("messages")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(|m| {
                            let snippet = m
                                .get("snippet")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let label = if snippet.contains("urgent") || snippet.contains("asap")
                            {
                                "Act now"
                            } else {
                                "Delegate"
                            };
                            serde_json::json!({
                                "message_id": m.get("id").cloned().unwrap_or_else(|| serde_json::json!("")),
                                "label": label,
                                "reason": "Heuristic fallback classification",
                                "draft_reply": if label == "Act now" { "Acknowledged. I will handle this today." } else { "Received. Delegating to the right owner and will track status." },
                            })
                        })
                        .collect();
                    serde_json::json!({ "triage": triage })
                });

            let rows = parsed
                .get("triage")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if rows.is_empty() {
                return Ok("Inbox triage complete. No items were classified.".to_string());
            }
            let mut out = format!("Inbox triage complete: {} item(s).\n", rows.len());
            for row in rows.iter().take(10) {
                let id = row
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        row.get("message_id")
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".to_string())
                    });
                let label = row.get("label").and_then(|v| v.as_str()).unwrap_or("-");
                let reason = row.get("reason").and_then(|v| v.as_str()).unwrap_or("-");
                out.push_str(&format!("- [{}] {}: {}\n", label, id, reason));
            }
            if rows.len() > 10 {
                out.push_str(&format!("... and {} more.\n", rows.len() - 10));
            }
            out.push_str("Tip: use /delegate <task> for anything labeled Delegate.");
            Ok(out)
        }
        AutonomyQuickCommand::Delegate {
            task,
            require_approval,
        } => {
            if task.trim().is_empty() {
                return Err("Task is required. Usage: /delegate <task description>".to_string());
            }
            let agent = state.agent.read().await;
            let settings = load_autonomy_settings(&agent).await;
            let trust = score_action_risk(
                "delegate",
                &serde_json::json!({"task": task}),
                &settings.trust_policy,
            );

            if trust.requires_approval || require_approval {
                let mut approval_task = Task::new(
                    format!("Delegation approval: {}", task),
                    "delegate".to_string(),
                    serde_json::json!({
                        "task": task,
                        "context": "",
                        "_approval": {
                            "title": format!("Delegate: {}", task),
                            "summary": "This delegation will spawn specialist/background work on your behalf.",
                            "reason": trust.reasons.join("; "),
                            "rule_name": "elevated_action_requires_explicit_approval",
                            "risk_level": risk_level_label(&trust.level),
                            "risk_score": trust.score,
                            "source": "autonomy_quick_command"
                        }
                    }),
                );
                approval_task.status = TaskStatus::AwaitingApproval;
                approval_task.approval = TaskApproval::RequireApproval;
                let queued = agent.add_or_update_similar_task(approval_task, false).await;
                if let Err(e) = queued {
                    return Err(format!("Failed to queue delegation approval: {}", e));
                }
                return Ok(format!(
                    "Delegation queued for approval (risk: {} / score {}).",
                    risk_level_label(&trust.level),
                    trust.score
                ));
            }

            let Some(ref swarm) = agent.swarm else {
                return Err("Swarm is not enabled, so delegation is unavailable.".to_string());
            };
            let actions = agent.runtime.list_actions().await.unwrap_or_default();
            match swarm.delegate(&task, "", &agent.llm, &[], &actions).await {
                Ok(result) => {
                    let delegation = crate::storage::entities::swarm_delegation::Model {
                        id: uuid::Uuid::new_v4().to_string(),
                        parent_task_id: None,
                        agent_id: result.agents_used.join(","),
                        task_description: task.clone(),
                        result: Some(result.final_result.clone()),
                        success: 1,
                        confidence: Some(0.82),
                        execution_time_ms: Some(result.total_time_ms as i32),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        completed_at: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    let _ = agent.storage.insert_swarm_delegation(&delegation).await;
                    let final_result = crate::security::redact_pii(&result.final_result);
                    Ok(format!(
                        "Delegation complete.\nAgents used: {}\nTime: {} ms\nResult:\n{}",
                        if result.agents_used.is_empty() {
                            "-".to_string()
                        } else {
                            result.agents_used.join(", ")
                        },
                        result.total_time_ms,
                        final_result
                    ))
                }
                Err(e) => Err(format!("Delegation failed: {}", e)),
            }
        }
        AutonomyQuickCommand::Rollback {
            event_id,
            operation,
        } => {
            let agent = state.agent.read().await;
            let event_id_trimmed = event_id.trim();
            let operation = operation.unwrap_or_default();

            if let Some(task_id) = event_id_trimmed.strip_prefix("task:") {
                let uuid = uuid::Uuid::parse_str(task_id)
                    .map_err(|_| "Invalid task id. Expected format: task:<uuid>".to_string())?;
                let mut tasks = agent.tasks.write().await;
                let Some(task) = tasks.get_mut(uuid) else {
                    return Err("Task not found.".to_string());
                };
                if !matches!(
                    task.status,
                    TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
                ) {
                    return Err("Task cannot be cancelled from its current state.".to_string());
                }
                task.status = TaskStatus::Cancelled;
                let status_json = serde_json::to_string(&task.status)
                    .unwrap_or_else(|_| "\"Cancelled\"".to_string());
                let _ = agent
                    .storage
                    .update_task_status(task_id, &status_json)
                    .await;
                return Ok("Rollback applied: task cancelled.".to_string());
            }

            if let Some(watcher_id) = event_id_trimmed.strip_prefix("watcher:") {
                let uuid = uuid::Uuid::parse_str(watcher_id).map_err(|_| {
                    "Invalid watcher id. Expected format: watcher:<uuid>".to_string()
                })?;
                if agent.watcher_manager.cancel(uuid).await {
                    if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                        agent
                            .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                            .await;
                    }
                    return Ok("Rollback applied: watcher cancelled.".to_string());
                }
                return Err("Watcher not found or not cancellable.".to_string());
            }

            if let Some(notification_id) = event_id_trimmed.strip_prefix("notification:") {
                let read = operation != "mark_unread";
                agent
                    .storage
                    .set_notification_read(notification_id, read)
                    .await
                    .map_err(|e| format!("Failed to update notification: {}", e))?;
                return Ok(if read {
                    "Rollback applied: notification marked as read.".to_string()
                } else {
                    "Rollback applied: notification marked as unread.".to_string()
                });
            }

            Err("Unsupported rollback target. Use task:<uuid>, watcher:<uuid>, or notification:<id>.".to_string())
        }
    }
}

/// Chat response
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub response: String,
    pub proof_id: Option<String>,
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_title: Option<String>,
}

/// Agent status response
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub did: String,
    pub memory_entries: usize,
    pub skills_loaded: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_loaded: Option<usize>,
    pub tasks_pending: usize,
    pub version: String,
}

/// User profile response
#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub name: Option<String>,
    pub location: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    pub preferences: Option<String>,
    pub onboarding_complete: bool,
}

#[derive(Debug, Serialize)]
pub struct TaskInfo {
    pub id: String,
    pub description: String,
    pub action: String,
    pub arguments: serde_json::Value,
    pub status: String,
    pub cron: Option<String>,
    pub result: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct AutomationObjectInfo {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: String,
    pub detail: Option<String>,
    pub created_at: Option<String>,
    pub next_run_at: Option<String>,
    pub view: String,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub connected: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AutomationRunInfo {
    pub id: String,
    pub automation_id: String,
    pub kind: String,
    pub title: String,
    pub action: String,
    pub trigger: String,
    pub status: String,
    pub current_status: Option<String>,
    pub attempt: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub summary: String,
    pub output_preview: Option<String>,
    pub error: Option<String>,
    pub next_retry_at: Option<String>,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub view: String,
}

#[derive(Debug, Default, Serialize)]
pub struct AutomationInventoryTotals {
    pub total: usize,
    pub tasks: usize,
    pub watchers: usize,
    pub apps: usize,
    pub integrations: usize,
}

/// Create task request
#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub description: String,
    pub action: String,
    pub arguments: serde_json::Value,
    /// Cron expression for scheduling (e.g., "*/5 * * * *" for every 5 minutes)
    pub cron: Option<String>,
    /// Approval policy: "auto" or "require"
    pub approval: Option<String>,
    #[serde(default)]
    pub allow_duplicate: bool,
}

/// Update task request
#[derive(Debug, Deserialize)]
pub struct UpdateTaskRequest {
    pub description: Option<String>,
    pub arguments: Option<serde_json::Value>,
    pub cron: Option<String>,
}

/// Plan task request (LLM-assisted)
#[derive(Debug, Deserialize)]
pub struct PlanTaskRequest {
    pub description: String,
    pub prompt: Option<String>,
}

/// Plan task response
#[derive(Debug, Serialize)]
pub struct PlanTaskResponse {
    pub plan: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct CodexCliOAuthStartResponse {
    pub started: bool,
    pub running: bool,
    pub opened_browser: bool,
    pub auth_url: String,
    pub device_code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct CodexCliOAuthStatusResponse {
    pub connected: bool,
    pub has_api_key: bool,
    pub running: bool,
    pub auth_url: String,
    pub device_code: String,
    pub message: String,
}

/// Settings response (for GET)
#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub bot_name: String,
    pub personality: String,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    pub daily_brief_enabled: bool,
    pub daily_brief_time: String,
    pub daily_brief_channel: String,
    // Primary LLM (legacy)
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub has_api_key: bool,
    // Fallback LLM (legacy)
    pub llm_fallback_provider: Option<String>,
    pub llm_fallback_model: Option<String>,
    pub llm_fallback_base_url: Option<String>,
    pub has_fallback_api_key: bool,
    // Model pool
    pub model_pool: Vec<ModelSlotSummary>,
    pub smart_routing: bool,
    /// Optional pinned model slot for app_deploy.
    /// If unset, app_deploy uses the default primary model.
    pub app_deploy_model_id: Option<String>,
    // Telegram
    pub telegram_enabled: bool,
    pub has_telegram_token: bool,
    pub telegram_delivery_ready: bool,
    pub telegram_allowed_users: Vec<i64>,
    // WhatsApp
    pub whatsapp_enabled: bool,
    pub whatsapp_mode: String,
    pub has_whatsapp_token: bool,
    pub whatsapp_delivery_ready: bool,
    pub whatsapp_phone_number_id: String,
    pub whatsapp_bridge_url: String,
    pub whatsapp_dm_policy: String,
    pub whatsapp_allowed_numbers: Vec<String>,
    pub auto_approve: Vec<String>,
    // Search
    pub search_primary: String,
    pub search_fallback1: String,
    pub search_fallback2: String,
    pub search_serper_configured: bool,
    pub search_searxng_url: Option<String>,
    pub search_brave_configured: bool,
    pub settings_complete: bool,
    // Moltbook
    pub moltbook_enabled: bool,
    pub moltbook_mode: String,
    pub moltbook_sync_frequency: String,
    pub moltbook_write_enabled: bool,
    pub moltbook_defer_when_busy: bool,
    pub moltbook_last_run_at: Option<String>,
    pub moltbook_last_status: Option<String>,
    pub tunnel_active: bool,
    pub deployment_mode: String,
    pub public_app_bind_addr: Option<String>,
    pub public_app_base_url: Option<String>,
    // Memory retention (episodic pruning; disabled by default)
    pub memory_retention_enabled: bool,
    pub memory_retention_min_age_days: u64,
    pub memory_retention_keep_last: usize,
    pub memory_retention_max_importance: f32,
    pub memory_retention_max_access_count: i32,
    pub memory_retention_require_consolidated: bool,
    pub memory_retention_run_interval_days: u64,
    pub memory_retention_idle_threshold_secs: u64,
    pub memory_retention_max_delete_per_run: u64,
    pub memory_retention_protect_fact_sources: bool,
    pub observability: observability::ObservabilitySettingsResponse,
}

/// Model slot summary for API responses
#[derive(Debug, Serialize)]
pub struct ModelSlotSummary {
    pub id: String,
    pub label: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub has_api_key: bool,
    pub enabled: bool,
}

/// Request to create/update a model slot
#[derive(Debug, Deserialize)]
pub struct ModelSlotRequest {
    pub label: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

/// Settings update request (for POST)
#[derive(Debug, Deserialize)]
pub struct SettingsUpdate {
    pub bot_name: Option<String>,
    pub personality: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    #[serde(default)]
    pub daily_brief_enabled: Option<bool>,
    #[serde(default)]
    pub daily_brief_time: Option<String>,
    pub daily_brief_channel: Option<String>,
    /// Model pool routing behavior (if false, always use primary)
    #[serde(default)]
    pub smart_routing: Option<bool>,
    /// Optional model slot id to always use for app_deploy.
    /// Empty string clears the override.
    #[serde(default)]
    pub app_deploy_model_id: Option<String>,
    // Primary LLM
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    // Fallback LLM (used if primary fails)
    pub llm_fallback_provider: Option<String>,
    pub llm_fallback_model: Option<String>,
    pub llm_fallback_base_url: Option<String>,
    pub llm_fallback_api_key: Option<String>,
    // Telegram
    pub telegram_enabled: bool,
    pub telegram_bot_token: Option<String>,
    pub telegram_allowed_users: Option<Vec<i64>>,
    // WhatsApp
    #[serde(default)]
    pub whatsapp_enabled: bool,
    #[serde(default)]
    pub whatsapp_mode: Option<String>,
    pub whatsapp_access_token: Option<String>,
    pub whatsapp_phone_number_id: Option<String>,
    pub whatsapp_verify_token: Option<String>,
    pub whatsapp_bridge_url: Option<String>,
    #[serde(default)]
    pub whatsapp_dm_policy: Option<String>,
    #[serde(default)]
    pub whatsapp_allowed_numbers: Option<Vec<String>>,
    /// Actions that run without approval
    #[serde(default)]
    pub auto_approve: Option<Vec<String>>,
    #[serde(default)]
    pub deployment_mode: Option<String>,
    #[serde(default)]
    pub public_app_bind_addr: Option<String>,
    #[serde(default)]
    pub public_app_base_url: Option<String>,
    /// Media generation provider API keys (all stored encrypted)
    #[serde(default)]
    pub media_providers: std::collections::HashMap<String, String>,
    /// Default provider for image generation
    pub default_image_provider: Option<String>,
    /// Image model name
    pub image_model: Option<String>,
    /// Fallback provider for image generation
    pub fallback_image_provider: Option<String>,
    /// Default provider for video generation
    pub default_video_provider: Option<String>,
    /// Fallback provider for video generation
    pub fallback_video_provider: Option<String>,
    /// Search: primary backend
    #[serde(default)]
    pub search_primary: Option<String>,
    /// Search: first fallback backend
    #[serde(default)]
    pub search_fallback1: Option<String>,
    /// Search: second fallback backend
    #[serde(default)]
    pub search_fallback2: Option<String>,
    /// Search: Serper API key
    #[serde(default)]
    pub search_serper_key: Option<String>,
    /// Search: SearXNG URL
    #[serde(default)]
    pub search_searxng_url: Option<String>,
    /// Search: Brave API key
    #[serde(default)]
    pub search_brave_key: Option<String>,
    // Moltbook (optional)
    #[serde(default)]
    pub moltbook_api_key: Option<String>,
    #[serde(default)]
    pub moltbook_enabled: Option<bool>,
    #[serde(default)]
    pub moltbook_mode: Option<String>,
    #[serde(default)]
    pub moltbook_sync_frequency: Option<String>,
    #[serde(default)]
    pub moltbook_write_enabled: Option<bool>,
    #[serde(default)]
    pub moltbook_defer_when_busy: Option<bool>,
    // Memory retention (episodic pruning; optional)
    #[serde(default)]
    pub memory_retention_enabled: Option<bool>,
    #[serde(default)]
    pub memory_retention_min_age_days: Option<u64>,
    #[serde(default)]
    pub memory_retention_keep_last: Option<usize>,
    #[serde(default)]
    pub memory_retention_max_importance: Option<f32>,
    #[serde(default)]
    pub memory_retention_max_access_count: Option<i32>,
    #[serde(default)]
    pub memory_retention_require_consolidated: Option<bool>,
    #[serde(default)]
    pub memory_retention_run_interval_days: Option<u64>,
    #[serde(default)]
    pub memory_retention_idle_threshold_secs: Option<u64>,
    #[serde(default)]
    pub memory_retention_max_delete_per_run: Option<u64>,
    #[serde(default)]
    pub memory_retention_protect_fact_sources: Option<bool>,
    #[serde(default)]
    pub observability: Option<observability::ObservabilitySettingsUpdate>,
}

#[derive(Debug, Serialize)]
struct MediaSettingsResponse {
    configured: Vec<String>,
    default_image_provider: Option<String>,
    image_model: Option<String>,
    fallback_image_provider: Option<String>,
    default_video_provider: Option<String>,
    fallback_video_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoalLoopRequest {
    goal: String,
    #[serde(default)]
    constraints: Option<String>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    report_cron: Option<String>,
    #[serde(default)]
    preview_only: bool,
    #[serde(default)]
    plan_override: Option<serde_json::Value>,
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoalReportNowRequest {
    goal_id: String,
}

#[derive(Debug, Deserialize, Default)]
struct InboxTriageRequest {
    #[serde(default)]
    messages: Vec<serde_json::Value>,
    #[serde(default)]
    labels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct TimelineRollbackRequest {
    event_id: String,
    #[serde(default)]
    operation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KnowledgeQueryRequest {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NudgeFeedbackRequest {
    action: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    snooze_minutes: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct NudgePlannerRequest {
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    max_items: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TrustEvaluateRequest {
    action_kind: String,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct VoiceCommandRequest {
    command: String,
    #[serde(default)]
    action_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodeExecuteRequest {
    language: String,
    code: String,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CodeExecuteResponse {
    output: String,
    exit_code: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct EvolutionCanarySummary {
    enabled: bool,
    rollout_percent: u8,
    baseline_version: String,
    candidate_version: String,
}

#[derive(Debug, Serialize)]
struct EvolutionSettingsResponse {
    self_evolve_enabled: bool,
    canary: EvolutionCanarySummary,
    last_promotion_result: String,
    replay_gate_result: Option<String>,
    promotion_mode: String,
    deploy_guard_default: bool,
}

#[derive(Debug, Deserialize)]
struct EvolutionSettingsUpdateRequest {
    deploy_guard_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct EvolutionDevQuery {
    limit: Option<u64>,
}

#[derive(Debug, Serialize)]
struct EvolutionVersionMetric {
    version: String,
    samples: usize,
    success_rate: f64,
    error_rate: f64,
    p95_latency_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct EvolutionDevResponse {
    canary_state: Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    last_result: Option<serde_json::Value>,
    lineage_recent: Vec<serde_json::Value>,
    policy_metrics: Vec<EvolutionVersionMetric>,
    strategy_metrics: Vec<EvolutionVersionMetric>,
}

#[derive(Debug, Deserialize)]
struct EvolutionDevActionRequest {
    action: String,
}

const AUTONOMY_LAST_BRIEF_KEY: &str = "autonomy_last_brief_v1";
const AUTONOMY_NUDGE_FEEDBACK_KEY: &str = "autonomy_nudge_feedback_v1";
const AUTONOMY_NUDGE_NOTIFIED_KEY: &str = "autonomy_nudge_notified_v1";
const AUTONOMY_LAST_NUDGES_KEY: &str = "autonomy_last_nudges_v1";
const AUTONOMY_NUDGE_PLANNED_KEY: &str = "autonomy_nudge_planned_v1";
const AUTONOMY_NUDGE_LAST_SCAN_KEY: &str = "autonomy_nudge_last_scan_v1";
const AUTONOMY_ATTENTION_STATE_KEY: &str = "autonomy_attention_state_v1";
const AUTONOMY_CHAT_SUGGESTIONS_KEY: &str = "autonomy_chat_suggestions_v1";
const AUTONOMY_CHAT_SUGGESTION_SCAN_STATE_KEY: &str = "autonomy_chat_suggestion_scan_state_v1";
const DAILY_BRIEF_ENABLED_KEY: &str = "daily_brief_enabled";
const DAILY_BRIEF_TIME_KEY: &str = "daily_brief_time";
const DAILY_BRIEF_CHANNEL_KEY: &str = "daily_brief_channel";
const DEFAULT_DAILY_BRIEF_TIME: &str = "09:00";
const PUBLIC_SELECTED_APP_KEY: &str = "public_selected_app_id";
const HOOKS_STORAGE_KEY: &str = "hooks_v1";
const ROUTING_POLICY_LINEAGE_REL_PATH: &str = ".agentark/self_evolve/routing_policy_lineage.jsonl";
const CHAT_SUGGESTION_SCAN_INTERVAL_HOURS: i64 = 12;
const CHAT_SUGGESTION_SCAN_DEFER_MINUTES: i64 = 30;
const CHAT_SUGGESTION_SCAN_FETCH_LIMIT: u64 = 48;
const CHAT_SUGGESTION_SCAN_BATCH_LIMIT: usize = 12;
const CHAT_SUGGESTION_RECENT_MESSAGES_PER_CHAT: usize = 8;
const CHAT_SUGGESTION_OPEN_LIMIT: usize = 24;
const CHAT_SUGGESTION_RETAINED_HISTORY: usize = 80;
const CHAT_SUGGESTION_RETAINED_WATERMARKS: usize = 512;

fn parse_bool_pref(raw: Option<Vec<u8>>) -> bool {
    raw.and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn normalize_daily_brief_time(value: &str) -> Option<String> {
    let parsed = chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M").ok()?;
    Some(format!("{:02}:{:02}", parsed.hour(), parsed.minute()))
}

fn daily_brief_time_from_cron(cron: &str) -> Option<String> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    let (hour_raw, minute_raw) = match parts.as_slice() {
        [_, minute, hour, _, _, _] => (*hour, *minute),
        [minute, hour, _, _, _] => (*hour, *minute),
        _ => return None,
    };
    let hour = hour_raw.parse::<u32>().ok()?;
    let minute = minute_raw.parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{:02}:{:02}", hour, minute))
}

fn daily_brief_cron_from_time(value: &str) -> Option<String> {
    let normalized = normalize_daily_brief_time(value)?;
    let (hour_raw, minute_raw) = normalized.split_once(':')?;
    let hour = hour_raw.parse::<u32>().ok()?;
    let minute = minute_raw.parse::<u32>().ok()?;
    Some(format!("0 {} {} * * *", minute, hour))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutonomyBriefingResponse {
    generated_at: String,
    scope: String,
    top_risks: Vec<serde_json::Value>,
    top_opportunities: Vec<serde_json::Value>,
    recommended_actions: Vec<RecommendedAction>,
    trust_summary: serde_json::Value,
    suggested_automations: Vec<ChatAutomationSuggestion>,
    suggestion_scan: ChatSuggestionScanState,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomyExecuteActionRequest {
    action: RecommendedAction,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ChatSuggestionConversationWatermark {
    conversation_id: String,
    last_scanned_updated_at: String,
    #[serde(default)]
    last_user_message_id: Option<String>,
    #[serde(default)]
    last_user_message_at: Option<String>,
    scanned_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ChatSuggestionScanState {
    #[serde(default)]
    last_started_at: Option<String>,
    #[serde(default)]
    last_completed_at: Option<String>,
    #[serde(default)]
    next_due_at: Option<String>,
    #[serde(default)]
    last_status: Option<String>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    defer_count: u32,
    #[serde(default)]
    cursor_updated_at: Option<String>,
    #[serde(default)]
    cursor_conversation_id: Option<String>,
    #[serde(default)]
    last_examined_chats: usize,
    #[serde(default)]
    last_created_suggestions: usize,
    #[serde(default)]
    last_low_signal_skips: usize,
    #[serde(default)]
    last_artifact_skips: usize,
    #[serde(default)]
    last_backlog_hint: usize,
    #[serde(default)]
    tracked_chats: usize,
    #[serde(default)]
    conversation_watermarks: Vec<ChatSuggestionConversationWatermark>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatAutomationSuggestion {
    id: String,
    status: String,
    kind: String,
    title: String,
    detail: String,
    rationale: String,
    confidence: f32,
    created_at: String,
    updated_at: String,
    conversation_id: String,
    conversation_title: String,
    conversation_channel: String,
    source_message_id: String,
    source_snippet: String,
    fingerprint: String,
    goal_title: String,
    #[serde(default)]
    goal_detail: Option<String>,
    #[serde(default)]
    accepted_goal_id: Option<String>,
    #[serde(default)]
    dismissed_at: Option<String>,
    #[serde(default)]
    accepted_at: Option<String>,
    #[serde(default)]
    accepted_trace_id: Option<String>,
    #[serde(default)]
    run_status: Option<String>,
    #[serde(default)]
    last_run_error: Option<String>,
    #[serde(default)]
    last_run_started_at: Option<String>,
    #[serde(default)]
    last_run_completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    accepted_outcomes: Vec<suggestions::ChatSuggestionOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct NudgeFeedbackPreference {
    #[serde(default)]
    dismissed: bool,
    #[serde(default)]
    suppressed_until: Option<String>,
    #[serde(default)]
    last_feedback: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryContextSummary {
    id: String,
    summary: String,
    memory_type: String,
    timestamp: String,
    channel: Option<String>,
    importance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PredictiveNudge {
    id: String,
    #[serde(rename = "type")]
    nudge_type: String,
    title: String,
    detail: String,
    confidence: f32,
    priority: u8,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    recommended_action: Option<RecommendedAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    memory_clues: Vec<MemoryContextSummary>,
}

fn summarize_text(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() <= 160 {
        trimmed.to_string()
    } else {
        format!("{} -¬ -...", trimmed.chars().take(160).collect::<String>())
    }
}

fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn parse_utc_rfc3339(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    parse_rfc3339_utc(value)
}

async fn collect_memory_clues(agent: &Agent, query: &str) -> Vec<MemoryContextSummary> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let entries = match agent.memory.retrieve_relevant(trimmed, 3, None).await {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!("Failed to collect predictive nudge memory clues: {}", error);
            return Vec::new();
        }
    };

    entries
        .into_iter()
        .map(|entry| {
            let (memory_type, channel) = match &entry.memory_type {
                MemoryType::Episodic { context } => {
                    ("episodic".to_string(), Some(context.channel.clone()))
                }
                MemoryType::Semantic { .. } => ("semantic".to_string(), None),
                MemoryType::Procedural { .. } => ("procedural".to_string(), None),
            };
            MemoryContextSummary {
                id: entry.id.to_string(),
                summary: summarize_text(&entry.content),
                memory_type,
                timestamp: entry.timestamp.to_rfc3339(),
                channel,
                importance: entry.importance,
            }
        })
        .collect()
}

fn normalize_chat_suggestion_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn chat_suggestion_due_at(now: chrono::DateTime<chrono::Utc>) -> String {
    (now + chrono::Duration::hours(CHAT_SUGGESTION_SCAN_INTERVAL_HOURS)).to_rfc3339()
}

fn chat_suggestion_deferred_due_at(now: chrono::DateTime<chrono::Utc>, defer_count: u32) -> String {
    let steps = defer_count.saturating_sub(1) as i64;
    let delay_minutes = (CHAT_SUGGESTION_SCAN_DEFER_MINUTES + steps * 15).min(180);
    (now + chrono::Duration::minutes(delay_minutes)).to_rfc3339()
}

fn suggestion_kind_title(kind: &str) -> &'static str {
    match kind {
        "watcher" => "Watcher",
        "workflow" => "Workflow",
        "task" => "Task",
        "app" => "App",
        _ => "Automation",
    }
}

fn suggestion_goal_title(kind: &str, title: &str) -> String {
    match kind {
        "watcher" => format!("Draft watcher: {}", title),
        "app" => format!("Draft app: {}", title),
        "workflow" => format!("Draft workflow: {}", title),
        "task" => format!("Draft task: {}", title),
        _ => format!("Draft automation: {}", title),
    }
}

fn chat_suggestion_display_status(raw: &str) -> &'static str {
    match raw {
        "completed" => "Ready",
        "deferred_busy" => "Deferred",
        "no_user_chat" => "Waiting for chat",
        "no_candidates" => "Idle",
        "running" => "Scanning",
        "error" => "Needs attention",
        _ => "Scheduled",
    }
}

async fn load_chat_suggestions(storage: &crate::storage::Storage) -> Vec<ChatAutomationSuggestion> {
    match storage.get(AUTONOMY_CHAT_SUGGESTIONS_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<Vec<ChatAutomationSuggestion>>(&raw).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn save_chat_suggestions(
    storage: &crate::storage::Storage,
    suggestions: &[ChatAutomationSuggestion],
) {
    if let Ok(bytes) = serde_json::to_vec(suggestions) {
        let _ = storage.set(AUTONOMY_CHAT_SUGGESTIONS_KEY, &bytes).await;
    }
}

async fn load_chat_suggestion_scan_state(
    storage: &crate::storage::Storage,
) -> ChatSuggestionScanState {
    match storage.get(AUTONOMY_CHAT_SUGGESTION_SCAN_STATE_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<ChatSuggestionScanState>(&raw).unwrap_or_default()
        }
        _ => ChatSuggestionScanState::default(),
    }
}

async fn save_chat_suggestion_scan_state(
    storage: &crate::storage::Storage,
    state: &ChatSuggestionScanState,
) {
    if let Ok(bytes) = serde_json::to_vec(state) {
        let _ = storage
            .set(AUTONOMY_CHAT_SUGGESTION_SCAN_STATE_KEY, &bytes)
            .await;
    }
}

fn upsert_chat_suggestion_watermark(
    state: &mut ChatSuggestionScanState,
    conversation_id: &str,
    conversation_updated_at: &str,
    user_message_id: Option<&str>,
    user_message_at: Option<&str>,
    scanned_at: &str,
) {
    if let Some(existing) = state
        .conversation_watermarks
        .iter_mut()
        .find(|entry| entry.conversation_id == conversation_id)
    {
        existing.last_scanned_updated_at = conversation_updated_at.to_string();
        existing.last_user_message_id = user_message_id.map(ToString::to_string);
        existing.last_user_message_at = user_message_at.map(ToString::to_string);
        existing.scanned_at = scanned_at.to_string();
    } else {
        state
            .conversation_watermarks
            .push(ChatSuggestionConversationWatermark {
                conversation_id: conversation_id.to_string(),
                last_scanned_updated_at: conversation_updated_at.to_string(),
                last_user_message_id: user_message_id.map(ToString::to_string),
                last_user_message_at: user_message_at.map(ToString::to_string),
                scanned_at: scanned_at.to_string(),
            });
    }

    state.conversation_watermarks.sort_by(|a, b| {
        parse_rfc3339_utc(&b.scanned_at)
            .cmp(&parse_rfc3339_utc(&a.scanned_at))
            .then_with(|| a.conversation_id.cmp(&b.conversation_id))
    });
    if state.conversation_watermarks.len() > CHAT_SUGGESTION_RETAINED_WATERMARKS {
        state
            .conversation_watermarks
            .truncate(CHAT_SUGGESTION_RETAINED_WATERMARKS);
    }
    state.tracked_chats = state.conversation_watermarks.len();
}

fn prune_chat_suggestion_history(
    mut suggestions: Vec<ChatAutomationSuggestion>,
) -> Vec<ChatAutomationSuggestion> {
    suggestions.sort_by(|a, b| {
        parse_rfc3339_utc(&b.updated_at)
            .cmp(&parse_rfc3339_utc(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut open = 0usize;
    let mut retained = Vec::new();
    for suggestion in suggestions {
        if suggestion.status == "open" {
            if open >= CHAT_SUGGESTION_OPEN_LIMIT {
                continue;
            }
            open += 1;
        }
        retained.push(suggestion);
        if retained.len() >= CHAT_SUGGESTION_RETAINED_HISTORY {
            break;
        }
    }
    retained
}

fn chat_suggestion_scan_is_due(
    state: &ChatSuggestionScanState,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    match state.next_due_at.as_deref().and_then(parse_rfc3339_utc) {
        Some(next_due) => now >= next_due,
        None => true,
    }
}

async fn server_busy_for_chat_suggestions(state: &AppState) -> bool {
    if server_under_load(state).await {
        return true;
    }
    let active_traces = {
        let history = state.trace_history.read().await;
        history
            .iter()
            .filter(|trace| trace.completed_at.is_none())
            .count()
    };
    active_traces > 0 || moltbook::is_moltbook_running()
}

async fn conversation_has_recent_app_artifact(
    storage: &crate::storage::Storage,
    conversation_id: &str,
) -> bool {
    let key = Agent::conversation_recent_artifact_key(conversation_id);
    let artifact = storage
        .get(&key)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok());
    artifact
        .as_ref()
        .and_then(|artifact| {
            artifact
                .get("artifact_type")
                .and_then(|value| value.as_str())
        })
        .is_some_and(|artifact_type| artifact_type.eq_ignore_ascii_case("app"))
        || storage
            .get(&Agent::conversation_last_deployed_app_key(conversation_id))
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
            .is_some()
}

fn looks_like_low_signal_message(input: &str) -> bool {
    let normalized = normalize_chat_suggestion_text(input)
        .to_ascii_lowercase()
        .replace(
            |ch: char| !ch.is_ascii_alphanumeric() && !ch.is_ascii_whitespace(),
            " ",
        );
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return true;
    }
    let generic = [
        "hi",
        "hello",
        "hey",
        "thanks",
        "thank you",
        "ok",
        "okay",
        "cool",
        "great",
        "awesome",
        "yep",
        "yes",
        "no",
        "continue",
        "do it",
        "done",
        "stop",
        "wait",
        "hold on",
        "sounds good",
    ];
    if generic.contains(&compact.as_str()) {
        return true;
    }
    let words: Vec<&str> = compact.split_whitespace().collect();
    if words.len() <= 3 {
        let strong = [
            "watch",
            "monitor",
            "alert",
            "notify",
            "dashboard",
            "app",
            "portal",
            "automate",
            "remind",
        ];
        if !words.iter().any(|word| strong.contains(word)) {
            return true;
        }
    }
    false
}

fn conversation_has_signal(messages: &[crate::storage::entities::message::Model]) -> bool {
    messages.iter().any(|message| {
        message.role.eq_ignore_ascii_case("user")
            && !looks_like_low_signal_message(&message.content)
    })
}

fn extract_latest_signal_user_message(
    messages: &[crate::storage::entities::message::Model],
) -> Option<crate::storage::entities::message::Model> {
    messages
        .iter()
        .rev()
        .find(|message| {
            message.role.eq_ignore_ascii_case("user")
                && !looks_like_low_signal_message(&message.content)
        })
        .cloned()
}

fn normalize_suggestion_fingerprint(kind: &str, title: &str, conversation_id: &str) -> String {
    let mut out = String::new();
    for ch in format!("{}:{}:{}", kind, conversation_id, title)
        .to_ascii_lowercase()
        .chars()
    {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn chat_opportunity_text(input: &str) -> String {
    let normalized = normalize_chat_suggestion_text(input);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let prefixes = [
        "i wish ",
        "it would be nice if ",
        "would be nice if ",
        "it would help if ",
        "i need a way to ",
        "i want a way to ",
        "i always have to ",
        "i keep forgetting to ",
        "can you remind me to ",
        "please remind me to ",
        "tell me when ",
        "let me know when ",
        "notify me when ",
        "alert me when ",
        "watch for ",
        "watch ",
        "monitor ",
    ];
    let lower = trimmed.to_ascii_lowercase();
    for prefix in prefixes {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if !rest.trim().is_empty() {
                return trimmed
                    .chars()
                    .skip(prefix.chars().count())
                    .collect::<String>()
                    .trim()
                    .trim_end_matches('.')
                    .to_string();
            }
        }
    }
    trimmed.trim_end_matches('.').to_string()
}

fn infer_chat_automation_suggestion(
    conversation: &crate::storage::entities::conversation::Model,
    source_message: &crate::storage::entities::message::Model,
) -> Option<ChatAutomationSuggestion> {
    let raw = normalize_chat_suggestion_text(&source_message.content);
    let lower = raw.to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }

    let watcher_phrases = [
        "tell me when",
        "let me know when",
        "notify me when",
        "alert me when",
        "keep an eye on",
        "watch for",
        "monitor",
        "track ",
    ];
    let app_phrases = [
        "dashboard",
        "portal",
        "admin panel",
        "app for",
        "interface for",
        "ui for",
        "tool for",
        "console for",
    ];
    let workflow_phrases = [
        "every day",
        "every week",
        "every month",
        "daily",
        "weekly",
        "monthly",
        "recurring",
        "routine",
        "manual",
        "manually",
        "automate",
        "automation",
        "run every",
        "keep forgetting",
        "remind me",
    ];
    let desire_phrases = [
        "i wish",
        "would be nice if",
        "it would help if",
        "i need a way to",
        "i want a way to",
        "i always have to",
    ];

    let kind = if watcher_phrases.iter().any(|phrase| lower.contains(phrase)) {
        "watcher"
    } else if app_phrases.iter().any(|phrase| lower.contains(phrase)) {
        "app"
    } else if workflow_phrases.iter().any(|phrase| lower.contains(phrase)) {
        "workflow"
    } else if desire_phrases.iter().any(|phrase| lower.contains(phrase)) {
        "task"
    } else {
        return None;
    };

    if lower.contains("build this")
        || lower.contains("make this now")
        || lower.contains("implement this")
        || lower.contains("create this now")
    {
        return None;
    }

    let focus = chat_opportunity_text(&raw);
    if focus.is_empty() {
        return None;
    }

    let title = match kind {
        "watcher" => format!("Monitor {}", summarize_text(&focus).trim_end_matches('.')),
        "app" => format!("App for {}", summarize_text(&focus).trim_end_matches('.')),
        "workflow" => format!("Automate {}", summarize_text(&focus).trim_end_matches('.')),
        "task" => format!("Capture {}", summarize_text(&focus).trim_end_matches('.')),
        _ => summarize_text(&focus),
    };
    let detail = match kind {
        "watcher" => format!(
            "Draft a watcher so AgentArk can keep tabs on {} without you asking again.",
            summarize_text(&focus)
        ),
        "app" => format!(
            "Draft a dedicated app or dashboard around {} instead of leaving it buried in chat.",
            summarize_text(&focus)
        ),
        "workflow" => format!(
            "Draft a repeatable workflow for {} so it can become a routine later.",
            summarize_text(&focus)
        ),
        _ => format!(
            "Capture {} as a structured draft so it can be turned into automation later.",
            summarize_text(&focus)
        ),
    };
    let rationale = match kind {
        "watcher" => "This chat sounded like you wanted ongoing monitoring, not just a one-off answer.",
        "app" => "This chat described a durable interface or dashboard rather than a single response.",
        "workflow" => "This chat described recurring or repetitive work that should probably become automation.",
        _ => "This chat sounded like a useful follow-up the system should not lose.",
    };
    let confidence = match kind {
        "watcher" => 0.88,
        "app" => 0.83,
        "workflow" => 0.80,
        _ => 0.72,
    };
    let now = chrono::Utc::now().to_rfc3339();

    Some(ChatAutomationSuggestion {
        id: uuid::Uuid::new_v4().to_string(),
        status: "open".to_string(),
        kind: kind.to_string(),
        title: title.clone(),
        detail,
        rationale: rationale.to_string(),
        confidence,
        created_at: now.clone(),
        updated_at: now,
        conversation_id: conversation.id.clone(),
        conversation_title: summarize_text(&conversation.title),
        conversation_channel: conversation.channel.clone(),
        source_message_id: source_message.id.clone(),
        source_snippet: summarize_text(&source_message.content),
        fingerprint: normalize_suggestion_fingerprint(kind, &title, &conversation.id),
        goal_title: suggestion_goal_title(kind, &title),
        goal_detail: Some(focus),
        accepted_goal_id: None,
        dismissed_at: None,
        accepted_at: None,
        accepted_trace_id: None,
        run_status: None,
        last_run_error: None,
        last_run_started_at: None,
        last_run_completed_at: None,
        accepted_outcomes: Vec::new(),
    })
}

fn build_chat_suggestion_execution_prompt(suggestion: &ChatAutomationSuggestion) -> String {
    let kind_label = suggestion_kind_title(&suggestion.kind);
    let focus = suggestion
        .goal_detail
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&suggestion.title);
    let execution_directive = match suggestion.kind.as_str() {
        "app" => "Build and deploy a concrete starter app now if feasible. Prefer a working thin slice over a plan-only response.",
        "watcher" => "Create a concrete watcher now. Do not just describe the watcher.",
        "workflow" => "Create a concrete automation now, preferably as a watcher, scheduled task, or goal loop.",
        "task" => "Create a concrete task or goal now rather than leaving this as an idea.",
        _ => "Execute the best concrete automation now rather than only describing it.",
    };

    format!(
        "A Mission Control suggestion was inferred from a prior user chat, and the user has now explicitly clicked Accept.\n\
You should execute this accepted suggestion now.\n\
Do not merely save it as a draft goal unless you are blocked by missing information.\n\
If the suggestion is best fulfilled by building/deploying an app, do that so the trace includes real build/runtime details.\n\
If the suggestion is better fulfilled as a watcher, scheduled task, or goal workflow, create that concrete automation instead.\n\
If required inputs are missing, do the safest concrete version you can and clearly say what remains missing.\n\n\
Accepted suggestion type: {kind_label}\n\
Suggestion title: {title}\n\
Suggestion detail: {detail}\n\
Rationale: {rationale}\n\
Original user snippet: {snippet}\n\
Conversation title: {conversation_title}\n\
Requested focus: {focus}\n\n\
Execution directive: {execution_directive}\n\
Return a concise final outcome after the actual work is done.",
        kind_label = kind_label,
        title = suggestion.title,
        detail = suggestion.detail,
        rationale = suggestion.rationale,
        snippet = suggestion.source_snippet,
        conversation_title = suggestion.conversation_title,
        focus = focus,
        execution_directive = execution_directive,
    )
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

/// Persist a security event in background without blocking the current request.
fn spawn_security_log(
    agent: SharedAgent,
    event_type: &str,
    severity: &str,
    message: String,
    source: Option<String>,
) {
    let event_type = event_type.to_string();
    let severity = severity.to_string();
    tokio::spawn(async move {
        let log = crate::storage::security_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            event_type,
            severity,
            message,
            source,
            count: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let agent_guard = agent.read().await;
        if let Err(e) = agent_guard.storage.insert_security_log(&log).await {
            tracing::debug!("Failed to persist security log entry: {}", e);
        }
    });
}

/// Rate limit middleware applies tiered limits per route prefix.
async fn rate_limit_middleware(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let session_token = auth::current_ui_session_token(&state).await;
    if auth::has_valid_ui_session_cookie(request.headers(), session_token.as_deref()) {
        return next.run(request).await;
    }

    let ip = addr.ip().to_string();
    let path = request.uri().path().to_string();
    let method = request.method().to_string();
    let limiter = state.tiered_rate_limiter.select_for_path(&path);

    if !limiter.check_rate_limit(&ip).await {
        state
            .security_events
            .rate_limit_hits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        spawn_security_log(
            state.agent.clone(),
            "rate_limit",
            "low",
            format!("Rate limit exceeded for {} {}", method, path),
            Some(format!("ip={}", ip)),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: "Rate limit exceeded".to_string(),
            }),
        )
            .into_response();
    }

    next.run(request).await
}

/// Start the HTTP server with authentication, CORS, and rate limiting
pub async fn serve(
    agent: SharedAgent,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let tiered_rate_limiter = TieredRateLimiter::new();
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());

    // Spawn a background task to periodically clean up expired rate-limit entries
    {
        let trl = tiered_rate_limiter.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = interval.tick() => trl.cleanup_all().await,
                }
            }
        });
    }

    // Greeting generation is disabled (static defaults only).

    // Clone Arc handles for independent access (avoids blocking during long operations)
    let state = {
        let agent_guard = agent.read().await;
        let deployment_mode = deployment_mode_from_config(&agent_guard.config);
        let public_app_bind_addr =
            public_app_bind_addr_from_config(&agent_guard.config, deployment_mode);
        let public_app_base_url =
            public_app_base_url_from_config(&agent_guard.config).or_else(|| {
                public_app_bind_addr
                    .as_deref()
                    .and_then(default_base_url_for_bind_addr)
            });
        let allow_insecure_no_auth = parse_env_truthy("AGENTARK_INSECURE_NO_AUTH").unwrap_or(false)
            && deployment_mode == DeploymentMode::TrustedLocal;
        if allow_insecure_no_auth {
            tracing::warn!(
                "AGENTARK_INSECURE_NO_AUTH is enabled: protected routes can run without API auth"
            );
        } else if parse_env_truthy("AGENTARK_INSECURE_NO_AUTH").unwrap_or(false)
            && deployment_mode == DeploymentMode::InternetFacing
        {
            tracing::warn!(
                "Ignoring AGENTARK_INSECURE_NO_AUTH because deployment_mode=internet_facing"
            );
        }
        let api_key_info = crate::core::config::SecureConfigManager::new_with_data_dir(
            &agent_guard.config_dir,
            Some(&agent_guard.data_dir),
        )
        .ok()
        .and_then(|sc| sc.get_api_key_info().ok().flatten());
        let initial_api_key = api_key_info
            .as_ref()
            .map(|k| k.key.clone())
            .or_else(|| agent_guard.api_key.clone());
        let initial_api_key_expires_at = api_key_info.as_ref().map(|k| k.expires_at);

        // Generate a random session token (in-memory) so the web UI can auth via cookie
        // without exposing the API key. Rotates on each server start.
        let session_token = initial_api_key.as_ref().map(|_| generate_ephemeral_token());

        // If TLS is configured (direct HTTPS), mark session cookies Secure by default.
        let cookie_secure_default = {
            #[cfg(feature = "tls")]
            {
                agent_guard.config.tls_cert_path.is_some()
                    && agent_guard.config.tls_key_path.is_some()
            }
            #[cfg(not(feature = "tls"))]
            {
                false
            }
        };
        let local_ui_bootstrap_enabled = deployment_mode == DeploymentMode::TrustedLocal;
        AppState {
            agent: agent.clone(),
            trace_history: agent_guard.trace_history.clone(),
            last_trace: agent_guard.last_trace.clone(),
            tasks: agent_guard.tasks.clone(),
            user_profile: agent_guard.user_profile.clone(),
            tiered_rate_limiter,
            api_key: Arc::new(RwLock::new(initial_api_key)),
            api_key_expires_at: Arc::new(RwLock::new(initial_api_key_expires_at)),
            allow_insecure_no_auth,
            session_token: Arc::new(RwLock::new(session_token)),
            local_ui_bootstrap_enabled,
            local_ui_bootstrap_tokens: Arc::new(RwLock::new(HashMap::new())),
            cookie_secure_default,
            oauth_states: Arc::new(RwLock::new(HashMap::new())),
            remote_login_attempts: Arc::new(RwLock::new(HashMap::new())),
            tunnel: Arc::new(RwLock::new(tunnel::TunnelState::new())),
            whatsapp_bridge: Arc::new(RwLock::new(WhatsAppBridgeState::new())),
            security_events: agent_guard.security_events.clone(),
            app_registry: agent_guard.app_registry.clone(),
            deployment_mode,
            server_role: HttpServerRole::ControlPlane,
            public_app_bind_addr,
            public_app_base_url,
        }
    };

    // Public routes (no auth required)
    let public_routes = Router::new()
        .route("/", get(web_ui))
        .route("/ui", get(web_ui))
        .route("/ui/v2", get(web_ui_v2))
        .route("/session/bootstrap", post(auth::bootstrap_ui_session))
        .route(
            "/session/bootstrap/local",
            get(auth::issue_local_ui_bootstrap_token).post(auth::bootstrap_local_ui_session),
        )
        .route(
            "/tunnel/login",
            get(tunnel_auth::tunnel_login_page).post(tunnel_auth::tunnel_login),
        )
        .route("/docs", get(api_docs_page))
        .route("/openapi.json", get(openapi_spec))
        .route("/ui/{*path}", get(web_ui))
        .route("/assets/{*path}", get(serve_frontend_asset))
        .route("/logo.svg", get(serve_logo_svg))
        .route("/logo.png", get(serve_logo_png))
        .route("/logo.jpg", get(serve_logo_jpg))
        .route("/public/proxy/raw", get(public_proxy_raw))
        .route("/health", get(health))
        // WhatsApp webhook (public - Meta calls without auth)
        .route("/webhook/whatsapp", get(whatsapp_webhook_verify))
        .route("/webhook/whatsapp", post(whatsapp_webhook_handler))
        // OAuth callback (public - browser redirect from Google/Meta with no auth headers)
        .route("/oauth/callback", get(integrations::oauth_callback))
        // Deployed apps (public - these are user-facing apps, no auth required)
        .route("/apps/{app_id}", any(serve_app_root))
        .route("/apps/{app_id}/", any(serve_app_root))
        .route("/apps/{app_id}/{*path}", any(serve_app_path));

    // Protected routes (require Bearer token + rate limited)
    let protected_routes = Router::new()
        .route("/status", get(status))
        .route("/chat", post(chat))
        .route("/chat/stream", post(chat_stream))
        .route("/chat/clear", post(clear_chat))
        .route("/skills", get(actions::list_actions))
        .route("/skills", post(actions::create_action))
        .route("/skills/{name}", get(actions::get_action_content))
        .route("/skills/{name}", post(actions::update_action_content))
        .route(
            "/skills/{name}/enabled",
            post(actions::set_action_enabled_endpoint),
        )
        .route(
            "/skills/{name}/secrets",
            get(actions::get_action_secrets).post(actions::set_action_secrets),
        )
        .route("/skills/{name}/test", post(actions::test_action))
        .route("/skills/import", post(actions::import_action))
        .route(
            "/skills/{name}",
            axum::routing::delete(actions::delete_action),
        )
        .route("/tasks", get(list_tasks))
        .route("/tasks", post(create_task))
        .route("/tasks/plan", post(plan_task))
        .route("/tasks/{id}", post(update_task))
        .route("/tasks/{id}", axum::routing::delete(delete_task))
        .route("/tasks/{id}/pause", post(pause_task))
        .route("/tasks/{id}/resume", post(resume_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/retry", post(retry_task))
        .route("/tasks/{id}/approve", post(approve_task))
        .route("/tasks/{id}/reject", post(reject_task))
        .route("/automation/objects", get(list_automation_objects))
        .route("/automation/runs", get(list_automation_runs_endpoint))
        .route("/goals", get(list_goals))
        .route("/goals", post(create_goal))
        .route("/goals/{id}", axum::routing::delete(delete_goal_endpoint))
        // Autonomy control plane
        .route(
            "/autonomy/settings",
            get(get_autonomy_settings).post(update_autonomy_settings),
        )
        .route("/autonomy/briefing", get(get_autonomy_briefing))
        .route(
            "/autonomy/suggestions/{id}",
            get(suggestions::get_autonomy_suggestion_detail),
        )
        .route(
            "/autonomy/suggestions/{id}/accept",
            post(accept_autonomy_suggestion),
        )
        .route(
            "/autonomy/suggestions/{id}/dismiss",
            post(dismiss_autonomy_suggestion),
        )
        .route("/autonomy/skills/execute", post(execute_autonomy_action))
        .route(
            "/autonomy/modes",
            get(list_autonomy_modes).post(save_autonomy_modes),
        )
        .route(
            "/autonomy/modes/{id}/activate",
            post(activate_autonomy_mode),
        )
        .route(
            "/autonomy/context",
            get(get_context_policy).post(set_context_policy),
        )
        .route("/autonomy/goals/loop", post(start_goal_loop))
        .route("/autonomy/goals/progress", get(goal_progress_endpoint))
        .route("/autonomy/goals/report_now", post(run_goal_report_now))
        .route("/autonomy/incidents/live", get(get_live_incidents))
        .route(
            "/autonomy/incidents/{id}/execute",
            post(execute_incident_playbook),
        )
        .route("/autonomy/inbox/triage", post(triage_inbox))
        .route("/autonomy/timeline", get(get_outcome_timeline))
        .route("/autonomy/timeline/rollback", post(rollback_timeline_event))
        .route("/autonomy/knowledge/query", post(query_knowledge_brain))
        .route(
            "/autonomy/knowledge/suggest-imports",
            get(suggest_knowledge_imports),
        )
        .route(
            "/autonomy/nudges",
            get(get_predictive_nudges).post(emit_predictive_nudges),
        )
        .route("/autonomy/nudges/plan", post(plan_predictive_nudges))
        .route(
            "/autonomy/nudges/{id}/feedback",
            post(set_predictive_nudge_feedback),
        )
        .route("/autonomy/trust/evaluate", post(evaluate_trust_request))
        .route("/autonomy/voice/briefing", get(get_voice_briefing))
        .route("/autonomy/voice/command", post(handle_voice_command))
        .route("/gmail/oauth/start", post(integrations::gmail_oauth_start))
        .route("/gmail/status", get(integrations::gmail_status))
        .route("/gmail/test", get(integrations::gmail_test))
        .route("/settings", get(get_settings))
        .route("/settings", post(update_settings))
        .route(
            "/settings/evolution",
            get(get_evolution_settings).post(update_evolution_settings),
        )
        .route("/settings/evolution/dev", get(get_evolution_dev))
        .route(
            "/settings/evolution/dev/action",
            post(run_evolution_dev_action),
        )
        .route("/settings/api-key", get(get_api_key_endpoint))
        .route(
            "/settings/api-key/regenerate",
            post(regenerate_api_key_endpoint),
        )
        .route("/settings/media", get(get_media_settings))
        .route(
            "/settings/observability/logs",
            get(observability::get_observability_logs),
        )
        .route(
            "/settings/observability/test",
            post(observability::test_observability_export),
        )
        .route("/settings/secrets", get(list_settings_secrets))
        .route("/settings/secrets/reveal", post(reveal_settings_secrets))
        .route("/settings/secrets/upsert", post(upsert_settings_secret))
        .route("/settings/secrets/delete", post(delete_settings_secret))
        .route("/tunnel/providers", get(tunnel::get_tunnel_providers))
        .route("/tunnel/configure", post(tunnel::configure_tunnel))
        .route("/tunnel/test", post(tunnel::test_tunnel_connection))
        // Model pool routes
        .route("/models", get(list_models))
        .route("/models", post(add_model))
        .route("/models/{id}", axum::routing::put(update_model))
        .route("/models/{id}", axum::routing::delete(delete_model))
        .route(
            "/models/openai-subscription/oauth/start",
            post(start_codex_cli_oauth),
        )
        .route(
            "/models/openai-subscription/oauth/status",
            get(codex_cli_oauth_status),
        )
        .route("/models/codex/oauth/start", post(start_codex_cli_oauth))
        .route("/models/codex/oauth/status", get(codex_cli_oauth_status))
        .route("/models/discover/{provider}", get(discover_provider_models))
        .route("/profile", get(get_profile))
        .route("/restart", post(restart_server))
        // Self-update endpoints are intentionally left unmounted for now.
        .route("/trace", get(trace::get_trace))
        .route("/trace/{id}", get(trace::get_trace_detail))
        // Integrations routes
        .route("/integrations", get(integrations::list_integrations))
        .route(
            "/integrations/{id}/auth",
            get(integrations::get_integration_auth_url),
        )
        .route(
            "/integrations/{id}/disconnect",
            post(integrations::disconnect_integration),
        )
        .route(
            "/integrations/{id}/configure",
            post(integrations::configure_integration),
        )
        .route(
            "/integrations/{id}/enable",
            post(integrations::enable_integration),
        )
        .route(
            "/integrations/{id}/disable",
            post(integrations::disable_integration),
        )
        .route(
            "/integrations/{id}/test",
            post(integrations::test_integration),
        )
        .route("/gmail/configure", post(integrations::configure_gmail))
        // Calendar routes
        .route(
            "/calendar/configure",
            post(integrations::configure_calendar),
        )
        .route(
            "/calendar/oauth/start",
            post(integrations::calendar_oauth_start),
        )
        .route("/calendar/status", get(integrations::calendar_status))
        .route("/calendar/test", get(integrations::calendar_test))
        // SSH routes
        .route("/ssh/connections", get(ssh_list_connections))
        .route("/ssh/connections", post(ssh_add_connection))
        .route(
            "/ssh/connections/{name}",
            axum::routing::delete(ssh_remove_connection),
        )
        .route("/ssh/keys", get(ssh_list_keys))
        .route("/ssh/keys", post(ssh_upload_key))
        .route("/ssh/keys/{name}", axum::routing::delete(ssh_remove_key))
        .route("/ssh/test", post(ssh_test_connection))
        // Swarm routes
        .route("/swarm/status", get(swarm_status))
        .route("/swarm/agents", get(swarm_list_agents))
        .route("/swarm/agents", post(swarm_add_agent))
        .route(
            "/swarm/agents/{id}",
            post(swarm_update_agent).delete(swarm_remove_agent),
        )
        .route("/swarm/config", get(swarm_get_config))
        .route("/swarm/config", post(swarm_update_config))
        .route("/swarm/delegations", get(swarm_list_delegations))
        // Conversation routes
        .route("/conversations", get(list_conversations))
        .route("/conversations", post(create_conversation_endpoint))
        .route("/conversations/{id}", get(get_conversation_endpoint))
        .route(
            "/conversations/{id}",
            axum::routing::patch(update_conversation_endpoint),
        )
        .route(
            "/conversations/{id}",
            axum::routing::delete(delete_conversation_endpoint),
        )
        .route(
            "/conversations/{id}/messages",
            get(get_conversation_messages),
        )
        // Project routes
        .route("/projects", get(list_projects_endpoint))
        .route("/projects", post(create_project_endpoint))
        .route("/projects/{id}", get(get_project_endpoint))
        .route(
            "/projects/{id}",
            axum::routing::put(update_project_endpoint),
        )
        .route(
            "/projects/{id}",
            axum::routing::delete(delete_project_endpoint),
        )
        // Notification routes
        .route("/notifications", get(list_notifications_endpoint))
        .route("/notifications/stream", get(notification_stream_endpoint))
        .route("/notifications/read-all", post(mark_all_read_endpoint))
        .route("/notifications/{id}/read", post(mark_read_endpoint))
        .route("/notifications/count", get(notification_count_endpoint))
        // Analytics
        .route("/analytics/llm", get(llm_analytics_endpoint))
        // Document routes
        .route("/documents", get(list_documents_endpoint))
        .route("/documents/upload", post(upload_document_endpoint))
        .route(
            "/documents/upload-file",
            post(upload_document_file_endpoint),
        )
        .route(
            "/documents/{id}",
            axum::routing::delete(delete_document_endpoint),
        )
        .route("/documents/{id}/search", get(search_document_endpoint))
        // Memory consolidation
        .route("/memory/consolidate", post(trigger_consolidation))
        .route("/memory/stats", get(memory_stats))
        .route("/memory/episodes", get(list_episodes))
        .route("/memory/facts", get(list_facts))
        .route("/memory/preferences", get(list_user_preferences))
        .route("/memory/preferences", post(upsert_user_preference))
        .route(
            "/memory/preferences/{key}",
            axum::routing::delete(delete_user_preference),
        )
        .route("/memory/user-data", get(list_user_data_items))
        .route("/memory/user-data", post(create_user_data_item))
        .route(
            "/memory/user-data/{id}",
            axum::routing::delete(delete_user_data_item),
        )
        .route("/memory/knowledge", get(list_knowledge_items))
        .route("/memory/knowledge", post(create_knowledge_item))
        .route(
            "/memory/knowledge/{id}",
            axum::routing::delete(delete_knowledge_item),
        )
        // Code execution sandbox
        .route("/code/execute", post(execute_code))
        // Hook routes
        .route("/hooks", get(list_hooks))
        .route("/hooks/runs", get(list_hook_runs))
        .route("/hooks", post(add_hook))
        .route("/hooks/{id}", axum::routing::delete(remove_hook))
        // MCP (Model Context Protocol) routes
        .route("/mcp", post(mcp_handler))
        .route("/mcp/tools", get(mcp_list_tools))
        .route(
            "/mcp/servers",
            get(list_mcp_servers).post(create_mcp_server),
        )
        .route(
            "/mcp/servers/{id}",
            get(get_mcp_server)
                .put(update_mcp_server)
                .delete(delete_mcp_server),
        )
        .route("/mcp/servers/{id}/refresh", post(refresh_mcp_server))
        // Hosted apps management (protected)
        .route("/api/apps", get(list_apps))
        .route("/api/apps/{app_id}/stop", post(stop_app))
        .route("/api/apps/{app_id}/restart", post(restart_app))
        .route(
            "/api/apps/{app_id}/access-guard",
            post(update_app_access_guard),
        )
        .route("/api/apps/{app_id}", axum::routing::delete(delete_app))
        // Output file serving (code execution artifacts)
        .route("/api/outputs/{exec_id}/{filename}", get(serve_output_file))
        .route(
            "/api/outputs/{exec_id}/{filename}/download",
            get(download_output_file),
        )
        // File upload for chat attachments
        .route("/api/upload", post(upload_chat_file))
        .route("/api/uploads/{filename}", get(serve_upload_file))
        // Approval audit log
        .route("/approvals/log", get(get_approval_log))
        // Security event log
        .route("/security/logs", get(get_security_logs))
        // Security / Master Password
        .route("/security/status", get(security_status))
        .route("/security/set-password", post(set_master_password))
        .route("/security/change-password", post(change_master_password))
        .route("/security/remove-password", post(remove_master_password))
        // Tunnel management
        .route("/tunnel/status", get(tunnel::get_tunnel_status))
        .route("/tunnel/start", post(tunnel::start_tunnel))
        .route("/tunnel/stop", post(tunnel::stop_tunnel))
        // Watchers
        .route("/watchers", get(get_watchers))
        .route("/watchers/{id}", axum::routing::delete(delete_watcher))
        .route("/watchers/{id}/cancel", post(cancel_watcher))
        .route("/watchers/{id}/pause", post(pause_watcher))
        .route("/watchers/{id}/resume", post(resume_watcher))
        // Greetings (LLM-generated, cached in DB)
        // ArkPulse log
        .route("/arkpulse", get(get_pulse_log))
        .route("/arkpulse/trigger", post(trigger_pulse))
        .route("/arkpulse/fix", post(run_arkpulse_fix))
        // Moltbook automation and traceability
        .route("/moltbook/status", get(moltbook::get_moltbook_status))
        .route("/moltbook/log", get(moltbook::get_moltbook_log))
        .route("/moltbook/run", post(moltbook::run_moltbook_now))
        // Browser automation sessions
        .route("/browser/sessions", get(browser_list_sessions))
        .route("/browser/sessions/{id}/respond", post(browser_respond))
        .route("/browser/sessions/{id}/status", get(browser_session_status))
        // WhatsApp bridge proxy (so web UI can reach the sidecar)
        .route("/api/whatsapp-bridge/status", get(whatsapp_bridge_status))
        .route("/api/whatsapp-bridge/logout", post(whatsapp_bridge_logout))
        .route("/api/telegram/status", get(telegram_channel_status))
        // Apply rate limiting middleware (inner layer, runs after auth)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        // Apply authentication middleware (outer layer, runs first)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));

    // CORS layer - allow localhost + explicit configured origins + exact active tunnel origin
    let tunnel_for_cors = state.tunnel.clone();
    let explicit_origins: HashSet<String> = std::env::var("AGENTARK_ALLOWED_ORIGINS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .filter_map(normalize_origin)
                .collect::<HashSet<String>>()
        })
        .unwrap_or_default();
    let deployment_mode_for_cors = state.deployment_mode;
    if !explicit_origins.is_empty() {
        tracing::info!(
            "Additional allowed CORS origins configured: {}",
            explicit_origins.len()
        );
    }
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _| {
            if let Ok(origin_str) = origin.to_str() {
                if deployment_mode_for_cors == DeploymentMode::TrustedLocal
                    && is_local_origin(origin_str)
                {
                    return true;
                }

                let normalized = match normalize_origin(origin_str) {
                    Some(v) => v,
                    None => return false,
                };

                if explicit_origins.contains(&normalized) {
                    return true;
                }

                if let Ok(tunnel) = tunnel_for_cors.try_read() {
                    if let Some(tunnel_url) = tunnel.url.as_deref() {
                        if let Some(tunnel_origin) = normalize_origin(tunnel_url) {
                            return normalized == tunnel_origin;
                        }
                    }
                }

                false
            } else {
                false
            }
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true);

    // Keep handles for auto-start and ArkSentinel
    let tunnel_handle = state.tunnel.clone();
    let wa_bridge_handle = state.whatsapp_bridge.clone();

    let app = public_routes
        .merge(protected_routes)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            tunnel_exposure_middleware,
        ))
        .layer(cors);

    let isolate_public_apps = internet_facing_apps_should_be_isolated(
        state.deployment_mode,
        state.public_app_bind_addr.as_deref(),
    );
    let mut public_app_server: Option<tokio::task::JoinHandle<()>> = None;
    if isolate_public_apps {
        let mut public_app_state = state.clone();
        public_app_state.server_role = HttpServerRole::PublicApps;
        public_app_state.local_ui_bootstrap_enabled = false;
        public_app_state.allow_insecure_no_auth = false;
        let public_app_routes = Router::new()
            .route("/health", get(health))
            .route("/public/proxy/raw", get(public_proxy_raw))
            .route("/apps/{app_id}", any(serve_app_root))
            .route("/apps/{app_id}/", any(serve_app_root))
            .route("/apps/{app_id}/{*path}", any(serve_app_path))
            .with_state(public_app_state.clone());
        let public_app_bind_addr = state
            .public_app_bind_addr
            .clone()
            .unwrap_or_else(|| "127.0.0.1:8992".to_string());
        let public_app_listener = tokio::net::TcpListener::bind(&public_app_bind_addr).await?;
        let mut app_shutdown = shutdown_rx.clone();
        public_app_server = Some(tokio::spawn(async move {
            tracing::info!(
                "Public app server listening on http://{}",
                public_app_bind_addr
            );
            if let Err(error) = axum::serve(
                public_app_listener,
                public_app_routes.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = app_shutdown.changed().await;
            })
            .await
            {
                tracing::error!(
                    "Public app server on {} exited with error: {}",
                    public_app_bind_addr,
                    error
                );
            }
        }));
    }

    // Auto-start tunnel if AGENTARK_TUNNEL=true (for VPS users who can't access the UI)
    let auto_tunnel = std::env::var("AGENTARK_TUNNEL")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if auto_tunnel {
        let state_for_tunnel = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            tracing::info!("AGENTARK_TUNNEL=true - auto-starting public tunnel...");
            let provider = tunnel::load_tunnel_config(&state_for_tunnel).await.provider;
            if let Err(err) =
                tunnel_auth::ensure_control_plane_tunnel_ready(&state_for_tunnel, provider).await
            {
                tracing::error!(
                    "Skipping AGENTARK_TUNNEL auto-start because secure remote access requirements are not met: {}",
                    err.message()
                );
                return;
            }
            match tunnel::spawn_tunnel(&state_for_tunnel, None).await {
                Ok(()) => tracing::info!("Tunnel auto-started successfully"),
                Err(e) => tracing::error!("Failed to auto-start tunnel: {}", e),
            }
        });
    }

    // Always auto-start WhatsApp bridge - it's lightweight and just waits for QR scan
    {
        let wb = wa_bridge_handle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            tracing::info!("Auto-starting WhatsApp bridge...");
            match spawn_whatsapp_bridge(wb).await {
                Ok(()) => tracing::info!("WhatsApp bridge started"),
                Err(e) => tracing::warn!("WhatsApp bridge unavailable: {}", e),
            }
        });
    }

    // ArkSentinel: monitor tunnel + WhatsApp bridge processes, auto-restart if they die
    {
        let t = tunnel_handle.clone();
        let state_for_tunnel = state.clone();
        let wb = wa_bridge_handle.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;

                // Check tunnel process health
                let needs_restart = {
                    let mut tunnel = t.write().await;
                    if let Some(ref mut child) = tunnel.process {
                        match child.try_wait() {
                            Ok(Some(_status)) => {
                                // Process exited
                                tracing::warn!(
                                    "Tunnel process exited unexpectedly, will restart..."
                                );
                                tunnel.process = None;
                                tunnel.active = false;
                                tunnel.url = None;
                                tunnel.error = Some("Process exited, restarting...".to_string());
                                true
                            }
                            Ok(None) => false, // Still running
                            Err(_) => false,   // Can't check, assume ok
                        }
                    } else {
                        false
                    }
                };

                if needs_restart {
                    // Wait a bit before restart to avoid tight loops
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    tracing::info!("ArkSentinel: restarting tunnel...");
                    match tunnel::spawn_tunnel(&state_for_tunnel, None).await {
                        Ok(()) => tracing::info!("ArkSentinel: tunnel restarted successfully"),
                        Err(e) => tracing::error!("ArkSentinel: failed to restart tunnel: {}", e),
                    }
                }

                // Check WhatsApp bridge process health (only if it's supposed to be running)
                let wa_needs_restart = {
                    let mut bridge = wb.write().await;
                    if bridge.active {
                        if let Some(ref mut child) = bridge.process {
                            match child.try_wait() {
                                Ok(Some(_status)) => {
                                    tracing::warn!("ArkSentinel: WhatsApp bridge exited unexpectedly, will restart...");
                                    bridge.process = None;
                                    bridge.active = false;
                                    bridge.error =
                                        Some("Process exited, restarting...".to_string());
                                    true
                                }
                                Ok(None) => false,
                                Err(_) => false,
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if wa_needs_restart {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    tracing::info!("ArkSentinel: restarting WhatsApp bridge...");
                    match spawn_whatsapp_bridge(wb.clone()).await {
                        Ok(()) => {
                            tracing::info!("ArkSentinel: WhatsApp bridge restarted successfully")
                        }
                        Err(e) => {
                            tracing::error!("ArkSentinel: failed to restart WhatsApp bridge: {}", e)
                        }
                    }
                }
            }
        });
    }

    // Moltbook scheduler: twice-daily cadence (configurable), defer when server is busy.
    {
        let state_for_moltbook = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
            loop {
                interval.tick().await;
                // If disabled/off, don't run or log noise. Keep status keys updated for the UI.
                let storage = { state_for_moltbook.agent.read().await.storage.clone() };
                let settings = moltbook::load_moltbook_settings(&storage).await;
                if !settings.enabled {
                    let _ = storage.delete(moltbook::MOLTBOOK_NEXT_RUN_KEY).await;
                    let _ = storage.delete(moltbook::MOLTBOOK_DEFER_COUNT_KEY).await;
                    let _ = storage
                        .set(moltbook::MOLTBOOK_LAST_STATUS_KEY, b"disabled")
                        .await;
                    continue;
                }
                if settings.mode == "off" {
                    let _ = storage.delete(moltbook::MOLTBOOK_NEXT_RUN_KEY).await;
                    let _ = storage.delete(moltbook::MOLTBOOK_DEFER_COUNT_KEY).await;
                    let _ = storage
                        .set(moltbook::MOLTBOOK_LAST_STATUS_KEY, b"off_mode")
                        .await;
                    continue;
                }

                let result = moltbook::run_moltbook_cycle(&state_for_moltbook, "scheduler").await;
                if result.get("status").and_then(|v| v.as_str()) == Some("ok") {
                    tracing::info!(
                        "Moltbook scheduler run complete: {}",
                        result.get("run_id").and_then(|v| v.as_str()).unwrap_or("-")
                    );
                }
            }
        });
    }

    // Chat suggestion scanner: sweep chat wishes on a controlled cadence and defer while busy.
    {
        let state_for_suggestions = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
            loop {
                interval.tick().await;
                let storage = { state_for_suggestions.agent.read().await.storage.clone() };
                let scan_state = load_chat_suggestion_scan_state(&storage).await;
                if !chat_suggestion_scan_is_due(&scan_state, chrono::Utc::now()) {
                    continue;
                }
                let result = run_chat_suggestion_scan(&state_for_suggestions, "scheduler").await;
                if result.get("status").and_then(|value| value.as_str()) == Some("completed") {
                    tracing::info!(
                        "Chat suggestion scan completed: examined={} created={}",
                        result
                            .get("examined_chats")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0),
                        result
                            .get("created_suggestions")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                    );
                }
            }
        });
    }

    // Check TLS configuration
    let tls_cert = {
        let agent_guard = agent.read().await;
        (
            agent_guard.config.tls_cert_path.clone(),
            agent_guard.config.tls_key_path.clone(),
        )
    };

    #[cfg(feature = "tls")]
    if let (Some(cert_path), Some(key_path)) = tls_cert {
        let addr: SocketAddr = bind_addr
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;
        let rustls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert_path, &key_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to load TLS certs: {}", e))?;
        tracing::info!("HTTPS server listening on https://{}", bind_addr);
        tracing::info!("Web UI available at https://{}/", bind_addr);
        if bind_addr.starts_with("0.0.0.0") {
            tracing::warn!("Server bound to 0.0.0.0 -- accessible from all network interfaces. Ensure authentication is enabled.");
        }
        let handle = axum_server::Handle::new();
        let mut tls_shutdown = shutdown_rx.clone();
        let shutdown_task = {
            let handle = handle.clone();
            tokio::spawn(async move {
                let _ = tls_shutdown.changed().await;
                handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
            })
        };
        axum_server::bind_rustls(addr, rustls_config)
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
        let _ = shutdown_task.await;
        if let Some(mut handle) = public_app_server {
            match tokio::time::timeout(std::time::Duration::from_secs(10), &mut handle).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!("Public app server join failed during shutdown: {}", error)
                }
                Err(_) => {
                    tracing::warn!("Public app server did not stop within 10s; aborting task");
                    handle.abort();
                }
            }
        }
        return Ok(());
    }

    #[cfg(not(feature = "tls"))]
    let _ = tls_cert; // suppress unused warning

    // Plain HTTP fallback
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("HTTP server listening on http://{}", bind_addr);
    tracing::info!("Web UI available at http://{}/", bind_addr);
    if bind_addr.starts_with("0.0.0.0") {
        tracing::warn!("Server bound to 0.0.0.0 -- accessible from all network interfaces. Ensure authentication is enabled.");
    }
    if !bind_addr.starts_with("127.0.0.1") && !bind_addr.starts_with("localhost") {
        tracing::warn!(
            "Non-localhost bind without TLS -- traffic is unencrypted. Consider enabling TLS."
        );
    }

    let mut http_shutdown = shutdown_rx.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = http_shutdown.changed().await;
    })
    .await?;

    if let Some(mut handle) = public_app_server {
        match tokio::time::timeout(std::time::Duration::from_secs(10), &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!("Public app server join failed during shutdown: {}", error)
            }
            Err(_) => {
                tracing::warn!("Public app server did not stop within 10s; aborting task");
                handle.abort();
            }
        }
    }

    Ok(())
}

/// Serve the compiled V2 web UI. Issues the session cookie only for trusted UI bootstrap flows.
async fn web_ui(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut response = if let Some(index_html) = read_frontend_index_html() {
        Html(index_html).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "UI assets are missing. Build frontend assets to continue.",
        )
            .into_response()
    };
    if auth::should_issue_ui_session_cookie(&state, &headers, addr).await {
        let session_token = auth::current_ui_session_token(&state).await;
        auth::apply_session_cookie(
            &mut response,
            session_token.as_ref(),
            state.cookie_secure_default || auth::is_https_forwarded(&headers),
        );
    }
    response
}

/// Serve the compiled V2 UI directly.
async fn web_ui_v2(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut response = if let Some(index_html) = read_frontend_index_html() {
        Html(index_html).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "UI assets are missing. Build frontend assets to continue.",
        )
            .into_response()
    };
    if auth::should_issue_ui_session_cookie(&state, &headers, addr).await {
        let session_token = auth::current_ui_session_token(&state).await;
        auth::apply_session_cookie(
            &mut response,
            session_token.as_ref(),
            state.cookie_secure_default || auth::is_https_forwarded(&headers),
        );
    }
    response
}

fn normalize_host_for_compare(raw: &str) -> String {
    let host = raw.trim().trim_matches('"').trim_end_matches('.');
    if host.is_empty() {
        return String::new();
    }
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            return host[1..end].to_ascii_lowercase();
        }
    }
    if let Some(idx) = host.rfind(':') {
        let left = &host[..idx];
        if !left.is_empty() && !left.contains(':') {
            return left.to_ascii_lowercase();
        }
    }
    host.to_ascii_lowercase()
}

fn extract_request_host(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())?;
    let first = raw.split(',').next()?.trim();
    let normalized = normalize_host_for_compare(first);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn request_matches_active_tunnel(headers: &HeaderMap, tunnel_url: Option<&str>) -> bool {
    let Some(request_host) = extract_request_host(headers) else {
        return false;
    };

    if request_host.ends_with(".trycloudflare.com") || request_host.ends_with(".cfargotunnel.com") {
        return true;
    }

    let Some(url) = tunnel_url else {
        return false;
    };
    if let Ok(parsed) = reqwest::Url::parse(url) {
        if let Some(tunnel_host) = parsed.host_str() {
            return normalize_host_for_compare(tunnel_host) == request_host;
        }
    }
    false
}

fn redirect_to_selected_tunnel_app(app_id: &str) -> Response {
    let location = format!("/apps/{}/", app_id);
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn is_public_app_tunnel_path(path: &str) -> bool {
    path == "/public/proxy/raw"
        || path == "/public/proxy/raw/"
        || path == "/apps"
        || path == "/apps/"
        || path.starts_with("/apps/")
}

async fn tunnel_exposure_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let (tunnel_url, selected_app_id) = {
        let tunnel = state.tunnel.read().await;
        (tunnel.url.clone(), tunnel.selected_app_id.clone())
    };

    if !request_matches_active_tunnel(request.headers(), tunnel_url.as_deref()) {
        return next.run(request).await;
    }

    let path = request.uri().path();
    if let Some(selected_app_id) = selected_app_id
        .as_deref()
        .filter(|id| is_valid_app_id(id))
        .map(ToString::to_string)
    {
        if path == "/" || path == "/ui" || path == "/ui/" || path == "/ui/v2" {
            return redirect_to_selected_tunnel_app(&selected_app_id);
        }
        if is_public_app_tunnel_path(path) {
            return next.run(request).await;
        }

        return StatusCode::NOT_FOUND.into_response();
    }

    if tunnel_auth::is_public_tunnel_login_path(path)
        || tunnel_auth::is_public_tunnel_login_asset_path(path)
    {
        return next.run(request).await;
    }

    if tunnel_auth::is_control_plane_tunnel_authenticated(&state, request.headers()).await {
        return next.run(request).await;
    }

    if request.method() == Method::GET || request.method() == Method::HEAD {
        return tunnel_auth::redirect_to_tunnel_login(request.uri());
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Remote tunnel login required".to_string(),
        }),
    )
        .into_response()
}

async fn docs_blocked_for_tunnel(state: &AppState, headers: &HeaderMap) -> bool {
    let tunnel_url = { state.tunnel.read().await.url.clone() };
    request_matches_active_tunnel(headers, tunnel_url.as_deref())
}

async fn docs_is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let expected_key = match auth::sync_http_api_key_state(state, false).await {
        Ok((info, _rotated)) => info.map(|k| k.key),
        Err(_) => return false,
    };
    let Some(expected_key) = expected_key else {
        return true;
    };

    if let Some(auth_value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = auth_value
            .strip_prefix("Bearer ")
            .or_else(|| auth_value.strip_prefix("bearer "))
        {
            if token.trim() == expected_key {
                return true;
            }
        }
        if let Some(basic) = auth_value
            .strip_prefix("Basic ")
            .or_else(|| auth_value.strip_prefix("basic "))
        {
            if let Ok(decoded) = base64::engine::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                basic.trim(),
            ) {
                if let Ok(creds) = String::from_utf8(decoded) {
                    if let Some((username, password)) = creds.split_once(':') {
                        if password == expected_key || username == expected_key {
                            return true;
                        }
                    }
                }
            }
        }
    }

    let session_token = auth::current_ui_session_token(state).await;
    if auth::has_valid_ui_session_cookie(headers, session_token.as_deref()) {
        return true;
    }
    false
}

fn docs_auth_required_response() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Html(
            "Documentation is protected. Enter your API key as the password in the browser prompt.",
        ),
    )
        .into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"AgentArk Docs\""),
    );
    response
}

fn build_openapi_paths() -> serde_json::Map<String, serde_json::Value> {
    let mut paths = serde_json::Map::new();
    let mut add = |path: &str, method: &str, summary: &str, tag: &str| {
        let method_lc = method.to_ascii_lowercase();
        let entry = paths
            .entry(path.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(
                method_lc,
                serde_json::json!({
                    "tags": [tag],
                    "summary": summary,
                    "responses": {
                        "200": { "description": "OK" }
                    }
                }),
            );
        }
    };

    // --- Chat & Status ---
    add("/status", "GET", "Agent status", "Status");
    add("/chat", "POST", "Chat completion", "Chat");
    add("/chat/stream", "POST", "Streaming chat completion", "Chat");
    add("/chat/clear", "POST", "Clear current chat context", "Chat");

    // --- Skills ---
    add("/skills", "GET", "List skills", "Skills");
    add("/skills", "POST", "Create skill", "Skills");
    add("/skills/{name}", "GET", "Get skill content", "Skills");
    add("/skills/{name}", "POST", "Update skill content", "Skills");
    add("/skills/{name}", "DELETE", "Delete skill", "Skills");
    add(
        "/skills/{name}/enabled",
        "POST",
        "Enable/disable skill",
        "Skills",
    );
    add(
        "/skills/{name}/secrets",
        "GET",
        "Get skill secrets",
        "Skills",
    );
    add(
        "/skills/{name}/secrets",
        "POST",
        "Set skill secrets",
        "Skills",
    );
    add("/skills/{name}/test", "POST", "Test skill", "Skills");
    add(
        "/skills/import",
        "POST",
        "Import skill(s) from URL",
        "Skills",
    );

    // --- Tasks ---
    add("/tasks", "GET", "List tasks", "Tasks");
    add("/tasks", "POST", "Create task", "Tasks");
    add("/tasks/plan", "POST", "Plan task", "Tasks");
    add("/tasks/{id}", "POST", "Update task", "Tasks");
    add("/tasks/{id}", "DELETE", "Delete task", "Tasks");
    add("/tasks/{id}/retry", "POST", "Retry failed task", "Tasks");
    add("/tasks/{id}/approve", "POST", "Approve task", "Tasks");
    add("/tasks/{id}/reject", "POST", "Reject task", "Tasks");
    add(
        "/automation/objects",
        "GET",
        "Unified automation inventory",
        "Automation",
    );
    add(
        "/automation/runs",
        "GET",
        "Recent automation run history",
        "Automation",
    );

    // --- Goals ---
    add("/goals", "GET", "List goals", "Goals");
    add("/goals", "POST", "Create goal", "Goals");
    add("/goals/{id}", "DELETE", "Delete goal", "Goals");

    // --- Autonomy ---
    add(
        "/autonomy/settings",
        "GET",
        "Get autonomy settings",
        "Autonomy",
    );
    add(
        "/autonomy/settings",
        "POST",
        "Update autonomy settings",
        "Autonomy",
    );
    add(
        "/autonomy/briefing",
        "GET",
        "Get autonomy briefing",
        "Autonomy",
    );
    add(
        "/autonomy/suggestions/{id}",
        "GET",
        "Get a suggested automation detail",
        "Autonomy",
    );
    add(
        "/autonomy/suggestions/{id}/accept",
        "POST",
        "Accept a suggested automation draft",
        "Autonomy",
    );
    add(
        "/autonomy/suggestions/{id}/dismiss",
        "POST",
        "Dismiss a suggested automation draft",
        "Autonomy",
    );
    add(
        "/autonomy/incidents/live",
        "GET",
        "List live incidents",
        "Autonomy",
    );
    add(
        "/autonomy/timeline",
        "GET",
        "Get autonomy timeline",
        "Autonomy",
    );
    add(
        "/autonomy/timeline/rollback",
        "POST",
        "Rollback timeline event",
        "Autonomy",
    );
    // --- Settings & Models ---
    add("/settings", "GET", "Get settings", "Settings");
    add("/settings", "POST", "Update settings", "Settings");
    add(
        "/settings/observability/logs",
        "GET",
        "List observability export delivery logs",
        "Settings",
    );
    add(
        "/settings/observability/test",
        "POST",
        "Send a test observability trace",
        "Settings",
    );
    add(
        "/settings/evolution",
        "GET",
        "Get evolution control center status",
        "Settings",
    );
    add(
        "/settings/evolution",
        "POST",
        "Update evolution minimal settings",
        "Settings",
    );
    add(
        "/settings/evolution/dev",
        "GET",
        "Get evolution developer metrics",
        "Settings",
    );
    add(
        "/settings/evolution/dev/action",
        "POST",
        "Run evolution developer action",
        "Settings",
    );
    add(
        "/settings/api-key",
        "GET",
        "Get API key metadata",
        "Settings",
    );
    add(
        "/settings/api-key/regenerate",
        "POST",
        "Regenerate API key",
        "Settings",
    );
    add("/models", "GET", "List models", "Models");
    add("/models", "POST", "Add model", "Models");
    add("/models/{id}", "PUT", "Update model", "Models");
    add("/models/{id}", "DELETE", "Delete model", "Models");
    add(
        "/models/discover/{provider}",
        "GET",
        "Discover available models for a provider",
        "Models",
    );
    add(
        "/models/openai-subscription/oauth/start",
        "POST",
        "Start OpenAI Subscription browser OAuth",
        "Models",
    );
    add(
        "/models/openai-subscription/oauth/status",
        "GET",
        "Check OpenAI Subscription OAuth status",
        "Models",
    );
    add(
        "/models/codex/oauth/start",
        "POST",
        "Start OpenAI Subscription browser OAuth (legacy path)",
        "Models",
    );
    add(
        "/models/codex/oauth/status",
        "GET",
        "Check OpenAI Subscription OAuth status (legacy path)",
        "Models",
    );

    // --- Integrations ---
    add("/integrations", "GET", "List integrations", "Integrations");
    add(
        "/integrations/{id}/auth",
        "GET",
        "Integration auth URL",
        "Integrations",
    );
    add(
        "/integrations/{id}/configure",
        "POST",
        "Configure integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/enable",
        "POST",
        "Enable integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/disable",
        "POST",
        "Disable integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/test",
        "POST",
        "Test integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/disconnect",
        "POST",
        "Disconnect integration",
        "Integrations",
    );

    // --- Documents ---
    add("/documents", "GET", "List documents", "Documents");
    add("/documents/upload", "POST", "Upload document", "Documents");
    add(
        "/documents/upload-file",
        "POST",
        "Upload file document",
        "Documents",
    );
    add("/documents/{id}", "DELETE", "Delete document", "Documents");
    add(
        "/documents/{id}/search",
        "GET",
        "Search document",
        "Documents",
    );

    // --- Memory ---
    add(
        "/memory/stats",
        "GET",
        "Memory statistics by domain",
        "Memory",
    );
    add(
        "/memory/consolidate",
        "POST",
        "Run memory consolidation",
        "Memory",
    );
    add("/memory/episodes", "GET", "List episodic memory", "Memory");
    add("/memory/facts", "GET", "List semantic facts", "Memory");
    add(
        "/memory/preferences",
        "GET",
        "List user preferences",
        "Memory",
    );
    add(
        "/memory/preferences",
        "POST",
        "Create or update user preference",
        "Memory",
    );
    add(
        "/memory/preferences/{key}",
        "DELETE",
        "Delete user preference",
        "Memory",
    );
    add("/memory/user-data", "GET", "List user data items", "Memory");
    add(
        "/memory/user-data",
        "POST",
        "Create user data item",
        "Memory",
    );
    add(
        "/memory/user-data/{id}",
        "DELETE",
        "Delete user data item",
        "Memory",
    );
    add(
        "/memory/knowledge",
        "GET",
        "List knowledge base items",
        "Memory",
    );
    add(
        "/memory/knowledge",
        "POST",
        "Create knowledge base item",
        "Memory",
    );
    add(
        "/memory/knowledge/{id}",
        "DELETE",
        "Delete knowledge base item",
        "Memory",
    );

    // --- Notifications ---
    add(
        "/notifications",
        "GET",
        "List notifications",
        "Notifications",
    );
    add(
        "/notifications/count",
        "GET",
        "Notification count",
        "Notifications",
    );
    add(
        "/notifications/stream",
        "GET",
        "Live notification stream (SSE)",
        "Notifications",
    );
    add(
        "/notifications/read-all",
        "POST",
        "Mark all notifications read",
        "Notifications",
    );
    add(
        "/notifications/{id}/read",
        "POST",
        "Mark notification read",
        "Notifications",
    );

    // --- Projects & Conversations ---
    add("/projects", "GET", "List projects", "Projects");
    add("/projects", "POST", "Create project", "Projects");
    add("/projects/{id}", "GET", "Get project", "Projects");
    add("/projects/{id}", "PUT", "Update project", "Projects");
    add("/projects/{id}", "DELETE", "Delete project", "Projects");
    add(
        "/conversations",
        "GET",
        "List conversations",
        "Conversations",
    );
    add(
        "/conversations",
        "POST",
        "Create conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}",
        "GET",
        "Get conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}",
        "PATCH",
        "Update conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}",
        "DELETE",
        "Delete conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}/messages",
        "GET",
        "List conversation messages",
        "Conversations",
    );

    // --- MCP ---
    add("/mcp", "POST", "MCP request", "MCP");
    add("/mcp/tools", "GET", "List MCP tools", "MCP");
    add("/mcp/servers", "GET", "List MCP servers", "MCP");
    add("/mcp/servers", "POST", "Create MCP server", "MCP");
    add("/mcp/servers/{id}", "GET", "Get MCP server", "MCP");
    add("/mcp/servers/{id}", "PUT", "Update MCP server", "MCP");
    add("/mcp/servers/{id}", "DELETE", "Delete MCP server", "MCP");
    add(
        "/mcp/servers/{id}/refresh",
        "POST",
        "Refresh MCP server",
        "MCP",
    );

    // --- Security ---
    add("/security/status", "GET", "Security status", "Security");
    add("/security/logs", "GET", "Security logs", "Security");
    add(
        "/security/set-password",
        "POST",
        "Set master password",
        "Security",
    );
    add(
        "/security/change-password",
        "POST",
        "Change master password",
        "Security",
    );
    add(
        "/security/remove-password",
        "POST",
        "Remove master password",
        "Security",
    );

    // --- Tunnel ---
    add("/tunnel/status", "GET", "Tunnel status", "Tunnel");
    add(
        "/tunnel/providers",
        "GET",
        "List tunnel providers",
        "Tunnel",
    );
    add(
        "/tunnel/configure",
        "POST",
        "Save tunnel provider settings",
        "Tunnel",
    );
    add(
        "/tunnel/test",
        "POST",
        "Test selected tunnel provider",
        "Tunnel",
    );
    add("/tunnel/start", "POST", "Start tunnel", "Tunnel");
    add("/tunnel/stop", "POST", "Stop tunnel", "Tunnel");

    // --- Moltbook ---
    add("/moltbook/status", "GET", "Moltbook status", "Moltbook");
    add("/moltbook/log", "GET", "Moltbook activity log", "Moltbook");
    add("/moltbook/run", "POST", "Run Moltbook cycle", "Moltbook");

    // --- Swarm ---
    add("/swarm/status", "GET", "Swarm status", "Swarm");
    add("/swarm/agents", "GET", "List swarm agents", "Swarm");
    add("/swarm/agents", "POST", "Add swarm agent", "Swarm");
    add("/swarm/agents/{id}", "POST", "Update swarm agent", "Swarm");
    add(
        "/swarm/agents/{id}",
        "DELETE",
        "Remove swarm agent",
        "Swarm",
    );
    add("/swarm/config", "GET", "Get swarm config", "Swarm");
    add("/swarm/config", "POST", "Update swarm config", "Swarm");
    add(
        "/swarm/delegations",
        "GET",
        "List swarm delegations",
        "Swarm",
    );

    // --- Apps ---
    add("/api/apps", "GET", "List deployed apps", "Apps");
    add("/api/apps/{app_id}/stop", "POST", "Stop app", "Apps");
    add("/api/apps/{app_id}/restart", "POST", "Restart app", "Apps");
    add("/api/apps/{app_id}", "DELETE", "Delete app", "Apps");

    paths
}

async fn openapi_spec(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if docs_blocked_for_tunnel(&state, &headers).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !docs_is_authorized(&state, &headers).await {
        return docs_auth_required_response();
    }

    let security = serde_json::json!([
        { "BearerAuth": [] }
    ]);

    let spec = serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": "AgentArk API",
            "version": "1.0.0",
            "description": "Interactive API reference for AgentArk. Endpoints listed here require API key authentication."
        },
        "servers": [
            { "url": "/" }
        ],
        "tags": [
            { "name": "Status", "description": "Agent health and status" },
            { "name": "Chat", "description": "Send messages and stream responses" },
            { "name": "Skills", "description": "Manage agent skills and actions" },
            { "name": "Tasks", "description": "Create, schedule, and manage tasks" },
            { "name": "Goals", "description": "Long-term goal tracking" },
            { "name": "Autonomy", "description": "Autonomous operation settings, briefings, and incidents" },
            { "name": "Settings", "description": "Application settings and API keys" },
            { "name": "Models", "description": "LLM model configuration" },
            { "name": "Integrations", "description": "Third-party service connections" },
            { "name": "Documents", "description": "Document storage and semantic search" },
            { "name": "Memory", "description": "Episodic memory, user preferences, user data, and knowledge base" },
            { "name": "Notifications", "description": "Notification inbox and read status" },
            { "name": "Projects", "description": "Project workspace management" },
            { "name": "Conversations", "description": "Conversation history and messages" },
            { "name": "MCP", "description": "Model Context Protocol servers and tools" },
            { "name": "Security", "description": "Security logs and master password" },
            { "name": "Tunnel", "description": "Public tunnel for remote access" },
            { "name": "Moltbook", "description": "Moltbook lifecycle engine" },
            { "name": "Swarm", "description": "Multi-agent swarm coordination" },
            { "name": "Apps", "description": "Deployed app management" }
        ],
        "paths": build_openapi_paths(),
        "components": {
            "securitySchemes": {
                "BearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "API Key"
                }
            }
        },
        "security": security
    });
    (StatusCode::OK, Json(spec)).into_response()
}

async fn api_docs_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if docs_blocked_for_tunnel(&state, &headers).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !docs_is_authorized(&state, &headers).await {
        return docs_auth_required_response();
    }

    let html = r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>AgentArk API Docs</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
  <style>
 /* - AgentArk theme - exact match to app palette - */
    /* bg.default=#030711  bg.paper=#091527  primary=#2fd4ff  secondary=#14f195
       text.primary=#ecf5ff  text.secondary=#9bb4d6  border=rgba(106,150,198,0.22)
       card=linear-gradient(140deg,rgba(9,21,39,0.92),rgba(9,21,39,0.72))
       font='Space Grotesk','IBM Plex Sans','Segoe UI',sans-serif */

    body {
      margin: 0;
      background: #030711;
      color: #ecf5ff;
      font-family: 'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif;
    }
    .swagger-ui,
    .swagger-ui .wrapper { background: #030711; font-family: inherit; }
    .swagger-ui .topbar { display: none; }

    /* Info */
    .swagger-ui .info .title,
    .swagger-ui .info h1,
    .swagger-ui .info h2,
    .swagger-ui .info h3 { color: #ecf5ff; font-family: inherit; }
    .swagger-ui .info p,
    .swagger-ui .info li,
    .swagger-ui .info .markdown p { color: #9bb4d6; }
    .swagger-ui .info a { color: #2fd4ff; }

    /* Tag groups */
    .swagger-ui .opblock-tag {
      color: #ecf5ff !important;
      font-family: inherit !important;
      border-bottom: 1px solid rgba(106,150,198,0.22) !important;
    }
    .swagger-ui .opblock-tag:hover { background: rgba(47,212,255,0.04) !important; }
    .swagger-ui .opblock-tag small { color: #9bb4d6 !important; }
    .swagger-ui .opblock-tag svg { fill: #9bb4d6 !important; }

 /* Operation blocks - card-style matching MuiCard */
    .swagger-ui .opblock {
      background: linear-gradient(140deg, rgba(9,21,39,0.92), rgba(9,21,39,0.72)) !important;
      border: 1px solid rgba(106,150,198,0.22) !important;
      border-radius: 14px !important;
      backdrop-filter: blur(6px);
      box-shadow: none !important;
      margin-bottom: 8px;
    }
    .swagger-ui .opblock .opblock-summary {
      border-bottom: 1px solid rgba(106,150,198,0.15);
      border-radius: 14px 14px 0 0;
    }
    .swagger-ui .opblock .opblock-summary-method { font-weight: 700; border-radius: 6px; }

    /* GET = primary cyan */
    .swagger-ui .opblock.opblock-get { border-color: rgba(47,212,255,0.22) !important; }
    .swagger-ui .opblock.opblock-get .opblock-summary-method { background: #2fd4ff; color: #030711; }
    .swagger-ui .opblock.opblock-get .opblock-summary { background: rgba(47,212,255,0.05); }

    /* POST = secondary green */
    .swagger-ui .opblock.opblock-post { border-color: rgba(20,241,149,0.22) !important; }
    .swagger-ui .opblock.opblock-post .opblock-summary-method { background: #14f195; color: #030711; }
    .swagger-ui .opblock.opblock-post .opblock-summary { background: rgba(20,241,149,0.05); }

    /* PUT = amber */
    .swagger-ui .opblock.opblock-put { border-color: rgba(252,161,48,0.22) !important; }
    .swagger-ui .opblock.opblock-put .opblock-summary-method { background: #fca130; color: #030711; }
    .swagger-ui .opblock.opblock-put .opblock-summary { background: rgba(252,161,48,0.04); }

    /* DELETE = red */
    .swagger-ui .opblock.opblock-delete { border-color: rgba(249,62,62,0.22) !important; }
    .swagger-ui .opblock.opblock-delete .opblock-summary-method { background: #f93e3e; color: #fff; }
    .swagger-ui .opblock.opblock-delete .opblock-summary { background: rgba(249,62,62,0.04); }

    /* PATCH = teal */
    .swagger-ui .opblock.opblock-patch { border-color: rgba(20,241,149,0.16) !important; }
    .swagger-ui .opblock.opblock-patch .opblock-summary-method { background: #50e3c2; color: #030711; }
    .swagger-ui .opblock.opblock-patch .opblock-summary { background: rgba(80,227,194,0.04); }

    .swagger-ui .opblock .opblock-summary-path,
    .swagger-ui .opblock .opblock-summary-path__deprecated,
    .swagger-ui .opblock .opblock-summary-description { color: #ecf5ff !important; }

    /* Expanded operation body */
    .swagger-ui .opblock-body { background: rgba(3,7,17,0.6) !important; }
    .swagger-ui .opblock-body pre,
    .swagger-ui .opblock-body pre.example {
      background: #030711 !important;
      color: #9bb4d6 !important;
      border: 1px solid rgba(106,150,198,0.22) !important;
      border-radius: 10px !important;
    }
    .swagger-ui .opblock-section-header {
      background: #091527 !important;
      border-bottom: 1px solid rgba(106,150,198,0.22) !important;
    }
    .swagger-ui .opblock-section-header h4 { color: #ecf5ff !important; }

    /* Tables */
    .swagger-ui table thead tr th,
    .swagger-ui table thead tr td { color: #9bb4d6 !important; border-bottom: 1px solid rgba(106,150,198,0.22) !important; }
    .swagger-ui .parameter__name,
    .swagger-ui .parameter__type { color: #ecf5ff !important; }
    .swagger-ui .parameter__name.required::after { color: #f93e3e !important; }
    .swagger-ui table tbody tr td { color: #9bb4d6 !important; border-bottom: 1px solid rgba(106,150,198,0.10) !important; }

    /* Models */
    .swagger-ui section.models { border: 1px solid rgba(106,150,198,0.22) !important; border-radius: 14px !important; }
    .swagger-ui section.models h4 { color: #ecf5ff !important; }
    .swagger-ui .model-container { background: #091527 !important; }
    .swagger-ui .model { color: #9bb4d6 !important; }

 /* Buttons - match MuiButton */
    .swagger-ui .btn {
      color: #ecf5ff;
      border-color: rgba(106,150,198,0.22);
      background: transparent;
      text-transform: none;
      font-weight: 600;
      border-radius: 10px;
      font-family: inherit;
    }
    .swagger-ui .btn:hover { background: rgba(47,212,255,0.08); }
    .swagger-ui .btn.authorize { color: #14f195; border-color: #14f195; }
    .swagger-ui .btn.authorize svg { fill: #14f195; }
    .swagger-ui .btn.execute { background: #2fd4ff; border-color: #2fd4ff; color: #030711; font-weight: 700; }

    /* Auth modal */
    .swagger-ui .dialog-ux .modal-ux {
      background: #091527 !important;
      border: 1px solid rgba(106,150,198,0.22);
      border-radius: 14px;
    }
    .swagger-ui .dialog-ux .modal-ux-header h3 { color: #ecf5ff; font-family: inherit; }
    .swagger-ui .dialog-ux .modal-ux-content p,
    .swagger-ui .dialog-ux .modal-ux-content label { color: #9bb4d6; }

    /* Inputs */
    .swagger-ui input[type=text],
    .swagger-ui textarea,
    .swagger-ui select {
      background: #030711 !important;
      color: #ecf5ff !important;
      border: 1px solid rgba(106,150,198,0.22) !important;
      border-radius: 10px !important;
      font-family: inherit !important;
    }
    .swagger-ui input[type=text]:focus,
    .swagger-ui textarea:focus { border-color: #2fd4ff !important; outline: none; }

    /* Responses */
    .swagger-ui .responses-inner { background: transparent !important; }
    .swagger-ui .response-col_status { color: #ecf5ff !important; }
    .swagger-ui .response-col_description { color: #9bb4d6 !important; }

    /* Scrollbar */
    ::-webkit-scrollbar { width: 8px; height: 8px; }
    ::-webkit-scrollbar-track { background: #030711; }
    ::-webkit-scrollbar-thumb { background: rgba(106,150,198,0.28); border-radius: 4px; }
    ::-webkit-scrollbar-thumb:hover { background: rgba(106,150,198,0.40); }

    /* Scheme container / server selector */
    .swagger-ui .scheme-container {
      background: #091527 !important;
      border-bottom: 1px solid rgba(106,150,198,0.22);
      box-shadow: none;
    }
    .swagger-ui .scheme-container .schemes > label { color: #9bb4d6; }

    /* Loading */
    .swagger-ui .loading-container .loading::after { color: #2fd4ff; }
    .swagger-ui .wrapper { padding: 0 20px; }
    .swagger-ui .info { margin: 30px 0 20px 0; }

    /* Links everywhere */
    .swagger-ui a { color: #2fd4ff; }

    /* Custom header bar */
    .ark-header {
      padding: 16px 24px;
      border-bottom: 1px solid rgba(106,150,198,0.22);
      font-family: 'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif;
      background: linear-gradient(140deg, rgba(9,21,39,0.92), rgba(9,21,39,0.72));
      backdrop-filter: blur(6px);
      display: flex; align-items: center; gap: 14px;
    }
    .ark-header img {
      width: 36px; height: 36px;
      filter: drop-shadow(0 0 10px rgba(47,212,255,0.28));
    }
    .ark-header strong {
      color: #ecf5ff;
      font-size: 16px;
      font-weight: 700;
      letter-spacing: 0.8px;
      text-transform: uppercase;
      font-family: 'Orbitron', 'Space Grotesk', 'Segoe UI', sans-serif;
      text-shadow: 0 0 14px rgba(47,212,255,0.28);
    }
    .ark-header small { color: #9bb4d6; font-size: 12px; margin-left: 4px; }
  </style>
</head>
<body>
  <div class="ark-header">
    <img src="/logo.svg" alt="" />
    <div>
      <strong>AgentArk</strong>
      <small>API Docs &middot; /openapi.json</small>
    </div>
  </div>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    window.ui = SwaggerUIBundle({
      url: '/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      persistAuthorization: true,
      docExpansion: 'list',
      defaultModelsExpandDepth: -1,
      syntaxHighlight: { theme: 'monokai' }
    });
  </script>
</body>
</html>"#;
    (StatusCode::OK, Html(html)).into_response()
}

/// Serve built frontend assets from `frontend/dist/assets/*`.
async fn serve_frontend_asset(Path(path): Path<String>) -> Response {
    if !is_safe_asset_path(&path) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let rel = PathBuf::from("assets").join(&path);
    for base in frontend_dist_roots() {
        let file_path = base.join(&rel);
        if file_path.is_file() {
            if let Ok(bytes) = std::fs::read(&file_path) {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, mime_for_asset(&path))],
                    bytes,
                )
                    .into_response();
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

fn frontend_dist_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from(FRONTEND_DIST_DIR),
        PathBuf::from("./frontend/dist"),
        PathBuf::from("/app/frontend/dist"),
    ]
}

fn read_frontend_index_html() -> Option<String> {
    for root in frontend_dist_roots() {
        let index = root.join("index.html");
        if index.is_file() {
            if let Ok(html) = std::fs::read_to_string(index) {
                return Some(html);
            }
        }
    }
    None
}

fn is_safe_asset_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') {
        return false;
    }
    let clean = FsPath::new(path);
    clean
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn mime_for_asset(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else {
        "application/octet-stream"
    }
}

/// Serve PNG logo
async fn serve_logo_png() -> Response {
    // Try to include PNG at compile time, return 404 if not available
    {
        // Try to read from filesystem at runtime as fallback
        if let Ok(bytes) = tokio::fs::read("assets/logo.png").await {
            return ([(header::CONTENT_TYPE, "image/png")], bytes).into_response();
        }
        // Check common paths
        for path in &[
            "/app/assets/logo.png",
            "./assets/logo.png",
            "../assets/logo.png",
        ] {
            if let Ok(bytes) = tokio::fs::read(path).await {
                return ([(header::CONTENT_TYPE, "image/png")], bytes).into_response();
            }
        }
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Serve JPG logo
async fn serve_logo_jpg() -> Response {
    // Try to read from filesystem at runtime
    if let Ok(bytes) = tokio::fs::read("assets/logo.jpg").await {
        return ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response();
    }
    // Check common paths
    for path in &[
        "/app/assets/logo.jpg",
        "./assets/logo.jpg",
        "../assets/logo.jpg",
    ] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

/// Serve SVG logo (animated)
async fn serve_logo_svg() -> Response {
    // Try to read from filesystem at runtime
    if let Ok(bytes) = tokio::fs::read("assets/logo.svg").await {
        return ([(header::CONTENT_TYPE, "image/svg+xml")], bytes).into_response();
    }
    // Check common paths
    for path in &[
        "/app/assets/logo.svg",
        "./assets/logo.svg",
        "../assets/logo.svg",
    ] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return ([(header::CONTENT_TYPE, "image/svg+xml")], bytes).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

/// Serve output files from code execution (images, CSVs, code files, etc.)
async fn serve_output_file(
    State(state): State<AppState>,
    Path((exec_id, filename)): Path<(String, String)>,
) -> Response {
    // Validate exec_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&exec_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Validate filename has no path separators
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Resolve data directory from agent config
    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let file_path = data_dir.join("outputs").join(&exec_id).join(&filename);

    match tokio::fs::read(&file_path).await {
        Ok(bytes) => {
            let content_type = guess_content_type(&filename);
            (
                [
                    (header::CONTENT_TYPE, content_type.as_str()),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("inline; filename=\"{}\"", filename),
                    ),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Serve output file as a download (Content-Disposition: attachment)
async fn download_output_file(
    State(state): State<AppState>,
    Path((exec_id, filename)): Path<(String, String)>,
) -> Response {
    if uuid::Uuid::parse_str(&exec_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let file_path = data_dir.join("outputs").join(&exec_id).join(&filename);

    match tokio::fs::read(&file_path).await {
        Ok(bytes) => {
            let content_type = guess_content_type(&filename);
            (
                [
                    (header::CONTENT_TYPE, content_type.as_str()),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("attachment; filename=\"{}\"", filename),
                    ),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Guess MIME content type from filename extension.
/// Falls back to octet-stream for unknown types.
fn guess_content_type(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        // Documents
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        // Data
        "json" | "ipynb" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "yaml" | "yml" => "text/yaml",
        "toml" => "text/toml",
        // Code (all served as plain text for viewing)
        "txt" | "log" | "md" | "rst" => "text/plain",
        "py" | "js" | "ts" | "java" | "c" | "cpp" | "h" | "hpp" | "rs" | "go" | "rb" | "php"
        | "pl" | "lua" | "r" | "sh" | "bash" | "zsh" | "fish" | "kt" | "swift" | "sql" | "css"
        | "scss" | "less" => "text/plain; charset=utf-8",
        // Archives
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        // Audio/Video
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        // Office
        "xlsx" | "xls" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" | "doc" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" | "ppt" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        // Fallback
        _ => "application/octet-stream",
    }
    .to_string()
}

// ==================== Deployed Apps ====================

/// List all deployed apps
async fn list_apps(State(state): State<AppState>) -> Json<serde_json::Value> {
    let apps = state
        .app_registry
        .list()
        .await
        .into_iter()
        .map(|mut row| {
            let Some(obj) = row.as_object_mut() else {
                return row;
            };
            let Some(app_id) = obj
                .get("id")
                .and_then(|value| value.as_str())
                .map(|s| s.to_string())
            else {
                return row;
            };
            let access_key = obj
                .get("access_url")
                .and_then(|value| value.as_str())
                .and_then(access_key_from_access_url);
            obj.insert(
                "url".to_string(),
                serde_json::Value::String(app_root_url_for_state(&state, &app_id)),
            );
            obj.insert(
                "access_url".to_string(),
                serde_json::Value::String(app_access_url_for_state(
                    &state,
                    &app_id,
                    access_key.as_deref(),
                )),
            );
            row
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "apps": apps }))
}

fn is_valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 64
        && app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_secure_origin_request(headers: &axum::http::HeaderMap) -> bool {
    if headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
    {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    host.ends_with(".trycloudflare.com") || host.ends_with(".cfargotunnel.com")
}

fn should_upgrade_insecure_links(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.starts_with("text/html")
        || ct.starts_with("application/javascript")
        || ct.starts_with("text/javascript")
        || ct.starts_with("text/css")
}

fn rewrite_external_proxy_urls_for_public_apps(content: &str) -> String {
    const ARXIV_EXPORT_PLACEHOLDER: &str = "__AGENTARK_ARXIV_EXPORT_API__";
    const ARXIV_ROOT_PLACEHOLDER: &str = "__AGENTARK_ARXIV_ROOT_API__";

    content
        // Redirect direct ArXiv API calls through our same-origin public proxy
        // to avoid browser CORS failures on tunneled/public app URLs.
        .replace(
            "http://export.arxiv.org/api/query",
            ARXIV_EXPORT_PLACEHOLDER,
        )
        .replace(
            "https://export.arxiv.org/api/query",
            ARXIV_EXPORT_PLACEHOLDER,
        )
        .replace("http://arxiv.org/api/query", ARXIV_ROOT_PLACEHOLDER)
        .replace("https://arxiv.org/api/query", ARXIV_ROOT_PLACEHOLDER)
        .replace(
            "https://api.allorigins.win/raw?url=",
            "/public/proxy/raw?url=",
        )
        .replace("https://corsproxy.io/?", "/public/proxy/raw?url=")
        .replace(
            "https://api.codetabs.com/v1/proxy/?quest=",
            "/public/proxy/raw?url=",
        )
        .replace(
            ARXIV_EXPORT_PLACEHOLDER,
            "/public/proxy/raw?url=https://export.arxiv.org/api/query",
        )
        .replace(
            ARXIV_ROOT_PLACEHOLDER,
            "/public/proxy/raw?url=https://arxiv.org/api/query",
        )
}

fn inject_app_runtime_fetch_shims(content: &str, app_id: &str) -> String {
    if content.contains("__agentarkLlmProxyShimApplied") {
        return content.to_string();
    }
    let shim = format!(
        r#"<script>
(function() {{
  if (window.__agentarkLlmProxyShimApplied) return;
  window.__agentarkLlmProxyShimApplied = true;
  const APP_ID = "{app_id}";
  const PROXY_PATH = "/apps/" + encodeURIComponent(APP_ID) + "/__agentark/llm/chat";

  const nativeFetch = window.fetch ? window.fetch.bind(window) : null;
  if (nativeFetch) {{
    const extractUrl = (input) => {{
      try {{
        if (typeof input === "string") return input;
        if (input && typeof input.url === "string") return input.url;
        if (input instanceof URL) return input.toString();
      }} catch (_) {{}}
      return "";
    }};
    const shouldProxy = (url) => {{
      const lower = String(url || "").toLowerCase();
      return (
        lower.includes("openrouter.ai/api/v1/chat/completions") ||
        lower.includes("openrouter.ai/api/v1/responses") ||
        lower.includes("openrouter.ai/api/v1/completions") ||
        lower.includes("api.openai.com/v1/chat/completions") ||
        lower.includes("api.openai.com/v1/responses") ||
        lower.includes("api.openai.com/v1/completions") ||
        lower.endsWith("/v1/chat/completions") ||
        lower.endsWith("/v1/responses") ||
        lower.endsWith("/v1/completions")
      );
    }};
    window.fetch = function(input, init) {{
      const targetUrl = extractUrl(input);
      if (!shouldProxy(targetUrl)) {{
        return nativeFetch(input, init);
      }}
      const proxyInit = Object.assign({{}}, init || {{}});
      const inferredMethod = (
        proxyInit.method ||
        (input && input.method) ||
        "POST"
      )
        .toString()
        .toUpperCase();
      proxyInit.method = inferredMethod;
      if (inferredMethod !== "POST") {{
        return nativeFetch(input, init);
      }}
      const headers = new Headers(proxyInit.headers || (input && input.headers) || {{}});
      headers.delete("authorization");
      headers.delete("x-api-key");
      headers.set("content-type", "application/json");
      headers.set("x-agentark-app-proxy", "llm");
      proxyInit.headers = headers;
      return nativeFetch(PROXY_PATH, proxyInit);
    }};
  }}

  const nativePrompt = window.prompt ? window.prompt.bind(window) : null;
  if (nativePrompt) {{
    window.prompt = function(message, defaultValue) {{
      const text = String(message || "").toLowerCase();
      if (
        text.includes("api key") ||
        text.includes("openai") ||
        text.includes("openrouter") ||
        text.includes("anthropic")
      ) {{
        return "agentark-managed";
      }}
      return nativePrompt(message, defaultValue);
    }};
  }}
}})();
</script>"#
    );

    if content.contains("</head>") {
        return content.replacen("</head>", &format!("{}\n</head>", shim), 1);
    }
    if content.contains("</body>") {
        return content.replacen("</body>", &format!("{}\n</body>", shim), 1);
    }
    format!("{}\n{}", content, shim)
}

fn extract_openai_message_text(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(obj) = value.as_object() {
        if let Some(s) = obj.get("text").and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(s) = obj.get("content").and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Some(arr) = value.as_array() {
        let mut chunks = Vec::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                let item_type = obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text")
                    .to_ascii_lowercase();
                if item_type != "text" && item_type != "input_text" && item_type != "output_text" {
                    continue;
                }
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        chunks.push(text.trim().to_string());
                    }
                }
            } else if let Some(s) = item.as_str() {
                if !s.trim().is_empty() {
                    chunks.push(s.trim().to_string());
                }
            }
        }
        if !chunks.is_empty() {
            return Some(chunks.join("\n"));
        }
    }
    None
}

async fn app_scoped_llm_chat_proxy(
    state: &AppState,
    app_id: &str,
    headers: &axum::http::HeaderMap,
    body: axum::body::Body,
) -> Response {
    let has_proxy_header = headers
        .get("x-agentark-app-proxy")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("llm"));
    let referer_ok = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| reqwest::Url::parse(v).ok())
        .map(|url| {
            let path_prefix = format!("/apps/{}/", app_id);
            url.path().starts_with(&path_prefix) || url.path() == format!("/apps/{}", app_id)
        })
        .unwrap_or(false);
    if !has_proxy_header && !referer_ok {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "forbidden",
                "message": "app-scoped LLM proxy requires app-origin request context"
            })),
        )
            .into_response();
    }

    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response()
        }
    };
    let payload: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid JSON payload" })),
            )
                .into_response()
        }
    };

    let stream_requested = payload
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if stream_requested {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "streaming_not_supported",
                "message": "Use non-streaming chat completion for app proxy requests."
            })),
        )
            .into_response();
    }

    let mut system_lines: Vec<String> = Vec::new();
    let mut convo: Vec<(String, String)> = Vec::new();
    let requested_model_hint = payload
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if let Some(messages) = payload.get("messages").and_then(|v| v.as_array()) {
        for msg in messages {
            let Some(obj) = msg.as_object() else {
                continue;
            };
            let role = obj
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_ascii_lowercase();
            let Some(content_val) = obj.get("content") else {
                continue;
            };
            let Some(text) = extract_openai_message_text(content_val) else {
                continue;
            };
            if role == "system" {
                system_lines.push(text);
            } else {
                convo.push((role, text));
            }
        }
    }

    if convo.is_empty() {
        if let Some(input) = payload.get("input") {
            if let Some(text) = extract_openai_message_text(input) {
                convo.push(("user".to_string(), text));
            } else if let Some(arr) = input.as_array() {
                for item in arr {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    let role = obj
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_ascii_lowercase();
                    let content_val = obj.get("content").or_else(|| obj.get("text"));
                    let Some(content_val) = content_val else {
                        continue;
                    };
                    let Some(text) = extract_openai_message_text(content_val) else {
                        continue;
                    };
                    if role == "system" {
                        system_lines.push(text);
                    } else {
                        convo.push((role, text));
                    }
                }
            }
        }
    }

    if convo.is_empty() {
        if let Some(prompt) = payload.get("prompt").and_then(|v| v.as_str()) {
            let trimmed = prompt.trim();
            if !trimmed.is_empty() {
                convo.push(("user".to_string(), trimmed.to_string()));
            }
        }
    }

    if convo.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_messages",
                "message": "Provide messages[], input, or prompt."
            })),
        )
            .into_response();
    }

    let system_prompt = if system_lines.is_empty() {
        "You are a concise assistant helping summarize and explain app content for the end user."
            .to_string()
    } else {
        system_lines.join("\n")
    };

    let (last_role, last_text) = convo
        .pop()
        .unwrap_or_else(|| ("user".to_string(), String::new()));
    let user_message = if last_text.trim().is_empty() {
        "Please help with this request.".to_string()
    } else if last_role == "assistant" {
        format!(
            "Continue from the previous assistant context:\n{}",
            last_text
        )
    } else {
        last_text
    };

    let history: Vec<crate::core::ConversationMessage> = convo
        .into_iter()
        .map(|(role, content)| crate::core::ConversationMessage {
            role: if role == "assistant" {
                "assistant".to_string()
            } else {
                "user".to_string()
            },
            content,
            _timestamp: chrono::Utc::now(),
        })
        .collect();

    let (selected_llm, model_name, selection_note) = {
        let agent = state.agent.read().await;
        let (llm, _slot_label, note) =
            agent.select_llm_for_app_proxy(requested_model_hint.as_deref());
        let name = llm.model_name().to_string();
        (llm, name, note)
    };

    let no_actions: Vec<crate::actions::ActionDef> = Vec::new();
    let response = match selected_llm
        .chat_with_history(&system_prompt, &user_message, &history, &[], &no_actions)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "llm_proxy_failed",
                    "message": format!("LLM request failed: {}", e)
                })),
            )
                .into_response()
        }
    };
    let assistant_content = response.content;

    let openai_like = serde_json::json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "selection_note": selection_note,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": &assistant_content,
            },
            "text": &assistant_content,
            "finish_reason": "stop"
        }],
        "output_text": &assistant_content,
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": &assistant_content
            }]
        }],
        "status": "completed",
        "usage": response.usage.as_ref().map(|u| serde_json::json!({
            "prompt_tokens": u.prompt_tokens,
            "completion_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens
        }))
    });

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        openai_like.to_string(),
    )
        .into_response()
}

fn is_local_or_private_host_for_upgrade(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches('[')
        .trim_matches(']')
        .to_ascii_lowercase();
    if h.is_empty()
        || h == "localhost"
        || h.ends_with(".localhost")
        || h == "0.0.0.0"
        || h.ends_with(".local")
        || h.ends_with(".internal")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
            }
        };
    }
    false
}

fn upgrade_http_links_for_secure_origin(content: &str) -> String {
    static HTTP_URL_RE: OnceLock<Regex> = OnceLock::new();
    let re = HTTP_URL_RE.get_or_init(|| {
        Regex::new(r#"http://[A-Za-z0-9\.\-]+(?::\d+)?[^\s"'<>)]*"#)
            .expect("valid insecure URL regex")
    });
    re.replace_all(content, |caps: &regex::Captures| {
        let raw = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let Some(parsed) = reqwest::Url::parse(raw).ok() else {
            return raw.to_string();
        };
        let host = parsed.host_str().unwrap_or_default();
        if is_local_or_private_host_for_upgrade(host) {
            return raw.to_string();
        }
        raw.replacen("http://", "https://", 1)
    })
    .into_owned()
}

fn is_allowed_public_proxy_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h == "export.arxiv.org" || h == "arxiv.org" || h.ends_with(".arxiv.org")
}

/// Public proxy for static tunneled apps.
/// Strict allowlist prevents open-proxy abuse.
async fn public_proxy_raw(uri: Uri, Query(params): Query<HashMap<String, String>>) -> Response {
    // Accept both properly encoded URLs and pragmatic unencoded forms such as:
    // /public/proxy/raw?url=https://export.arxiv.org/api/query?search_query=...&start=0
    // The latter appears in generated JS that appends query params dynamically.
    let mut raw_url = params
        .get("url")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if let Some(raw_query) = uri.query().map(str::trim) {
        if raw_query.starts_with("url=http://") || raw_query.starts_with("url=https://") {
            raw_url = raw_query.trim_start_matches("url=").trim().to_string();
        }
    }
    let raw_url = raw_url.trim();
    if raw_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing query param: url" })),
        )
            .into_response();
    }

    let mut parsed = match reqwest::Url::parse(raw_url) {
        Ok(url) => url,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid url" })),
            )
                .into_response()
        }
    };

    let host = parsed.host_str().unwrap_or("");
    if !is_allowed_public_proxy_host(host) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "host not allowed" })),
        )
            .into_response();
    }

    if parsed.scheme() == "http" {
        let _ = parsed.set_scheme("https");
    }
    if parsed.scheme() != "https" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "only https urls are allowed" })),
        )
            .into_response();
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    match client.get(parsed).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": format!("upstream returned {}", resp.status())
                    })),
                )
                    .into_response();
            }
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            match resp.bytes().await {
                Ok(bytes) => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "no-store".to_string()),
                        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string()),
                    ],
                    bytes,
                )
                    .into_response(),
                Err(_) => StatusCode::BAD_GATEWAY.into_response(),
            }
        }
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

fn extract_query_param(query: Option<&str>, key: &str) -> Option<String> {
    query.and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes()).find_map(|(k, v)| {
            if k == key {
                Some(v.into_owned())
            } else {
                None
            }
        })
    })
}

fn strip_query_param(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut has_pairs = false;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        if k == key {
            continue;
        }
        serializer.append_pair(&k, &v);
        has_pairs = true;
    }
    if has_pairs {
        Some(serializer.finish())
    } else {
        None
    }
}

fn extract_cookie(headers: &axum::http::HeaderMap, cookie_name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').map(|c| c.trim()).find_map(|c| {
                c.strip_prefix(&format!("{}=", cookie_name))
                    .map(|v| v.to_string())
            })
        })
}

fn filter_proxy_cookie(cookie_header: &str, app_id: &str) -> Option<String> {
    let app_cookie = format!("ark_app_{}=", app_id);
    let filtered: Vec<&str> = cookie_header
        .split(';')
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .filter(|c| !c.starts_with("agentark_session="))
        .filter(|c| !c.starts_with(&app_cookie))
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered.join("; "))
    }
}

fn build_app_url(app_id: &str, path: &str, query: Option<&str>) -> String {
    let mut url = if path.is_empty() {
        format!("/apps/{}/", app_id)
    } else {
        format!("/apps/{}/{}", app_id, path.trim_start_matches('/'))
    };
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(q);
    }
    url
}

fn build_absolute_app_url(
    base_url: Option<&str>,
    app_id: &str,
    path: &str,
    query: Option<&str>,
) -> String {
    let relative = build_app_url(app_id, path, query);
    match base_url {
        Some(base) if !base.trim().is_empty() => {
            format!("{}{}", base.trim_end_matches('/'), relative)
        }
        _ => relative,
    }
}

fn app_root_url_for_state(state: &AppState, app_id: &str) -> String {
    build_absolute_app_url(state.public_app_base_url.as_deref(), app_id, "", None)
}

fn app_access_url_for_state(state: &AppState, app_id: &str, access_key: Option<&str>) -> String {
    match access_key.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!(
            "{}?key={}",
            app_root_url_for_state(state, app_id),
            urlencoding::encode(value)
        ),
        None => app_root_url_for_state(state, app_id),
    }
}

fn access_key_from_access_url(access_url: &str) -> Option<String> {
    access_url
        .split_once('?')
        .and_then(|(_, query)| extract_query_param(Some(query), "key"))
}

fn is_hop_by_hop_header(header_name: &str) -> bool {
    matches!(
        header_name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    let has_upgrade_token = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket_upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    has_upgrade_token && websocket_upgrade
}

fn axum_to_tungstenite_message(msg: AxumWsMessage) -> Option<TungsteniteMessage> {
    match msg {
        AxumWsMessage::Text(text) => Some(TungsteniteMessage::Text(text.to_string())),
        AxumWsMessage::Binary(data) => Some(TungsteniteMessage::Binary(data.to_vec())),
        AxumWsMessage::Ping(data) => Some(TungsteniteMessage::Ping(data.to_vec())),
        AxumWsMessage::Pong(data) => Some(TungsteniteMessage::Pong(data.to_vec())),
        AxumWsMessage::Close(_) => Some(TungsteniteMessage::Close(None)),
    }
}

fn tungstenite_to_axum_message(msg: TungsteniteMessage) -> Option<AxumWsMessage> {
    match msg {
        TungsteniteMessage::Text(text) => Some(AxumWsMessage::Text(text.to_string().into())),
        TungsteniteMessage::Binary(data) => Some(AxumWsMessage::Binary(data.into())),
        TungsteniteMessage::Ping(data) => Some(AxumWsMessage::Ping(data.into())),
        TungsteniteMessage::Pong(data) => Some(AxumWsMessage::Pong(data.into())),
        TungsteniteMessage::Close(_) => Some(AxumWsMessage::Close(None)),
        TungsteniteMessage::Frame(_) => None,
    }
}

async fn proxy_websocket_connection(
    client_socket: WebSocket,
    upstream_url: String,
    requested_protocols: Vec<String>,
    forward_headers: Vec<(String, String)>,
) {
    let mut upstream_request = match upstream_url.into_client_request() {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!("Failed to build upstream WS request: {}", error);
            return;
        }
    };
    if !requested_protocols.is_empty() {
        let protocols = requested_protocols.join(", ");
        if let Ok(value) = axum::http::HeaderValue::from_str(&protocols) {
            upstream_request
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", value);
        }
    }
    for (name, value) in forward_headers {
        if let (Ok(header_name), Ok(header_value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            upstream_request
                .headers_mut()
                .insert(header_name, header_value);
        }
    }

    let (upstream_socket, _) = match tokio_tungstenite::connect_async(upstream_request).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!("Failed to connect to upstream WS app: {}", error);
            return;
        }
    };

    let (mut client_sender, mut client_receiver) = client_socket.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream_socket.split();

    let client_to_upstream = async {
        while let Some(result) = client_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(upstream_message) = axum_to_tungstenite_message(message) else {
                        continue;
                    };
                    if upstream_sender.send(upstream_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Client WS receive error: {}", error);
                    break;
                }
            }
        }
        let _ = upstream_sender.close().await;
    };

    let upstream_to_client = async {
        while let Some(result) = upstream_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(client_message) = tungstenite_to_axum_message(message) else {
                        continue;
                    };
                    if client_sender.send(client_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Upstream WS receive error: {}", error);
                    break;
                }
            }
        }
        let _ = client_sender.send(AxumWsMessage::Close(None)).await;
    };

    tokio::select! {
        _ = client_to_upstream => {}
        _ = upstream_to_client => {}
    }
}

/// Serve app root - static files or reverse proxy
async fn serve_app_root(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    request: Request,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let ws = if is_websocket_upgrade(&parts.headers) {
        WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .ok()
    } else {
        None
    };

    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    serve_app_file_inner(
        &state,
        &app_id,
        "",
        AppServeRequestContext {
            method,
            uri,
            headers,
            ws,
            body,
        },
    )
    .await
}

/// Serve app file by path - static files or reverse proxy
async fn serve_app_path(
    State(state): State<AppState>,
    Path((app_id, path)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let ws = if is_websocket_upgrade(&parts.headers) {
        WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .ok()
    } else {
        None
    };

    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    serve_app_file_inner(
        &state,
        &app_id,
        &path,
        AppServeRequestContext {
            method,
            uri,
            headers,
            ws,
            body,
        },
    )
    .await
}

struct AppServeRequestContext {
    method: Method,
    uri: Uri,
    headers: axum::http::HeaderMap,
    ws: Option<WebSocketUpgrade>,
    body: axum::body::Body,
}

/// Inner handler: serve static file or reverse proxy to dynamic app
async fn serve_app_file_inner(
    state: &AppState,
    app_id: &str,
    path: &str,
    request_ctx: AppServeRequestContext,
) -> Response {
    let method = request_ctx.method;
    let uri = request_ctx.uri;
    let headers = request_ctx.headers;
    let ws = request_ctx.ws;
    let body = request_ctx.body;

    if !is_valid_app_id(app_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    if state.server_role == HttpServerRole::ControlPlane
        && internet_facing_apps_should_be_isolated(
            state.deployment_mode,
            state.public_app_bind_addr.as_deref(),
        )
    {
        let target = build_absolute_app_url(
            state.public_app_base_url.as_deref(),
            app_id,
            path,
            uri.query(),
        );
        if target.starts_with("http://") || target.starts_with("https://") {
            return Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header(header::LOCATION, target)
                .body(axum::body::Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Public apps are isolated onto a dedicated app origin in internet-facing mode.",
        )
            .into_response();
    }

    // Check app existence first so unknown IDs return 404 instead of auth form.
    let Some(app_dir) = state.app_registry.get_dir(app_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let access_guard_enabled = state.app_registry.access_guard_enabled(app_id).await;
    let cookie_name = format!("ark_app_{}", app_id);
    let key_from_query = if access_guard_enabled {
        extract_query_param(uri.query(), "key")
    } else {
        None
    };
    let key_from_cookie = if access_guard_enabled {
        extract_cookie(&headers, &cookie_name)
    } else {
        None
    };
    let key_from_header = if access_guard_enabled {
        headers
            .get("x-agentark-app-key")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    } else {
        None
    };
    let is_ws_request = is_websocket_upgrade(&headers);

    if access_guard_enabled {
        let query_valid = match key_from_query.as_deref() {
            Some(key) => state.app_registry.verify_key(app_id, key).await,
            None => false,
        };
        let cookie_valid = match key_from_cookie.as_deref() {
            Some(key) => state.app_registry.verify_key(app_id, key).await,
            None => false,
        };
        let header_valid = match key_from_header.as_deref() {
            Some(key) => state.app_registry.verify_key(app_id, key).await,
            None => false,
        };

        if !query_valid && !cookie_valid && !header_valid {
            return app_access_denied_page(app_id);
        }

        // First successful key entry: set cookie and redirect to clean URL.
        if query_valid && !cookie_valid && !header_valid && method == Method::GET && !is_ws_request
        {
            if let Some(key) = key_from_query {
                let request_proto = headers
                    .get("x-forwarded-proto")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("http");
                let secure_attr = if request_proto.eq_ignore_ascii_case("https") {
                    "; Secure"
                } else {
                    ""
                };
                let cookie = format!(
                    "{}={}; Path=/apps/{}; HttpOnly; SameSite=Lax; Max-Age=604800{}",
                    cookie_name, key, app_id, secure_attr
                );
                let clean_query = strip_query_param(uri.query(), "key");
                let clean_url = build_app_url(app_id, path, clean_query.as_deref());
                return Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::SET_COOKIE, cookie)
                    .header(header::LOCATION, clean_url)
                    .body(axum::body::Body::empty())
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
        }
    }

    state.app_registry.touch(app_id).await;
    let clean_query = if access_guard_enabled {
        strip_query_param(uri.query(), "key")
    } else {
        uri.query().map(|q| q.to_string())
    };
    let normalized_path = path.trim_start_matches('/');
    if normalized_path.eq_ignore_ascii_case("__agentark/llm/chat") {
        if method != Method::POST {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return app_scoped_llm_chat_proxy(state, app_id, &headers, body).await;
    }

    if let Some(port) = state.app_registry.get_port(app_id).await {
        if is_ws_request {
            if method != Method::GET {
                return StatusCode::METHOD_NOT_ALLOWED.into_response();
            }

            let Some(ws_upgrade) = ws else {
                return (StatusCode::BAD_REQUEST, "Invalid websocket upgrade request")
                    .into_response();
            };

            let upstream_path = path.trim_start_matches('/');
            let mut upstream_url = if upstream_path.is_empty() {
                format!("ws://127.0.0.1:{}/", port)
            } else {
                format!("ws://127.0.0.1:{}/{}", port, upstream_path)
            };
            if let Some(q) = clean_query.as_deref().filter(|q| !q.is_empty()) {
                upstream_url.push('?');
                upstream_url.push_str(q);
            }

            let requested_protocols = headers
                .get("Sec-WebSocket-Protocol")
                .and_then(|v| v.to_str().ok())
                .map(|raw| {
                    raw.split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();
            let mut ws_forward_headers: Vec<(String, String)> = Vec::new();
            if let Some(v) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
                ws_forward_headers.push(("origin".to_string(), v.to_string()));
            }
            if let Some(v) = headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
            {
                ws_forward_headers.push(("user-agent".to_string(), v.to_string()));
            }
            if let Some(v) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
                ws_forward_headers.push(("x-forwarded-host".to_string(), v.to_string()));
            }
            let forwarded_proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("http");
            ws_forward_headers.push(("x-forwarded-proto".to_string(), forwarded_proto.to_string()));
            ws_forward_headers.push((
                "x-forwarded-prefix".to_string(),
                format!("/apps/{}", app_id),
            ));
            if let Some(raw_cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
                if let Some(filtered) = filter_proxy_cookie(raw_cookie, app_id) {
                    ws_forward_headers.push(("cookie".to_string(), filtered));
                }
            }

            let ws_upgrade = if requested_protocols.is_empty() {
                ws_upgrade
            } else {
                ws_upgrade.protocols(requested_protocols.clone())
            };
            return ws_upgrade
                .on_upgrade(move |socket| async move {
                    proxy_websocket_connection(
                        socket,
                        upstream_url,
                        requested_protocols,
                        ws_forward_headers,
                    )
                    .await;
                })
                .into_response();
        }

        let upstream_path = path.trim_start_matches('/');
        let mut target_url = if upstream_path.is_empty() {
            format!("http://127.0.0.1:{}/", port)
        } else {
            format!("http://127.0.0.1:{}/{}", port, upstream_path)
        };
        if let Some(q) = clean_query.as_deref().filter(|q| !q.is_empty()) {
            target_url.push('?');
            target_url.push_str(q);
        }

        let body_bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response()
            }
        };

        let client = reqwest::Client::new();
        let mut upstream = client.request(method.clone(), &target_url);
        for (name, value) in &headers {
            let lower = name.as_str().to_ascii_lowercase();
            if is_hop_by_hop_header(&lower)
                || lower == "host"
                || lower == "content-length"
                || lower == "authorization"
            {
                continue;
            }
            if lower == "cookie" {
                if let Ok(raw_cookie) = value.to_str() {
                    if let Some(filtered) = filter_proxy_cookie(raw_cookie, app_id) {
                        upstream = upstream.header(header::COOKIE, filtered);
                    }
                }
                continue;
            }
            upstream = upstream.header(name, value);
        }
        if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
            upstream = upstream.header("x-forwarded-host", host);
        }
        let forwarded_proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("http");
        upstream = upstream
            .header("x-forwarded-proto", forwarded_proto)
            .header("x-forwarded-prefix", format!("/apps/{}", app_id))
            .body(body_bytes);

        match upstream.send().await {
            Ok(resp) => {
                let status =
                    StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                let response_headers = resp.headers().clone();
                match resp.bytes().await {
                    Ok(response_body) => {
                        let mut builder = Response::builder().status(status);
                        for (name, value) in &response_headers {
                            if !is_hop_by_hop_header(name.as_str()) {
                                builder = builder.header(name, value);
                            }
                        }
                        let response_body = if method == Method::HEAD {
                            axum::body::Body::empty()
                        } else {
                            axum::body::Body::from(response_body)
                        };
                        builder
                            .body(response_body)
                            .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
                    }
                    Err(_) => StatusCode::BAD_GATEWAY.into_response(),
                }
            }
            Err(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "App server not responding").into_response()
            }
        }
    } else if state.app_registry.is_static(app_id).await {
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }

        let relative_path = path.trim_start_matches('/');
        let relative_path = if relative_path.is_empty() {
            "index.html"
        } else {
            relative_path
        };
        if relative_path.contains('\0') {
            return StatusCode::BAD_REQUEST.into_response();
        }

        let app_root = match tokio::fs::canonicalize(&app_dir).await {
            Ok(path) => path,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };

        let mut canonical_file = match tokio::fs::canonicalize(app_dir.join(relative_path)).await {
            Ok(path) => path,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };
        if !canonical_file.starts_with(&app_root) {
            return StatusCode::FORBIDDEN.into_response();
        }

        if tokio::fs::metadata(&canonical_file)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            let index_path = canonical_file.join("index.html");
            let index_canonical = match tokio::fs::canonicalize(index_path).await {
                Ok(path) => path,
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            };
            if !index_canonical.starts_with(&app_root) {
                return StatusCode::FORBIDDEN.into_response();
            }
            canonical_file = index_canonical;
        }

        match tokio::fs::read(&canonical_file).await {
            Ok(bytes) => {
                let filename = canonical_file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("index.html");
                let content_type = guess_content_type(filename);
                let mut response_bytes = bytes;
                if should_upgrade_insecure_links(&content_type) {
                    let mut rewritten = String::from_utf8_lossy(&response_bytes).into_owned();
                    rewritten = rewrite_external_proxy_urls_for_public_apps(&rewritten);
                    if content_type.to_ascii_lowercase().starts_with("text/html") {
                        rewritten = inject_app_runtime_fetch_shims(&rewritten, app_id);
                    }
                    if is_secure_origin_request(&headers) {
                        rewritten = upgrade_http_links_for_secure_origin(&rewritten);
                    }
                    response_bytes = rewritten.into_bytes();
                }
                if method == Method::HEAD {
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, content_type),
                            (header::CACHE_CONTROL, "no-store".to_string()),
                        ],
                        Vec::<u8>::new(),
                    )
                        .into_response()
                } else {
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, content_type),
                            (header::CACHE_CONTROL, "no-store".to_string()),
                        ],
                        response_bytes,
                    )
                        .into_response()
                }
            }
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Access denied page for apps with invalid/missing access key
fn app_access_denied_page(app_id: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Access Key Required</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{display:flex;justify-content:center;align-items:center;min-height:100vh;font-family:system-ui,-apple-system,sans-serif;background:#0f0f1a;color:#e0e0e0}}
.card{{background:#1a1a2e;border:1px solid #2a2a3e;border-radius:16px;padding:40px;max-width:400px;width:90%;text-align:center}}
h2{{margin-bottom:8px;font-size:1.4em}}
p{{color:#888;margin-bottom:24px;font-size:0.95em}}
input{{width:100%;padding:12px 16px;border-radius:8px;border:1px solid #333;background:#12121e;color:#fff;font-size:1em;outline:none;transition:border 0.2s}}
input:focus{{border-color:#7c3aed}}
button{{margin-top:16px;width:100%;padding:12px;border-radius:8px;border:none;background:#7c3aed;color:#fff;font-size:1em;cursor:pointer;transition:background 0.2s}}
button:hover{{background:#6d28d9}}
</style></head>
<body><div class="card">
<h2>Access Key Required</h2>
<p>This app is protected. Enter the access key to continue.</p>
<form method="GET" action="/apps/{app_id}/">
<input type="text" name="key" placeholder="Enter access key" autofocus required>
<button type="submit">Unlock</button>
</form>
</div></body></html>"#,
        app_id = app_id
    );
    Html(html).into_response()
}

#[derive(Debug, Deserialize)]
struct AppAccessGuardUpdateRequest {
    enabled: bool,
    #[serde(default)]
    regenerate_key: bool,
}

/// Stop a running app
async fn stop_app(State(state): State<AppState>, Path(app_id): Path<String>) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }
    if state.app_registry.get_dir(&app_id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "App not found" })),
        )
            .into_response();
    }
    if state.app_registry.is_static(&app_id).await {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Static apps cannot be stopped" })),
        )
            .into_response();
    }
    match state.app_registry.stop_runtime(&app_id).await {
        Ok(_) => {
            trigger_arkpulse_after_app_change(&state, "app_stop_runtime").await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "stopped", "app_id": app_id })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn update_app_access_guard(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(request): Json<AppAccessGuardUpdateRequest>,
) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }

    match state
        .app_registry
        .set_access_guard(&app_id, request.enabled, request.regenerate_key)
        .await
    {
        Ok(access_key) => {
            trigger_arkpulse_after_app_change(&state, "app_access_guard_update").await;
            let access_url = app_access_url_for_state(
                &state,
                &app_id,
                request.enabled.then_some(access_key.as_str()),
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "app_id": app_id,
                    "access_guard_enabled": request.enabled,
                    "access_key": if request.enabled { access_key } else { String::new() },
                    "access_url": access_url,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let status = if e.to_string() == "App not found" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

/// Restart an app from saved metadata
async fn restart_app(State(state): State<AppState>, Path(app_id): Path<String>) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }

    let app_dir = if let Some(path) = state.app_registry.get_dir(&app_id).await {
        path
    } else {
        let data_dir = {
            let agent = state.agent.read().await;
            agent.data_dir().to_path_buf()
        };
        let fallback = data_dir.join("apps").join(&app_id);
        if !fallback.exists() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "App not found" })),
            )
                .into_response();
        }
        fallback
    };

    let meta_path = app_dir.join(".app_meta.json");
    let mut meta: serde_json::Value = match tokio::fs::read(&meta_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    if !meta.is_object() {
        meta = serde_json::json!({});
    }

    let title = meta
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&app_id)
        .to_string();
    let entry_command = meta
        .get("entry_command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let install_command = meta
        .get("install_command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let runtime_image = meta
        .get("runtime_image")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let runtime_preference = crate::actions::app::runtime_preference_from_opt(
        meta.get("runtime_preference").and_then(|v| v.as_str()),
    );
    let required_inputs = crate::actions::app::parse_required_inputs(&meta);
    let config_values: std::collections::HashMap<String, String> = meta
        .get("config_values")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    let value = match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => return None,
                    };
                    Some((k.clone(), value))
                })
                .collect()
        })
        .unwrap_or_default();
    let access_guard_enabled = meta
        .get("access_guard_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let access_key = meta
        .get("access_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if access_guard_enabled {
                crate::actions::app::generate_access_key()
            } else {
                String::new()
            }
        });

    if meta.get("access_guard_enabled").is_none()
        || (access_guard_enabled && meta.get("access_key").is_none())
    {
        meta["access_guard_enabled"] = serde_json::Value::Bool(access_guard_enabled);
        meta["access_key"] = serde_json::Value::String(access_key.clone());
        let _ = tokio::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        )
        .await;
    }

    if let Err(e) = state.app_registry.stop_runtime(&app_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to stop running app before restart: {}", e) })),
        )
            .into_response();
    }

    if let Some(entry_command) = entry_command {
        let Some(port) = state.app_registry.find_available_port().await else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "No available app port" })),
            )
                .into_response();
        };

        let (config_dir, data_dir, llm_env) = {
            let agent = state.agent.read().await;
            (
                agent.config_dir.clone(),
                agent.data_dir().to_path_buf(),
                agent.app_model_env_vars(),
            )
        };
        let (resolved_env, missing_sensitive, missing_config) =
            match crate::actions::app::resolve_required_env_values(
                &config_dir,
                &data_dir,
                &required_inputs,
                &llm_env,
                &config_values,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to resolve app secrets: {}", e) })),
                )
                    .into_response();
                }
            };
        if !missing_sensitive.is_empty() || !missing_config.is_empty() {
            let mut missing_all = missing_sensitive.clone();
            for m in &missing_config {
                if !missing_all.iter().any(|x| x == m) {
                    missing_all.push(m.clone());
                }
            }
            let required_secret_keys: Vec<String> = required_inputs
                .iter()
                .filter(|r| r.sensitive)
                .map(|r| r.key.clone())
                .collect();
            let required_config_keys: Vec<String> = required_inputs
                .iter()
                .filter(|r| !r.sensitive)
                .map(|r| r.key.clone())
                .collect();
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "status": "needs_secrets",
                    "app_id": app_id,
                    "missing_env": missing_sensitive,
                    "missing_config": missing_config,
                    "missing_inputs": missing_all,
                    "required_inputs": required_inputs,
                    "required_secrets": required_secret_keys.clone(),
                    "required_env": required_secret_keys,
                    "required_config": required_config_keys,
                    "message": "Missing required inputs. Use set secret KEY=VALUE for sensitive values; provide config for non-sensitive values."
                })),
            )
                .into_response();
        }

        match crate::actions::app::launch_dynamic_runtime(
            crate::actions::app::DynamicRuntimeLaunch {
                app_id: &app_id,
                app_dir: &app_dir,
                entry_command: &entry_command,
                install_command: install_command.as_deref(),
                port,
                extra_env: &resolved_env,
                runtime_image: runtime_image.as_deref(),
                runtime_preference,
                stream_tx: None,
            },
        )
        .await
        {
            Ok(runtime_handle) => {
                let (child, container_id) = match runtime_handle {
                    crate::actions::app::DynamicRuntimeHandle::Container(container_id) => {
                        (None, Some(container_id))
                    }
                    crate::actions::app::DynamicRuntimeHandle::Process(child) => {
                        (Some(*child), None)
                    }
                };
                let app_dir_for_diagnostics = app_dir.clone();
                state
                    .app_registry
                    .register_dynamic(
                        app_id.clone(),
                        crate::actions::app::DynamicAppRegistration {
                            title: title.clone(),
                            app_dir,
                            child,
                            container_id,
                            port,
                            access_key: access_key.clone(),
                            access_guard_enabled,
                        },
                    )
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !state.app_registry.runtime_is_alive(&app_id).await {
                    let logs = crate::actions::app::read_local_runtime_log_tail(
                        &app_dir_for_diagnostics,
                        4096,
                    )
                    .await;
                    let detail = if logs.is_empty() {
                        "App process stopped shortly after restart.".to_string()
                    } else {
                        format!(
                            "App process stopped shortly after restart. Recent runtime logs:\n{}",
                            logs
                        )
                    };
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({ "error": detail })),
                    )
                        .into_response();
                }
                trigger_arkpulse_after_app_change(&state, "app_restart").await;
                let app_url =
                    build_absolute_app_url(state.public_app_base_url.as_deref(), &app_id, "", None);
                let app_access_query = access_guard_enabled
                    .then(|| format!("key={}", urlencoding::encode(&access_key)));
                let app_access_url = build_absolute_app_url(
                    state.public_app_base_url.as_deref(),
                    &app_id,
                    "",
                    app_access_query.as_deref(),
                );
                Json(serde_json::json!({
                    "status": "restarted",
                    "type": "dynamic",
                    "app_id": app_id,
                    "title": title,
                    "url": app_url,
                    "access_url": app_access_url,
                    "access_guard_enabled": access_guard_enabled,
                    "port": port,
                    "runtime_preference": runtime_preference.as_str(),
                }))
                .into_response()
            }
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to restart app: {}", error) })),
            )
                .into_response(),
        }
    } else {
        state
            .app_registry
            .register_static(
                app_id.clone(),
                title.clone(),
                app_dir,
                access_key.clone(),
                access_guard_enabled,
            )
            .await;
        trigger_arkpulse_after_app_change(&state, "app_restart").await;
        let app_url =
            build_absolute_app_url(state.public_app_base_url.as_deref(), &app_id, "", None);
        let app_access_query =
            access_guard_enabled.then(|| format!("key={}", urlencoding::encode(&access_key)));
        let app_access_url = build_absolute_app_url(
            state.public_app_base_url.as_deref(),
            &app_id,
            "",
            app_access_query.as_deref(),
        );
        Json(serde_json::json!({
            "status": "restarted",
            "type": "static",
            "app_id": app_id,
            "title": title,
            "url": app_url,
            "access_url": app_access_url,
            "access_guard_enabled": access_guard_enabled,
            "runtime_preference": runtime_preference.as_str(),
        }))
        .into_response()
    }
}

/// Stop and delete an app from disk
async fn delete_app(State(state): State<AppState>, Path(app_id): Path<String>) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }

    let app_title: Option<String> = {
        let apps = state.app_registry.list().await;
        apps.iter()
            .find(|row| row.get("id").and_then(|v| v.as_str()) == Some(app_id.as_str()))
            .and_then(|row| row.get("title").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };

    let app_dir = if let Some(path) = state.app_registry.get_dir(&app_id).await {
        path
    } else {
        let data_dir = {
            let agent = state.agent.read().await;
            agent.data_dir().to_path_buf()
        };
        let fallback = data_dir.join("apps").join(&app_id);
        if !fallback.exists() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "App not found" })),
            )
                .into_response();
        }
        fallback
    };

    if let Err(e) = state.app_registry.stop(&app_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Failed to stop app before delete: {}", e) }),
            ),
        )
            .into_response();
    }
    match tokio::fs::remove_dir_all(&app_dir).await {
        Ok(_) => {
            trigger_arkpulse_after_app_change(&state, "app_delete").await;
            let deleted_notifications = {
                let agent = state.agent.read().await;
                agent
                    .storage
                    .delete_app_notifications(&app_id, app_title.as_deref())
                    .await
                    .unwrap_or(0)
            };
            Json(serde_json::json!({
                "status": "deleted",
                "app_id": app_id,
                "deleted_notifications": deleted_notifications
            }))
            .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to delete app files: {}", error) })),
        )
            .into_response(),
    }
}

/// Upload a file for use in chat (attachments for code execution, analysis, etc.)
async fn upload_chat_file(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let uploads_dir = data_dir.join("uploads");

    let mut uploaded_files = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let original_name: String = field.file_name().unwrap_or("unnamed").to_string();

        // Sanitize filename: keep only safe characters
        let safe_name: String = original_name
            .chars()
            .map(|c: char| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        // Prevent path traversal
        if safe_name.contains("..") || safe_name.starts_with('.') {
            return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
        }

        match field.bytes().await {
            Ok(data) => {
                // 50MB limit per file
                if data.len() > 50 * 1024 * 1024 {
                    return (StatusCode::PAYLOAD_TOO_LARGE, "File too large (50MB max)")
                        .into_response();
                }

                if let Err(e) = tokio::fs::create_dir_all(&uploads_dir).await {
                    tracing::error!("Failed to create uploads dir: {}", e);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }

                let file_path = uploads_dir.join(&safe_name);
                if let Err(e) = tokio::fs::write(&file_path, &data).await {
                    tracing::error!("Failed to write upload: {}", e);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }

                tracing::info!("File uploaded: {} ({} bytes)", safe_name, data.len());
                uploaded_files.push(serde_json::json!({
                    "name": safe_name,
                    "size": data.len(),
                    "path": format!("/api/uploads/{}", safe_name),
                    "local_path": file_path.to_string_lossy(),
                }));
            }
            Err(e) => {
                tracing::error!("Failed to read upload field: {}", e);
                return StatusCode::BAD_REQUEST.into_response();
            }
        }
    }

    if uploaded_files.is_empty() {
        return (StatusCode::BAD_REQUEST, "No files uploaded").into_response();
    }

    Json(serde_json::json!({ "files": uploaded_files })).into_response()
}

/// Serve uploaded files (for preview/download in chat)
async fn serve_upload_file(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    // Validate filename
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let file_path = data_dir.join("uploads").join(&filename);

    match tokio::fs::read(&file_path).await {
        Ok(bytes) => {
            let content_type = guess_content_type(&filename);
            (
                [
                    (header::CONTENT_TYPE, content_type.as_str()),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("inline; filename=\"{}\"", filename),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Health check endpoint
async fn health(State(state): State<AppState>) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    let storage_ok = storage.get("__health_probe").await.is_ok();
    let sqlite_quick_check = storage.sqlite_quick_check().await.ok();
    let sqlite_ok = sqlite_quick_check
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("ok"))
        .unwrap_or(false);
    let scheduler_heartbeat = storage
        .get(crate::sentinel::SENTINEL_SCHEDULER_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let watcher_heartbeat = storage
        .get(crate::sentinel::SENTINEL_WATCHER_HEARTBEAT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    let heartbeat_recent = |value: Option<&String>| {
        value
            .and_then(|raw| parse_utc_rfc3339(raw))
            .map(|ts| (chrono::Utc::now() - ts).num_seconds() <= 5 * 60)
            .unwrap_or(false)
    };
    let scheduler_loop_ok = if state.server_role == HttpServerRole::ControlPlane {
        heartbeat_recent(scheduler_heartbeat.as_ref())
    } else {
        true
    };
    let watcher_loop_ok = if state.server_role == HttpServerRole::ControlPlane {
        heartbeat_recent(watcher_heartbeat.as_ref())
    } else {
        true
    };
    let (mem0_enabled, mem0_url, playwright_url, public_app_base_url_configured) = {
        let agent = state.agent.read().await;
        (
            agent.config.mem0.enabled,
            agent.config.mem0.bridge_url.clone(),
            agent.config.browser.bridge_url.clone(),
            agent
                .config
                .public_apps
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some(),
        )
    };
    let health_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok();
    let mem0_ok = if state.server_role == HttpServerRole::ControlPlane && mem0_enabled {
        if let Some(client) = health_client.as_ref() {
            client
                .get(format!("{}/health", mem0_url.trim_end_matches('/')))
                .send()
                .await
                .map(|resp| resp.status().is_success())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        true
    };
    let playwright_ok = if state.server_role == HttpServerRole::ControlPlane {
        if let Some(client) = health_client.as_ref() {
            client
                .get(format!("{}/health", playwright_url.trim_end_matches('/')))
                .send()
                .await
                .map(|resp| resp.status().is_success())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        true
    };
    let whatsapp_active = state.whatsapp_bridge.read().await.active;
    let tunnel_active = state.tunnel.read().await.active;
    let public_app_origin_ok = if state.server_role == HttpServerRole::ControlPlane
        && state.deployment_mode == DeploymentMode::InternetFacing
    {
        public_app_base_url_configured
    } else {
        true
    };
    let healthy = storage_ok
        && sqlite_ok
        && mem0_ok
        && playwright_ok
        && public_app_origin_ok
        && scheduler_loop_ok
        && watcher_loop_ok;
    (
        if healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({
            "status": if healthy { "ok" } else { "degraded" },
            "server_role": match state.server_role {
                HttpServerRole::ControlPlane => "control_plane",
                HttpServerRole::PublicApps => "public_apps",
            },
            "deployment_mode": state.deployment_mode.as_str(),
            "checks": {
                "storage": storage_ok,
                "sqlite_quick_check": sqlite_quick_check.unwrap_or_else(|| "unavailable".to_string()),
                "sqlite_ok": sqlite_ok,
                "mem0_bridge": mem0_ok,
                "playwright_bridge": playwright_ok,
                "whatsapp_bridge": whatsapp_active,
                "tunnel": tunnel_active,
                "scheduler_loop": scheduler_loop_ok,
                "watcher_loop": watcher_loop_ok,
                "public_app_origin_ready": public_app_origin_ok,
            }
        })),
    )
        .into_response()
}

// - WhatsApp Webhook -

/// GET /webhook/whatsapp - Meta verification handshake
async fn whatsapp_webhook_verify(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let verify_token = {
        let agent = state.agent.read().await;
        agent
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.verify_token.clone())
    };

    let Some(token) = verify_token else {
        return (StatusCode::FORBIDDEN, "WhatsApp not configured").into_response();
    };

    match crate::channels::whatsapp::verify_webhook(&params, &token).await {
        Ok(challenge) => challenge.into_response(),
        Err(e) => {
            tracing::warn!("WhatsApp webhook verify failed: {}", e);
            (StatusCode::FORBIDDEN, format!("Verification failed: {}", e)).into_response()
        }
    }
}

/// POST /webhook/whatsapp - Inbound messages from Meta
async fn whatsapp_webhook_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let agent = state.agent.clone();
    // Spawn processing so Meta gets a fast 200 response
    tokio::spawn(async move {
        if let Err(e) = crate::channels::whatsapp::handle_webhook(agent, &body).await {
            tracing::error!("WhatsApp webhook processing error: {}", e);
        }
    });
    StatusCode::OK.into_response()
}

// - WhatsApp Bridge Proxy -

/// GET /api/whatsapp-bridge/status - proxy to Baileys bridge sidecar
async fn whatsapp_bridge_status(State(state): State<AppState>) -> Response {
    let bridge_url = {
        let agent = state.agent.read().await;
        agent
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.bridge_url.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8999".to_string())
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    match client.get(format!("{}/status", bridge_url)).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Bridge unreachable: {}", e) })),
        )
            .into_response(),
    }
}

/// POST /api/whatsapp-bridge/logout - proxy logout to Baileys bridge
async fn whatsapp_bridge_logout(State(state): State<AppState>) -> Response {
    let bridge_url = {
        let agent = state.agent.read().await;
        agent
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.bridge_url.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8999".to_string())
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    match client.post(format!("{}/logout", bridge_url)).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Bridge unreachable: {}", e) })),
        )
            .into_response(),
    }
}

/// GET /api/telegram/status - connectivity check for configured Telegram bot.
async fn telegram_channel_status(State(state): State<AppState>) -> Response {
    let (enabled, bot_token) = {
        let agent = state.agent.read().await;
        if let Some(cfg) = &agent.config.telegram {
            (true, cfg.bot_token.clone())
        } else {
            (false, String::new())
        }
    };

    if !enabled {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "disabled",
                "detail": "Telegram is disabled."
            })),
        )
            .into_response();
    }

    if bot_token.trim().is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "missing_token",
                "detail": "Telegram bot token is not configured."
            })),
        )
            .into_response();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .unwrap_or_default();

    let url = format!("https://api.telegram.org/bot{}/getMe", bot_token.trim());

    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let payload = resp
                .json::<serde_json::Value>()
                .await
                .unwrap_or_else(|_| serde_json::json!({}));
            if status.is_success() && payload.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                let result = payload
                    .get("result")
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                let username = result
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let bot_id = result
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_default();
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "connected",
                        "detail": format!("Connected as @{} ({})", username, bot_id),
                        "username": username,
                        "bot_id": bot_id
                    })),
                )
                    .into_response()
            } else {
                let desc = payload
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Telegram API returned an error.");
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "detail": desc
                    })),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "detail": format!("Telegram API unreachable: {}", e)
            })),
        )
            .into_response(),
    }
}

/// Get approval audit log (persisted in database)
async fn get_approval_log(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    match agent
        .encrypted_storage
        .get_approval_log_decrypted(limit, offset)
        .await
    {
        Ok(log) => Json(serde_json::json!({ "approvals": log, "limit": limit, "offset": offset }))
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get approval log: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Get security event log (persisted in database), with pagination and optional event type filter.
async fn get_security_logs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64)
        .clamp(1, 100);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let event_type = params
        .get("event_type")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let agent = state.agent.read().await;
    let total = match agent
        .storage
        .count_security_logs(event_type.as_deref())
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to count security logs: {}", e),
                }),
            )
                .into_response();
        }
    };

    match agent
        .storage
        .list_security_logs_paginated(limit, offset, event_type.as_deref())
        .await
    {
        Ok(logs) => Json(serde_json::json!({
            "logs": logs,
            "total": total,
            "limit": limit,
            "offset": offset,
            "event_type": event_type,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get security logs: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Spawn the WhatsApp bridge Node.js process (if not already running)
async fn spawn_whatsapp_bridge(bridge_arc: Arc<RwLock<WhatsAppBridgeState>>) -> Result<(), String> {
    {
        let bridge = bridge_arc.read().await;
        if bridge.active {
            return Ok(());
        }
    }

    // Check that node and the bridge script exist
    if !std::path::Path::new("/app/whatsapp-bridge/index.js").exists() {
        return Err("WhatsApp bridge script not found".to_string());
    }

    match tokio::process::Command::new("node")
        .arg("/app/whatsapp-bridge/index.js")
        .env("BRIDGE_PORT", "8999")
        .env("BRIDGE_HOST", "127.0.0.1")
        .env("AGENTARK_URL", "http://127.0.0.1:8990")
        .env("AUTH_DIR", "/app/data/whatsapp-auth")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            let pid = child.id();
            let mut bridge = bridge_arc.write().await;
            bridge.process = Some(child);
            bridge.active = true;
            bridge.error = None;
            tracing::info!("WhatsApp bridge started (PID: {:?})", pid);
            Ok(())
        }
        Err(e) => {
            let mut bridge = bridge_arc.write().await;
            bridge.active = false;
            bridge.error = Some(format!("Failed to spawn bridge: {}", e));
            Err(format!("Failed to start WhatsApp bridge: {}", e))
        }
    }
}

/// Stop the WhatsApp bridge process
async fn stop_whatsapp_bridge(bridge_arc: Arc<RwLock<WhatsAppBridgeState>>) {
    let mut bridge = bridge_arc.write().await;
    if let Some(ref mut child) = bridge.process {
        let _ = child.kill().await;
        tracing::info!("WhatsApp bridge stopped");
    }
    bridge.process = None;
    bridge.active = false;
    bridge.error = None;
}

/// List active watchers
async fn get_watchers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (watchers, supervisor_states) = {
        let agent = state.agent.read().await;
        (
            agent.watcher_manager.list().await,
            crate::core::list_automation_supervisor_states(&agent.storage)
                .await
                .unwrap_or_default(),
        )
    };
    let live_ids: HashSet<String> = watchers.iter().map(|w| w.id.to_string()).collect();
    let mut watcher_list: Vec<serde_json::Value> = watchers
        .iter()
        .map(|w| {
            let status_error = match &w.status {
                crate::core::watcher::WatcherStatus::Failed { error } => Some(error.clone()),
                _ => None,
            };
            serde_json::json!({
                "id": w.id.to_string(),
                "description": w.description,
                "poll_action": w.poll_action,
                "poll_arguments": w.poll_arguments,
                "condition": w.condition,
                "status": automation_watcher_status_label(&w.status),
                "status_error": status_error,
                "interval_secs": w.interval_secs,
                "timeout_secs": w.timeout_secs,
                "poll_count": w.poll_count,
                "created_at": w.created_at.to_rfc3339(),
                "last_poll_at": w.last_poll_at.map(|t| t.to_rfc3339()),
                "notify_channel": w.notify_channel,
                "on_trigger": w.on_trigger,
                "trigger_result": w.trigger_result,
                "last_result": w.last_result,
                "last_error": w.last_error,
                "last_poll_outcome": w.last_poll_outcome,
                "notification_attempts": w.notification_attempts,
                "history_only": false,
            })
        })
        .collect();
    watcher_list.extend(
        supervisor_states
            .into_iter()
            .filter(|state| {
                state.automation_kind == "watcher" && !live_ids.contains(&state.automation_id)
            })
            .map(|state| {
                let created_at = state
                    .created_at
                    .clone()
                    .or_else(|| state.last_run_at.clone())
                    .or_else(|| state.last_success_at.clone());
                let status = state.status.clone();
                let status_error = state.last_error.clone();
                let last_poll_outcome = match status.as_str() {
                    "triggered" => Some("matched"),
                    "failed" | "timed_out" => Some("error"),
                    _ => None,
                };
                serde_json::json!({
                    "id": state.automation_id,
                    "description": state.title,
                    "poll_action": state.action,
                    "poll_arguments": serde_json::Value::Null,
                    "condition": serde_json::Value::Null,
                    "status": status,
                    "status_error": status_error,
                    "interval_secs": serde_json::Value::Null,
                    "timeout_secs": serde_json::Value::Null,
                    "poll_count": serde_json::Value::Null,
                    "created_at": created_at,
                    "last_poll_at": state.last_run_at,
                    "notify_channel": serde_json::Value::Null,
                    "on_trigger": serde_json::Value::Null,
                    "trigger_result": serde_json::Value::Null,
                    "last_result": serde_json::Value::Null,
                    "last_error": state.last_error,
                    "last_poll_outcome": last_poll_outcome,
                    "notification_attempts": Vec::<serde_json::Value>::new(),
                    "history_only": true,
                })
            }),
    );
    watcher_list.sort_by(|left, right| {
        let left_created = left
            .get("created_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let right_created = right
            .get("created_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        right_created.cmp(left_created)
    });
    Json(serde_json::json!({ "watchers": watcher_list }))
}

/// Cancel a watcher
async fn cancel_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let cancelled = agent.watcher_manager.cancel(uuid).await;
        if cancelled {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                    .await;
            }
        }
        Json(serde_json::json!({ "cancelled": cancelled }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

async fn pause_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let paused = agent.watcher_manager.pause(uuid).await;
        if paused {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("paused"), None)
                    .await;
            }
        }
        Json(serde_json::json!({ "paused": paused }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

async fn resume_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let resumed = agent.watcher_manager.resume(uuid).await;
        if resumed {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("active"), None)
                    .await;
            }
        }
        Json(serde_json::json!({ "resumed": resumed }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

async fn delete_watcher(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let deleted_live = agent.watcher_manager.delete(uuid).await;
        let deleted_history = agent.clear_watcher_supervisor_state(&id).await;
        let deleted = deleted_live || deleted_history;
        Json(serde_json::json!({ "deleted": deleted }))
    } else {
        Json(serde_json::json!({ "error": "Invalid watcher ID" }))
    }
}

/// List active browser automation sessions
async fn browser_list_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let agent = state.agent.read().await;
    let sessions: Vec<_> = agent
        .browser_sessions
        .list_sessions()
        .into_iter()
        .map(|(id, task, status)| serde_json::json!({ "id": id, "task": task, "status": status }))
        .collect();
    Json(serde_json::json!({ "sessions": sessions }))
}

/// Provide user response to a waiting browser session
async fn browser_respond(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let agent = state.agent.read().await;
    let response = body.get("response").and_then(|v| v.as_str()).unwrap_or("");
    if response.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Response text is required".to_string(),
            }),
        )
            .into_response();
    }
    let success = agent.browser_sessions.provide_user_response(&id, response);
    if success {
        (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "success": true })),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Browser session not found or not waiting for input".to_string(),
            }),
        )
            .into_response()
    }
}

/// Get browser session status
async fn browser_session_status(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.browser_sessions.get_status(&id) {
        Some(status) => {
            let status_str = match &status {
                crate::core::browser_session::SessionStatus::Active => "active".to_string(),
                crate::core::browser_session::SessionStatus::WaitingForUser {
                    question, ..
                } => {
                    format!("waiting_for_user: {}", question)
                }
                crate::core::browser_session::SessionStatus::Completed { summary } => {
                    format!("completed: {}", summary)
                }
                crate::core::browser_session::SessionStatus::Failed(e) => format!("failed: {}", e),
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({ "id": id, "status": status_str })),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Session not found".to_string(),
            }),
        )
            .into_response(),
    }
}

/// Get agent status
async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let agent = state.agent.read().await;
    let status = agent.status().await;

    Json(StatusResponse {
        did: status.did,
        memory_entries: status.memory_entries,
        skills_loaded: status.actions_loaded,
        actions_loaded: Some(status.actions_loaded),
        tasks_pending: status.tasks_pending,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// ==================== ArkPulse Log ====================

#[derive(Debug, Deserialize)]
struct RunArkPulseFixRequest {
    #[serde(default)]
    fix_command: String,
    #[serde(default)]
    remediation: Option<crate::sentinel::DoctorRemediationSpec>,
    #[serde(default)]
    issue_title: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    event_timestamp: Option<String>,
    #[serde(default)]
    finding_index: Option<usize>,
}

enum ArkPulseFixPlan {
    TunnelStartVerify,
    TunnelRestartVerify,
    AppRestart(String),
    ShellCommand(String),
}

fn arkpulse_fix_plan_from_remediation(
    remediation: &crate::sentinel::DoctorRemediationSpec,
    allow_shell_command: bool,
) -> Option<ArkPulseFixPlan> {
    match remediation {
        crate::sentinel::DoctorRemediationSpec::TunnelStartVerify => {
            Some(ArkPulseFixPlan::TunnelStartVerify)
        }
        crate::sentinel::DoctorRemediationSpec::TunnelRestartVerify => {
            Some(ArkPulseFixPlan::TunnelRestartVerify)
        }
        crate::sentinel::DoctorRemediationSpec::AppRestart { app_id } => {
            if is_valid_app_id(app_id) {
                Some(ArkPulseFixPlan::AppRestart(app_id.clone()))
            } else {
                None
            }
        }
        crate::sentinel::DoctorRemediationSpec::ShellCommand { command } if allow_shell_command => {
            let normalized = command.trim();
            if normalized.is_empty() {
                None
            } else {
                Some(ArkPulseFixPlan::ShellCommand(normalized.to_string()))
            }
        }
        crate::sentinel::DoctorRemediationSpec::ShellCommand { .. } => None,
    }
}

fn parse_arkpulse_app_restart(command: &str) -> Option<String> {
    let normalized = command.trim();
    let path = normalized
        .strip_prefix("POST ")
        .or_else(|| normalized.strip_prefix("post "))?
        .trim();
    let app_id = path.strip_prefix("/api/apps/")?.strip_suffix("/restart")?;
    if is_valid_app_id(app_id) {
        Some(app_id.to_string())
    } else {
        None
    }
}

fn is_supported_arkpulse_shell_segment(segment: &str) -> bool {
    let lower = segment.trim().to_ascii_lowercase();
    lower.starts_with("pip-compile requirements.txt")
        || lower.starts_with("rg -n ")
        || lower == "cargo generate-lockfile"
        || lower.starts_with("npm pkg delete ")
        || lower.starts_with("mv .env ")
}

fn parse_supported_arkpulse_shell_command(command: &str) -> Option<String> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if lower.contains('\n')
        || lower.contains('\r')
        || lower.contains("||")
        || lower.contains(';')
        || lower.contains('`')
        || lower.contains("$(")
    {
        return None;
    }

    let segments: Vec<&str> = normalized
        .split("&&")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 {
        return None;
    }

    let cd_segment = segments[0];
    let cd_prefix = cd_segment
        .strip_prefix("cd ")
        .or_else(|| cd_segment.strip_prefix("cd\t"))?;
    let cd_target = cd_prefix.trim();
    if !cd_target.starts_with("/app/data/apps/") {
        return None;
    }

    if segments
        .iter()
        .skip(1)
        .all(|segment| is_supported_arkpulse_shell_segment(segment))
    {
        Some(normalized.to_string())
    } else {
        None
    }
}

fn classify_arkpulse_fix_plan(command: &str) -> Option<ArkPulseFixPlan> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("start tunnel") && lower.contains("/tunnel/status") {
        return Some(ArkPulseFixPlan::TunnelStartVerify);
    }
    if lower.contains("restart") && lower.contains("tunnel") {
        return Some(ArkPulseFixPlan::TunnelRestartVerify);
    }
    if let Some(app_id) = parse_arkpulse_app_restart(normalized) {
        return Some(ArkPulseFixPlan::AppRestart(app_id));
    }
    parse_supported_arkpulse_shell_command(normalized).map(ArkPulseFixPlan::ShellCommand)
}

fn truncate_for_response(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>() + "..."
}

fn describe_arkpulse_remediation(
    remediation: Option<&crate::sentinel::DoctorRemediationSpec>,
    fix_command: &str,
) -> String {
    let normalized = fix_command.trim();
    if !normalized.is_empty() {
        return normalized.to_string();
    }
    match remediation {
        Some(crate::sentinel::DoctorRemediationSpec::TunnelStartVerify) => {
            "Start tunnel and verify /tunnel/status returns active + URL".to_string()
        }
        Some(crate::sentinel::DoctorRemediationSpec::TunnelRestartVerify) => {
            "Restart tunnel and verify public reachability".to_string()
        }
        Some(crate::sentinel::DoctorRemediationSpec::AppRestart { app_id }) => {
            format!("Restart app {} and re-check health", app_id)
        }
        Some(crate::sentinel::DoctorRemediationSpec::ShellCommand { command }) => {
            command.trim().to_string()
        }
        None => String::new(),
    }
}

async fn run_arkpulse_app_restart_fix(state: &AppState, app_id: &str) -> Response {
    let response = restart_app(State(state.clone()), Path(app_id.to_string())).await;
    let status = response.status();
    let body = response.into_body();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to read app restart response: {}", error),
                }),
            )
                .into_response();
        }
    };
    let payload = serde_json::from_slice::<serde_json::Value>(&body_bytes).unwrap_or_else(|_| {
        serde_json::json!({
            "raw": String::from_utf8_lossy(&body_bytes).to_string()
        })
    });

    if !status.is_success() {
        let error = payload
            .get("error")
            .and_then(|value| value.as_str())
            .or_else(|| payload.get("message").and_then(|value| value.as_str()))
            .unwrap_or("Failed to restart app");
        return (
            status,
            Json(serde_json::json!({
                "status": "error",
                "mode": "app_restart",
                "app_id": app_id,
                "error": error,
                "details": payload,
            })),
        )
            .into_response();
    }

    let title = payload
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or(app_id);
    let url = payload
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "mode": "app_restart",
            "app_id": app_id,
            "message": format!("Restarted app {} and queued a fresh ArkPulse run.", title),
            "url": url,
            "details": payload,
        })),
    )
        .into_response()
}

/// Execute a supported ArkPulse remediation directly (without going through Chat).
async fn run_arkpulse_fix(
    State(state): State<AppState>,
    Json(request): Json<RunArkPulseFixRequest>,
) -> Response {
    let RunArkPulseFixRequest {
        fix_command,
        remediation,
        issue_title,
        target,
        event_timestamp,
        finding_index,
    } = request;

    let request_fix_command = fix_command.trim().to_string();
    if request_fix_command.is_empty() && remediation.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "remediation or fix_command is required".to_string(),
            }),
        )
            .into_response();
    }

    let mut effective_fix_command = request_fix_command.clone();
    let mut effective_remediation = remediation.clone();

    let plan = if event_timestamp.is_some() || finding_index.is_some() {
        let event_timestamp = event_timestamp
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(event_timestamp) = event_timestamp else {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "event_timestamp is required when finding_index is provided".to_string(),
                }),
            )
                .into_response();
        };
        let Some(finding_index) = finding_index else {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "finding_index is required when event_timestamp is provided".to_string(),
                }),
            )
                .into_response();
        };

        let agent = state.agent.read().await;
        let events = crate::sentinel::get_pulse_log(&agent).await;
        let Some(event) = events
            .iter()
            .find(|event| event.timestamp == event_timestamp)
        else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "ArkPulse event not found".to_string(),
                }),
            )
                .into_response();
        };
        let Some(finding) = event.details.doctor_findings.get(finding_index) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "ArkPulse finding index is out of range".to_string(),
                }),
            )
                .into_response();
        };
        if !finding.user_actionable {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "This ArkPulse finding is advisory-only and must be fixed manually"
                        .to_string(),
                }),
            )
                .into_response();
        }
        if !request_fix_command.is_empty() && finding.fix_command.trim() != request_fix_command {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "fix_command does not match the selected ArkPulse finding".to_string(),
                }),
            )
                .into_response();
        }
        if let (Some(requested_remediation), Some(stored_remediation)) =
            (remediation.as_ref(), finding.remediation.as_ref())
        {
            if stored_remediation != requested_remediation {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "remediation does not match the selected ArkPulse finding"
                            .to_string(),
                    }),
                )
                    .into_response();
            }
        }
        effective_fix_command = finding.fix_command.trim().to_string();
        effective_remediation = finding.remediation.clone();
        effective_remediation
            .as_ref()
            .and_then(|value| arkpulse_fix_plan_from_remediation(value, true))
            .or_else(|| classify_arkpulse_fix_plan(&effective_fix_command))
    } else {
        effective_remediation
            .as_ref()
            .and_then(|value| arkpulse_fix_plan_from_remediation(value, false))
            .or_else(|| classify_arkpulse_fix_plan(&request_fix_command))
    };

    let Some(plan) = plan else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error:
                    "This fix cannot be auto-run directly. Copy the remediation and run it manually."
                        .to_string(),
            }),
        )
            .into_response();
    };

    let issue_title = issue_title.unwrap_or_default();
    let target = target.unwrap_or_default();
    let fix_summary =
        describe_arkpulse_remediation(effective_remediation.as_ref(), &effective_fix_command);
    tracing::info!(
        "ArkPulse fix requested: issue='{}' target='{}' command='{}'",
        issue_title,
        target,
        truncate_for_response(&fix_summary, 220)
    );

    match plan {
        ArkPulseFixPlan::TunnelStartVerify => {
            let tunnel_arc = state.tunnel.clone();
            if let Err(error) = tunnel::spawn_tunnel(&state, None).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error }),
                )
                    .into_response();
            }

            let discovered_url = tunnel::wait_for_tunnel_url(tunnel_arc.clone(), 12).await;
            if let Some(url) = discovered_url.as_ref() {
                tunnel::persist_public_tunnel_state(&state, Some(url), None).await;
            }

            let tunnel = tunnel_arc.read().await;
            let active = tunnel.active;
            let url = tunnel.url.clone();
            let message = if active && url.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                format!(
                    "Tunnel is active and publicly reachable at {}.",
                    url.clone().unwrap_or_default()
                )
            } else {
                "Tunnel start requested. URL is pending; re-check /tunnel/status shortly."
                    .to_string()
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "mode": "tunnel_start_verify",
                    "message": message,
                    "active": active,
                    "url": url
                })),
            )
                .into_response()
        }
        ArkPulseFixPlan::TunnelRestartVerify => {
            tunnel::stop_tunnel_internal(&state).await;
            let tunnel_arc = state.tunnel.clone();
            if let Err(error) = tunnel::spawn_tunnel(&state, None).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error }),
                )
                    .into_response();
            }

            let discovered_url = tunnel::wait_for_tunnel_url(tunnel_arc.clone(), 12).await;
            if let Some(url) = discovered_url.as_ref() {
                tunnel::persist_public_tunnel_state(&state, Some(url), None).await;
            }

            let tunnel = tunnel_arc.read().await;
            let active = tunnel.active;
            let url = tunnel.url.clone();
            let message = if active && url.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                format!(
                    "Tunnel restarted successfully and is reachable at {}.",
                    url.clone().unwrap_or_default()
                )
            } else {
                "Tunnel restart requested. URL is pending; re-check /tunnel/status shortly."
                    .to_string()
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "mode": "tunnel_restart_verify",
                    "message": message,
                    "active": active,
                    "url": url
                })),
            )
                .into_response()
        }
        ArkPulseFixPlan::AppRestart(app_id) => run_arkpulse_app_restart_fix(&state, &app_id).await,
        ArkPulseFixPlan::ShellCommand(command) => {
            let started_at = Instant::now();
            let mut process = if cfg!(windows) {
                let mut cmd = tokio::process::Command::new("cmd");
                cmd.arg("/C").arg(&command);
                cmd
            } else {
                let mut cmd = tokio::process::Command::new("sh");
                cmd.arg("-lc").arg(&command);
                cmd
            };
            let exec = process
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .output();

            let output = match tokio::time::timeout(Duration::from_secs(120), exec).await {
                Ok(Ok(output)) => output,
                Ok(Err(error)) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to run fix command: {}", error),
                        }),
                    )
                        .into_response();
                }
                Err(_) => {
                    return (
                        StatusCode::REQUEST_TIMEOUT,
                        Json(ErrorResponse {
                            error: "Fix command timed out after 120s".to_string(),
                        }),
                    )
                        .into_response();
                }
            };

            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let combined = if stdout.is_empty() && stderr.is_empty() {
                "(no output)".to_string()
            } else if stderr.is_empty() {
                stdout.clone()
            } else if stdout.is_empty() {
                stderr.clone()
            } else {
                format!("{}\n\nstderr:\n{}", stdout, stderr)
            };
            let output_preview = truncate_for_response(&combined, 4000);

            if output.status.success() {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "mode": "shell",
                        "message": "ArkPulse fix command executed successfully.",
                        "command": command,
                        "duration_ms": elapsed_ms,
                        "output": output_preview
                    })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "status": "error",
                        "mode": "shell",
                        "error": format!(
                            "Fix command failed with exit code {}",
                            output.status.code().unwrap_or(-1)
                        ),
                        "command": command,
                        "duration_ms": elapsed_ms,
                        "output": output_preview
                    })),
                )
                    .into_response()
            }
        }
    }
}

/// Return the ArkPulse event log (last 100 events)
async fn get_pulse_log(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let agent = state.agent.read().await;
    let mut all_events = crate::sentinel::get_pulse_log(&agent).await;
    all_events.sort_by(|a, b| {
        let a_ts = chrono::DateTime::parse_from_rfc3339(&a.timestamp)
            .map(|ts| ts.timestamp_millis())
            .unwrap_or(0);
        let b_ts = chrono::DateTime::parse_from_rfc3339(&b.timestamp)
            .map(|ts| ts.timestamp_millis())
            .unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    let total = all_events.len();
    let events: Vec<_> = all_events.into_iter().skip(offset).take(limit).collect();
    Json(serde_json::json!({
        "events": events,
        "total": total,
        "limit": limit,
        "offset": offset,
        "running": crate::sentinel::is_pulse_running()
    }))
}

async fn trigger_arkpulse_after_app_change(state: &AppState, reason: &'static str) {
    if crate::sentinel::is_pulse_running() {
        return;
    }
    {
        let agent_guard = state.agent.read().await;
        let autonomy = load_autonomy_settings(&agent_guard).await;
        if autonomy.agent_paused {
            return;
        }
    }
    let agent = state.agent.clone();
    tokio::spawn(async move {
        tracing::info!("ArkPulse auto-triggered after {}", reason);
        crate::sentinel::run_pulse(&agent).await;
    });
}

/// Trigger an ArkPulse check immediately
async fn trigger_pulse(State(state): State<AppState>) -> Json<serde_json::Value> {
    if crate::sentinel::is_pulse_running() {
        return Json(serde_json::json!({
            "status": "running",
            "message": "ArkPulse is already running"
        }));
    }
    {
        let agent_guard = state.agent.read().await;
        let autonomy = load_autonomy_settings(&agent_guard).await;
        if autonomy.agent_paused {
            return Json(serde_json::json!({
                "status": "paused",
                "message": "Agent is paused. Resume the agent to run ArkPulse."
            }));
        }
    }
    let agent = state.agent.clone();
    tokio::spawn(async move {
        crate::sentinel::run_pulse(&agent).await;
    });
    Json(serde_json::json!({ "status": "triggered", "message": "ArkPulse check started" }))
}

/// Get user profile (for checking onboarding status)
async fn get_profile(State(state): State<AppState>) -> Json<ProfileResponse> {
    let profile = state.user_profile.read().await;
    Json(ProfileResponse {
        name: profile.name.clone(),
        location: profile.location.clone(),
        timezone: profile.timezone.clone(),
        language: profile.language.clone(),
        tone: profile.tone.clone(),
        email_format: profile.email_format.clone(),
        preferences: profile.preferences.clone(),
        onboarding_complete: profile.onboarding_complete,
    })
}

/// Chat with the agent
async fn chat(
    State(state): State<AppState>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ChatRequest>,
) -> Response {
    tracing::info!(
        "HTTP /chat request: channel={}, msg={}chars, conv_id={:?}, project={:?}",
        request.channel,
        request.message.len(),
        request.conversation_id.as_deref().unwrap_or("-"),
        request.project_id.as_deref().unwrap_or("-"),
    );

    // Two-tier secrets UX: allow "set secret KEY=VALUE" without engaging the LLM.
    // This bypasses conversation history + traces, and stores the value encrypted in secrets.enc.
    if let Some((key, value)) = parse_set_secret_command(&request.message) {
        let cid = request.conversation_id.clone();
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        if let Err(e) =
            crate::core::secrets::store_user_secret(&config_dir, Some(&data_dir), &key, &value)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to store secret: {}", e),
                }),
            )
                .into_response();
        }

        let followup = if let Some(ref cid_str) = cid {
            let agent = state.agent.read().await;
            agent.on_secret_saved_followup(cid_str).await
        } else {
            None
        };

        let mut response = format!(
            "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
            key
        );
        if let Some(f) = followup {
            response.push_str("\n\n");
            response.push_str(&f);
        }

        return (
            StatusCode::OK,
            Json(ChatResponse {
                response,
                proof_id: None,
                conversation_id: cid,
                conversation_title: None,
            }),
        )
            .into_response();
    }

    // Human-in-the-loop shortcut: reuse currently configured model key without sending to the LLM.
    if let Some(key) = crate::core::secrets::parse_use_current_llm_key_command(&request.message) {
        let cid = request.conversation_id.clone();
        let (config_dir, data_dir, llm_env) = {
            let agent = state.agent.read().await;
            (
                agent.config_dir.clone(),
                agent.data_dir.clone(),
                agent.app_model_env_vars(),
            )
        };
        let Some(value) = llm_env.get(&key).cloned().filter(|v| !v.trim().is_empty()) else {
            let mut available: Vec<String> = llm_env
                .iter()
                .filter_map(|(k, v)| {
                    if v.trim().is_empty() {
                        None
                    } else if k.ends_with("_API_KEY")
                        || k.ends_with("_BASE_URL")
                        || k == "LLM_MODEL"
                        || k == "LLM_PROVIDER"
                    {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect();
            available.sort();
            let available_text = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Can't map '{}' from current model settings. Available model-backed keys: {}",
                        key, available_text
                    ),
                }),
            )
                .into_response();
        };

        if let Err(e) =
            crate::core::secrets::store_user_secret(&config_dir, Some(&data_dir), &key, &value)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to store secret: {}", e),
                }),
            )
                .into_response();
        }

        let followup = if let Some(ref cid_str) = cid {
            let agent = state.agent.read().await;
            agent.on_secret_saved_followup(cid_str).await
        } else {
            None
        };

        let mut response = format!(
            "Linked '{}' to the currently configured model credential (stored encrypted). This was not sent to the LLM.",
            key
        );
        if let Some(f) = followup {
            response.push_str("\n\n");
            response.push_str(&f);
        }

        return (
            StatusCode::OK,
            Json(ChatResponse {
                response,
                proof_id: None,
                conversation_id: cid,
                conversation_title: None,
            }),
        )
            .into_response();
    }

    // Fast command path: push-notification controls without LLM roundtrip.
    if let Some(cmd) = parse_notification_control_command(&request.message) {
        match handle_notification_control_command(&state, cmd).await {
            Ok(response) => {
                return (
                    StatusCode::OK,
                    Json(ChatResponse {
                        response,
                        proof_id: None,
                        conversation_id: request.conversation_id.clone(),
                        conversation_title: None,
                    }),
                )
                    .into_response();
            }
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    // Fast command path: tunnel control without LLM roundtrip.
    if let Some(cmd) = tunnel::parse_tunnel_command(&request.message) {
        match tunnel::handle_tunnel_control_command(&state, cmd).await {
            Ok(response) => {
                return (
                    StatusCode::OK,
                    Json(ChatResponse {
                        response,
                        proof_id: None,
                        conversation_id: request.conversation_id.clone(),
                        conversation_title: None,
                    }),
                )
                    .into_response();
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error }),
                )
                    .into_response();
            }
        }
    }

    // Fast command path: explicit autonomy helpers without LLM roundtrip.
    if let Some(cmd) = parse_autonomy_quick_command(&request.message) {
        match handle_autonomy_quick_command(&state, cmd).await {
            Ok(response) => {
                return (
                    StatusCode::OK,
                    Json(ChatResponse {
                        response,
                        proof_id: None,
                        conversation_id: request.conversation_id.clone(),
                        conversation_title: None,
                    }),
                )
                    .into_response();
            }
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    let result = {
        let agent_guard = state.agent.read().await;
        agent_guard
            .process_message_with_meta_and_hints(
                &request.message,
                &request.channel,
                request.conversation_id.as_deref(),
                request.project_id.as_deref(),
                crate::core::RequestExecutionHints {
                    deep_research: request.deep_research,
                },
            )
            .await
    };

    match result {
        Ok(processed) => {
            spawn_autonomy_analysis_tick(state.agent.clone(), "chat_event");
            (
                StatusCode::OK,
                Json(ChatResponse {
                    response: processed.response,
                    proof_id: None,
                    conversation_id: processed.conversation_id.or(request.conversation_id),
                    conversation_title: processed.conversation_title,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Chat with the agent via SSE - streams thinking steps in real-time
fn stream_detail_looks_like_html_payload(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || (lower.contains("<html") && (lower.contains("</html>") || lower.contains("</body>")))
}

fn stream_detail_looks_like_source_payload(text: &str) -> bool {
    let sample = text.trim().lines().take(12).collect::<Vec<_>>().join("\n");
    if sample.is_empty() {
        return false;
    }
    let lower = sample.to_ascii_lowercase();
    lower.contains("from fastapi import")
        || lower.contains("import asyncio")
        || lower.contains("import httpx")
        || lower.contains("function ")
        || lower.contains("const ")
        || lower.contains("let ")
        || lower.contains("class ")
        || lower.contains("def ")
        || lower.contains("async def ")
        || lower.contains("#include ")
}

fn summarize_stream_tool_activity_content(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if stream_detail_looks_like_html_payload(trimmed) {
        if let Some(start) = trimmed.to_ascii_lowercase().find("<title>") {
            let rest = &trimmed[start + "<title>".len()..];
            if let Some(end) = rest.to_ascii_lowercase().find("</title>") {
                let title = rest[..end].trim();
                if !title.is_empty() {
                    return format!("Read HTML document: {}.", title);
                }
            }
        }
        return "Read HTML document.".to_string();
    }

    let json_like = (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if json_like {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(obj) = value.as_object() {
                if let Some(title) = obj
                    .get("matched_app")
                    .and_then(|v| v.get("title"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    return format!("Matched app and loaded metadata for {}.", title);
                }
                let keys = obj.keys().take(4).cloned().collect::<Vec<_>>().join(", ");
                if !keys.is_empty() {
                    return format!("Returned structured data: {}.", keys);
                }
            } else if let Some(items) = value.as_array() {
                return format!(
                    "Returned list with {} item{}.",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" }
                );
            }
        }
    }

    if stream_detail_looks_like_source_payload(trimmed) {
        let line_count = trimmed.lines().count();
        return format!(
            "Read source file contents ({} line{}).",
            line_count,
            if line_count == 1 { "" } else { "s" }
        );
    }

    if trimmed.len() > 240
        && trimmed
            .chars()
            .any(|ch| matches!(ch, '{' | '}' | '<' | '>' | ';'))
    {
        return "Returned verbose tool output.".to_string();
    }

    trimmed.chars().take(240).collect::<String>()
}

fn normalize_stream_heartbeat_status(status: &str) -> String {
    let trimmed = status.trim();
    if trimmed.is_empty() {
        return "Still processing. No new output yet.".to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    let memory_is_active = (lower.contains("memory") || lower.contains("mem0"))
        && !lower.contains("available on demand");
    if memory_is_active {
        return "Memory/context setup in progress. No new output yet.".to_string();
    }
    if lower.contains("context") {
        return "Preparing conversation context. No new output yet.".to_string();
    }
    if lower.contains("tool") {
        return "Waiting on tool execution. No new output yet.".to_string();
    }
    if lower.contains("respond") || lower.contains("generating") || lower.contains("model") {
        return "Waiting on model response. No new output yet.".to_string();
    }
    "Still processing. No new output yet.".to_string()
}

fn normalize_stream_event_for_sse(
    ev: crate::core::StreamEvent,
    last_thinking_detail: &str,
) -> (Option<(&'static str, serde_json::Value)>, String) {
    match ev {
        crate::core::StreamEvent::Token(content) => (
            Some(("token", serde_json::json!({ "content": content }))),
            String::new(),
        ),
        crate::core::StreamEvent::Thinking(status) => {
            let detail = normalize_stream_heartbeat_status(&status);
            if detail == last_thinking_detail {
                (None, detail)
            } else {
                (
                    Some((
                        "thinking",
                        serde_json::json!({
                            "step_type": "heartbeat",
                            "title": "Still Working",
                            "detail": detail
                        }),
                    )),
                    detail,
                )
            }
        }
        crate::core::StreamEvent::ToolStart { name, payload } => {
            let payload_json = if let Some(payload) = payload {
                if let Some(obj) = payload.as_object() {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name));
                    for (k, v) in obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    serde_json::Value::Object(merged)
                } else {
                    serde_json::json!({ "name": name, "payload": payload })
                }
            } else {
                serde_json::json!({ "name": name })
            };
            (Some(("tool_start", payload_json)), String::new())
        }
        crate::core::StreamEvent::ToolResult { name, content } => {
            let content = summarize_stream_tool_activity_content(&content);
            (
                Some((
                    "tool_result",
                    serde_json::json!({ "name": name, "content": content }),
                )),
                String::new(),
            )
        }
        crate::core::StreamEvent::ToolProgress {
            name,
            content,
            payload,
        } => {
            let content = summarize_stream_tool_activity_content(&content);
            let payload_json = if let Some(payload) = payload {
                if let Some(obj) = payload.as_object() {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name));
                    merged.insert("content".to_string(), serde_json::json!(content));
                    for (k, v) in obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    serde_json::Value::Object(merged)
                } else {
                    serde_json::json!({ "name": name, "content": content, "payload": payload })
                }
            } else {
                serde_json::json!({ "name": name, "content": content })
            };
            (Some(("tool_progress", payload_json)), String::new())
        }
    }
}

fn truncate_stream_task_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn normalized_chat_execution_mode(mode: Option<&str>) -> &'static str {
    match mode.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "chat" | "ask" => "chat",
        "task" | "do" | "agent" => "task",
        _ => "auto",
    }
}

fn chat_message_contains_any(lower: &str, tokens: &[&str]) -> bool {
    tokens.iter().any(|token| lower.contains(token))
}

fn chat_message_contains_import_source(lower: &str) -> bool {
    let source_like = [
        "clawhub.ai/",
        "openclaw.ai/",
        "github.com/",
        "raw.githubusercontent.com/",
        "/skills/",
        "skill.md",
        "action.md",
    ];
    chat_message_contains_any(lower, &source_like)
}

fn chat_message_looks_like_direct_import_source(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.is_empty() || !chat_message_contains_import_source(trimmed) {
        return false;
    }
    trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("www.")
        || trimmed.split_whitespace().count() <= 6
}

fn chat_message_requests_app_work(lower: &str) -> bool {
    let build_like = [
        "build",
        "create",
        "make",
        "ship",
        "deploy",
        "fix",
        "debug",
        "repair",
        "prototype",
        "scaffold",
        "spin up",
        "stand up",
        "generate",
        "turn into",
        "convert",
    ];
    let app_like = [
        "app",
        "web app",
        "site",
        "website",
        "dashboard",
        "landing page",
        "portal",
        "interface",
        "admin panel",
        "console",
        "ui",
        "frontend",
    ];
    let explicit_like = [
        "build me an app",
        "build an app",
        "create an app",
        "make an app",
        "turn this into an app",
        "build a dashboard",
        "build a landing page",
    ];
    chat_message_contains_any(lower, &explicit_like)
        || (chat_message_contains_any(lower, &build_like)
            && chat_message_contains_any(lower, &app_like))
}

fn chat_message_requests_import(lower: &str) -> bool {
    let import_like = [
        "import", "install", "add", "pull in", "ingest", "load", "set up", "setup", "bring in",
        "use this", "try this",
    ];
    let target_like = [
        "skill",
        "skills",
        "repo",
        "repository",
        "url",
        "template",
        "document",
        "doc",
        "github",
        "clawhub",
        "openclaw",
        "skill.md",
        "action.md",
    ];
    let mentions_source = chat_message_contains_import_source(lower);
    if mentions_source
        && (chat_message_looks_like_direct_import_source(lower)
            || chat_message_contains_any(lower, &import_like)
            || lower.contains("skill"))
    {
        return true;
    }
    chat_message_contains_any(lower, &import_like)
        && (chat_message_contains_any(lower, &target_like) || mentions_source)
}

fn chat_message_requests_automation(lower: &str) -> bool {
    let automation_like = [
        "every day",
        "every week",
        "hourly",
        "daily",
        "weekly",
        "schedule",
        "cron",
        "remind",
        "monitor",
        "watch",
        "watcher",
        "automation",
    ];
    automation_like.iter().any(|token| lower.contains(token))
}

fn chat_message_requests_workspace_changes(lower: &str) -> bool {
    let change_like = [
        "fix",
        "debug",
        "edit",
        "change",
        "update",
        "rewrite",
        "refactor",
        "implement",
        "write files",
        "modify",
    ];
    let workspace_like = [
        "file",
        "files",
        "code",
        "repo",
        "repository",
        "project",
        "workspace",
        "app",
    ];
    change_like.iter().any(|token| lower.contains(token))
        && workspace_like.iter().any(|token| lower.contains(token))
}

fn chat_request_should_create_task(
    execution_mode: Option<&str>,
    message: &str,
    deep_research: bool,
    attachments_present: bool,
) -> bool {
    match normalized_chat_execution_mode(execution_mode) {
        "chat" => false,
        "task" => true,
        _ => {
            if deep_research || attachments_present {
                return true;
            }
            let lower = message.trim().to_ascii_lowercase();
            chat_message_requests_app_work(&lower)
                || chat_message_requests_import(&lower)
                || chat_message_requests_automation(&lower)
                || chat_message_requests_workspace_changes(&lower)
        }
    }
}

fn classify_chat_task_work_type(
    message: &str,
    deep_research: bool,
    attachments_present: bool,
) -> &'static str {
    if deep_research {
        return "research";
    }
    let lower = message.trim().to_ascii_lowercase();
    if chat_message_requests_app_work(&lower) {
        "app"
    } else if chat_message_requests_import(&lower) {
        "import"
    } else if chat_message_requests_automation(&lower) {
        "automation"
    } else if attachments_present {
        "workspace"
    } else if chat_message_requests_workspace_changes(&lower) {
        "workspace"
    } else {
        "task"
    }
}

fn build_chat_task_description(message: &str, work_type: &str) -> String {
    let trimmed = truncate_stream_task_text(message, 140);
    if trimmed.is_empty() {
        return "Run agent task".to_string();
    }
    let lower = message.trim().to_ascii_lowercase();
    let prefix = match work_type {
        "app" => {
            if chat_message_contains_any(&lower, &["fix", "debug", "repair", "update", "improve"]) {
                "App task"
            } else {
                "Build app"
            }
        }
        "import" => "Import",
        "automation" => "Automation",
        "workspace" => "Workspace task",
        "research" => "Research",
        _ => "Task",
    };
    format!("{}: {}", prefix, trimmed)
}

fn chat_task_status_key(status: &crate::core::TaskStatus) -> &'static str {
    match status {
        crate::core::TaskStatus::Pending => "pending",
        crate::core::TaskStatus::AwaitingApproval => "awaiting_approval",
        crate::core::TaskStatus::Paused => "paused",
        crate::core::TaskStatus::InProgress => "in_progress",
        crate::core::TaskStatus::Completed => "completed",
        crate::core::TaskStatus::Failed { .. } => "failed",
        crate::core::TaskStatus::Cancelled => "cancelled",
    }
}

fn chat_task_terminal_status(response: &str) -> crate::core::TaskStatus {
    let lower = response.trim().to_ascii_lowercase();
    if lower.contains("waiting for your approval")
        || lower.contains("waiting for your input")
        || lower.contains("reply with approval")
        || lower.contains("needs your approval")
        || lower.contains("requires approval")
        || lower.contains("api key")
    {
        crate::core::TaskStatus::Paused
    } else {
        crate::core::TaskStatus::Completed
    }
}

async fn chat_stream(
    State(state): State<AppState>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ChatRequest>,
) -> Response {
    tracing::info!(
        "HTTP /chat/stream request: channel={}, msg={}chars, conv_id={:?}",
        request.channel,
        request.message.len(),
        request.conversation_id.as_deref().unwrap_or("-"),
    );

    // Two-tier secrets UX: allow "set secret KEY=VALUE" without engaging the LLM.
    if let Some((key, value)) = parse_set_secret_command(&request.message) {
        let cid = request.conversation_id.clone();
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        let stored =
            crate::core::secrets::store_user_secret(&config_dir, Some(&data_dir), &key, &value);
        let followup = if stored.is_ok() {
            if let Some(ref cid_str) = cid {
                let agent = state.agent.read().await;
                agent.on_secret_saved_followup(cid_str).await
            } else {
                None
            }
        } else {
            None
        };
        let mut content = format!(
            "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
            key
        );
        if let Some(f) = followup {
            content.push_str("\n\n");
            content.push_str(&f);
        }
        let payload = match stored {
            Ok(_) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(e) => serde_json::json!({ "error": format!("Failed to store secret: {}", e) }),
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(4);
        tokio::spawn(async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Human-in-the-loop shortcut: reuse currently configured model key without sending to the LLM.
    if let Some(key) = crate::core::secrets::parse_use_current_llm_key_command(&request.message) {
        let cid = request.conversation_id.clone();
        let (config_dir, data_dir, llm_env) = {
            let agent = state.agent.read().await;
            (
                agent.config_dir.clone(),
                agent.data_dir.clone(),
                agent.app_model_env_vars(),
            )
        };
        let payload = if let Some(value) =
            llm_env.get(&key).cloned().filter(|v| !v.trim().is_empty())
        {
            match crate::core::secrets::store_user_secret(
                &config_dir,
                Some(&data_dir),
                &key,
                &value,
            ) {
                Ok(_) => {
                    let followup = if let Some(ref cid_str) = cid {
                        let agent = state.agent.read().await;
                        agent.on_secret_saved_followup(cid_str).await
                    } else {
                        None
                    };
                    let mut content = format!(
                        "Linked '{}' to the currently configured model credential (stored encrypted). This was not sent to the LLM.",
                        key
                    );
                    if let Some(f) = followup {
                        content.push_str("\n\n");
                        content.push_str(&f);
                    }
                    serde_json::json!({
                        "content": content,
                        "conversation_id": cid,
                    })
                }
                Err(e) => serde_json::json!({ "error": format!("Failed to store secret: {}", e) }),
            }
        } else {
            let mut available: Vec<String> = llm_env
                .iter()
                .filter_map(|(k, v)| {
                    if v.trim().is_empty() {
                        None
                    } else if k.ends_with("_API_KEY")
                        || k.ends_with("_BASE_URL")
                        || k == "LLM_MODEL"
                        || k == "LLM_PROVIDER"
                    {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect();
            available.sort();
            let available_text = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            serde_json::json!({
                "error": format!(
                    "Can't map '{}' from current model settings. Available model-backed keys: {}",
                    key, available_text
                )
            })
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(4);
        tokio::spawn(async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Fast command path: push-notification controls without LLM roundtrip.
    if let Some(cmd) = parse_notification_control_command(&request.message) {
        let cid = request.conversation_id.clone();
        let payload = match handle_notification_control_command(&state, cmd).await {
            Ok(content) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(4);
        tokio::spawn(async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Fast command path: tunnel control without LLM roundtrip.
    if let Some(cmd) = tunnel::parse_tunnel_command(&request.message) {
        let cid = request.conversation_id.clone();
        let payload = match tunnel::handle_tunnel_control_command(&state, cmd).await {
            Ok(content) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(4);
        tokio::spawn(async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Fast command path: explicit autonomy helpers without LLM roundtrip.
    if let Some(cmd) = parse_autonomy_quick_command(&request.message) {
        let cid = request.conversation_id.clone();
        let payload = match handle_autonomy_quick_command(&state, cmd).await {
            Ok(content) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(4);
        tokio::spawn(async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);
    // Per-request trace so concurrent requests cannot clobber each other.
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace::default()));
    let agent_ref = state.agent.clone();
    let message = request.message.clone();
    let channel = request.channel.clone();
    let conversation_id = request.conversation_id.clone();
    let project_id = request.project_id.clone();
    let deep_research = request.deep_research;
    let execution_mode = request.execution_mode.clone();

    tokio::spawn(async move {
        let tracked_task = if chat_request_should_create_task(
            execution_mode.as_deref(),
            &message,
            deep_research,
            request.attachments_present,
        ) {
            let work_type =
                classify_chat_task_work_type(&message, deep_research, request.attachments_present)
                    .to_string();
            let description = build_chat_task_description(&message, &work_type);
            let mut task = crate::core::Task::new(
                description.clone(),
                "chat_request".to_string(),
                serde_json::json!({
                    "_task_kind": "chat_request",
                    "_origin": "chat",
                    "_execution_mode": normalized_chat_execution_mode(execution_mode.as_deref()),
                    "_work_type": work_type,
                    "message": message.clone(),
                    "channel": channel.clone(),
                    "conversation_id": conversation_id.clone(),
                    "project_id": project_id.clone(),
                    "deep_research": deep_research,
                    "attachments_present": request.attachments_present,
                }),
            );
            task.status = crate::core::TaskStatus::InProgress;
            task.approval = crate::core::TaskApproval::Auto;

            let add_result = {
                let agent_guard = agent_ref.read().await;
                agent_guard.add_task(task.clone()).await
            };

            match add_result {
                Ok(()) => {
                    let payload = serde_json::json!({
                        "task_id": task.id.to_string(),
                        "description": description.clone(),
                        "status": "in_progress",
                        "work_type": work_type.clone(),
                        "conversation_id": conversation_id.clone(),
                        "project_id": project_id.clone(),
                    });
                    let event = Event::default()
                        .event("task_started")
                        .data(serde_json::to_string(&payload).unwrap_or_default());
                    let _ = tx.send(Ok(event)).await;
                    Some((task, work_type))
                }
                Err(error) => {
                    tracing::warn!("Failed to create chat task anchor: {}", error);
                    None
                }
            }
        } else {
            None
        };

        // Stream model tokens + tool progress as dedicated SSE events.
        let (stream_tx, mut stream_rx) =
            tokio::sync::mpsc::channel::<crate::core::StreamEvent>(256);
        let stream_forwarder = {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut last_thinking_detail = String::new();
                while let Some(ev) = stream_rx.recv().await {
                    let (maybe_event, next_thinking_detail) =
                        normalize_stream_event_for_sse(ev, &last_thinking_detail);
                    last_thinking_detail = next_thinking_detail;
                    let Some((event_name, payload)) = maybe_event else {
                        continue;
                    };
                    let event = Event::default()
                        .event(event_name)
                        .data(serde_json::to_string(&payload).unwrap_or_default());
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
            })
        };

        // Poll trace for new steps and emit as SSE events
        let trace_poller = {
            let tx = tx.clone();
            let trace_ref = trace_ref.clone();
            tokio::spawn(async move {
                let mut last_step_count = 0;
                let start = std::time::Instant::now();
                let mut last_progress_at = std::time::Instant::now();
                let mut last_heartbeat_at = std::time::Instant::now();
                const HEARTBEAT_SECS: u64 = 3;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    // Timeout safety: 30 min max (long app deploys / self-heal retries can take a while)
                    if start.elapsed().as_secs() > 1800 {
                        break;
                    }
                    let trace = trace_ref.read().await;
                    let current_count = trace.steps.len();
                    if current_count > last_step_count {
                        for step in &trace.steps[last_step_count..current_count] {
                            let event_data = serde_json::json!({
                                "icon": step.icon,
                                "title": step.title,
                                "detail": step.detail,
                                "step_type": step.step_type,
                                "data": step.data,
                                "time": step.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                                "duration_ms": step.duration_ms,
                            });
                            let event = Event::default()
                                .event("thinking")
                                .data(serde_json::to_string(&event_data).unwrap_or_default());
                            if tx.send(Ok(event)).await.is_err() {
                                return;
                            }
                        }
                        last_step_count = current_count;
                        last_progress_at = std::time::Instant::now();
                        last_heartbeat_at = last_progress_at;
                    }
                    if trace.completed_at.is_some() {
                        break;
                    }
                    if last_progress_at.elapsed().as_secs() >= HEARTBEAT_SECS
                        && last_heartbeat_at.elapsed().as_secs() >= HEARTBEAT_SECS
                    {
                        let idle_secs = last_progress_at.elapsed().as_secs();
                        let phase_hint = trace
                            .steps
                            .last()
                            .map(|s| {
                                let title = s.title.to_ascii_lowercase();
                                let detail = s.detail.to_ascii_lowercase();
                                let memory_is_active = (title.contains("memory")
                                    || detail.contains("memory")
                                    || title.contains("mem0")
                                    || detail.contains("mem0"))
                                    && !detail.contains("available on demand");
                                if memory_is_active {
                                    if detail.contains("mem0 pending") {
                                        "Memory layer is starting up (first run may include embedding warmup)."
                                    } else if detail.contains("mem0 active") {
                                        "Retrieving semantic memory/context."
                                    } else if detail.contains("warmup") {
                                        "Memory layer warmup in progress."
                                    } else {
                                        "Memory/context setup in progress."
                                    }
                                } else if title.contains("context") || detail.contains("context") {
                                    "Preparing conversation context."
                                } else if title.contains("repairing deploy payload")
                                    || detail.contains("deploy payload")
                                    || detail.contains("files payload")
                                {
                                    "Regenerating deploy payload (model is building required files map)."
                                } else if title.contains("llm request") || title.contains("llm call") {
                                    "Waiting on model response."
                                } else if title.contains("tool") || detail.contains("tool") {
                                    "Waiting on tool execution."
                                } else {
                                    "Still processing."
                                }
                            })
                            .unwrap_or("Still processing.");
                        let event_data = serde_json::json!({
                            "icon": "[wait]",
                            "title": "Still Working",
                            "detail": format!("{} No new output yet.", phase_hint),
                            "step_type": "heartbeat",
                            "data": serde_json::json!({ "idle_secs": idle_secs })
                        });
                        let event = Event::default()
                            .event("thinking")
                            .data(serde_json::to_string(&event_data).unwrap_or_default());
                        if tx.send(Ok(event)).await.is_err() {
                            return;
                        }
                        last_heartbeat_at = std::time::Instant::now();
                    }
                }
            })
        };

        // Run the actual agent processing
        let initial_status = Event::default().event("thinking").data(
            serde_json::json!({
                "icon": "[recv]",
                "title": "Request received",
                "detail": "Preparing model call and tool plan...",
                "step_type": "thinking",
                "data": null
            })
            .to_string(),
        );
        let _ = tx.send(Ok(initial_status)).await;

        let result = {
            let agent_guard = agent_ref.read().await;
            agent_guard
                .process_message_stream_with_meta_and_hints(
                    &message,
                    &channel,
                    conversation_id.as_deref(),
                    project_id.as_deref(),
                    trace_ref.clone(),
                    stream_tx,
                    crate::core::RequestExecutionHints { deep_research },
                )
                .await
        };

        // Ensure the trace is marked complete even on early errors, so the poller can't hang.
        {
            let mut trace = trace_ref.write().await;
            if trace.completed_at.is_none() {
                trace.completed_at = Some(chrono::Utc::now());
            }
        }

        // Wait for poller to catch up
        let _ = trace_poller.await;
        let _ = stream_forwarder.await;

        // Emit final response
        match result {
            Ok(processed) => {
                if let Some((task, work_type)) = tracked_task.as_ref() {
                    let terminal_status = chat_task_terminal_status(&processed.response);
                    let result_preview = truncate_stream_task_text(
                        if processed.response.trim().is_empty() {
                            "Task completed."
                        } else {
                            &processed.response
                        },
                        400,
                    );
                    {
                        let agent_guard = agent_ref.read().await;
                        if let Err(error) = agent_guard
                            .finalize_task(
                                task.id,
                                terminal_status.clone(),
                                Some(result_preview.clone()),
                            )
                            .await
                        {
                            tracing::warn!(
                                "Failed to finalize streamed chat task '{}': {}",
                                task.id,
                                error
                            );
                        }
                    }
                    let status_event = Event::default().event("task_status").data(
                        serde_json::json!({
                            "task_id": task.id.to_string(),
                            "description": task.description,
                            "status": chat_task_status_key(&terminal_status),
                            "work_type": work_type,
                            "result_preview": result_preview,
                            "conversation_id": processed.conversation_id.clone().or(conversation_id.clone()),
                            "project_id": project_id.clone(),
                        })
                        .to_string(),
                    );
                    let _ = tx.send(Ok(status_event)).await;
                }

                let mut content = serde_json::json!({
                    "content": processed.response,
                    "conversation_id": processed.conversation_id.or(conversation_id),
                });
                if let Some(title) = processed.conversation_title {
                    content["conversation_title"] = serde_json::json!(title);
                }
                let event = Event::default()
                    .event("content")
                    .data(serde_json::to_string(&content).unwrap_or_default());
                let _ = tx.send(Ok(event)).await;
            }
            Err(e) => {
                if let Some((task, work_type)) = tracked_task.as_ref() {
                    let error_text = e.to_string();
                    {
                        let agent_guard = agent_ref.read().await;
                        if let Err(finalize_error) = agent_guard
                            .finalize_task(
                                task.id,
                                crate::core::TaskStatus::Failed {
                                    error: error_text.clone(),
                                },
                                Some(truncate_stream_task_text(&error_text, 400)),
                            )
                            .await
                        {
                            tracing::warn!(
                                "Failed to finalize failed streamed chat task '{}': {}",
                                task.id,
                                finalize_error
                            );
                        }
                    }
                    let status_event = Event::default().event("task_status").data(
                        serde_json::json!({
                            "task_id": task.id.to_string(),
                            "description": task.description,
                            "status": "failed",
                            "work_type": work_type,
                            "result_preview": truncate_stream_task_text(&error_text, 400),
                            "conversation_id": conversation_id.clone(),
                            "project_id": project_id.clone(),
                        })
                        .to_string(),
                    );
                    let _ = tx.send(Ok(status_event)).await;
                }

                let error = serde_json::json!({ "error": e.to_string() });
                let event = Event::default()
                    .event("error")
                    .data(serde_json::to_string(&error).unwrap_or_default());
                let _ = tx.send(Ok(event)).await;
            }
        }

        let done = Event::default().event("done").data("{}");
        let _ = tx.send(Ok(done)).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Clear conversation history for a channel
async fn clear_chat(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let channel = request
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("web");
    let project_id = request.get("project_id").and_then(|v| v.as_str());
    let agent = state.agent.read().await;
    if let Some(pid) = project_id {
        agent
            .clear_conversation_for_project(channel, Some(pid))
            .await;
    } else {
        agent.clear_conversation_history(channel).await;
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "cleared" })),
    )
        .into_response()
}

async fn list_tasks(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let tasks = state.tasks.read().await;
    let all = tasks.all();
    let total = all.len();

    let task_infos: Vec<TaskInfo> = all
        .iter()
        .skip(offset)
        .take(limit)
        .map(|t| TaskInfo {
            id: t.id.to_string(),
            description: t.description.clone(),
            action: t.action.clone(),
            arguments: t.arguments.clone(),
            status: format!("{:?}", t.status),
            cron: t.cron.clone(),
            result: t.result.clone(),
            created_at: t.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    Json(
        serde_json::json!({ "tasks": task_infos, "total": total, "limit": limit, "offset": offset }),
    )
}

fn automation_task_status_label(status: &TaskStatus) -> String {
    match status {
        TaskStatus::Pending => "pending".to_string(),
        TaskStatus::AwaitingApproval => "awaiting_approval".to_string(),
        TaskStatus::Paused => "paused".to_string(),
        TaskStatus::InProgress => "in_progress".to_string(),
        TaskStatus::Completed => "completed".to_string(),
        TaskStatus::Failed { .. } => "failed".to_string(),
        TaskStatus::Cancelled => "cancelled".to_string(),
    }
}

fn automation_task_next_run_at(task: &Task, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    if let Some(scheduled_for) = task.scheduled_for {
        if task.cron.is_some() || scheduled_for >= now - chrono::Duration::seconds(5) {
            return Some(scheduled_for.to_rfc3339());
        }
    }
    let cron = task.cron.as_deref()?.trim();
    if cron.is_empty() {
        return None;
    }
    cron.parse::<cron::Schedule>()
        .ok()?
        .upcoming(chrono::Utc)
        .next()
        .map(|dt| dt.to_rfc3339())
}

fn automation_watcher_status_label(status: &crate::core::watcher::WatcherStatus) -> String {
    match status {
        crate::core::watcher::WatcherStatus::Active => "active".to_string(),
        crate::core::watcher::WatcherStatus::Paused => "paused".to_string(),
        crate::core::watcher::WatcherStatus::Triggered => "triggered".to_string(),
        crate::core::watcher::WatcherStatus::TimedOut => "timed_out".to_string(),
        crate::core::watcher::WatcherStatus::Cancelled => "cancelled".to_string(),
        crate::core::watcher::WatcherStatus::Failed { .. } => "failed".to_string(),
    }
}

fn automation_run_status_label(status: &crate::core::AutomationRunStatus) -> String {
    match status {
        crate::core::AutomationRunStatus::Running => "running".to_string(),
        crate::core::AutomationRunStatus::Succeeded => "succeeded".to_string(),
        crate::core::AutomationRunStatus::Failed => "failed".to_string(),
        crate::core::AutomationRunStatus::Retrying => "retrying".to_string(),
        crate::core::AutomationRunStatus::TimedOut => "timed_out".to_string(),
        crate::core::AutomationRunStatus::Triggered => "triggered".to_string(),
    }
}

fn automation_watcher_condition_label(condition: &crate::core::watcher::WatchCondition) -> String {
    match condition {
        crate::core::watcher::WatchCondition::NotEmpty => {
            "Trigger when results are not empty".to_string()
        }
        crate::core::watcher::WatchCondition::Contains { keyword } => {
            format!("Trigger when results contain \"{}\"", keyword)
        }
        crate::core::watcher::WatchCondition::Matches { pattern } => {
            format!("Trigger when results match {}", pattern)
        }
        crate::core::watcher::WatchCondition::Custom { description } => description.clone(),
    }
}

fn automation_watcher_next_run_at(watcher: &crate::core::watcher::Watcher) -> Option<String> {
    if !matches!(
        watcher.status,
        crate::core::watcher::WatcherStatus::Active | crate::core::watcher::WatcherStatus::Paused
    ) {
        return None;
    }
    let base = watcher.last_poll_at.unwrap_or(watcher.created_at);
    Some((base + chrono::Duration::seconds(watcher.interval_secs as i64)).to_rfc3339())
}

async fn list_automation_objects(State(state): State<AppState>) -> Json<serde_json::Value> {
    let now = chrono::Utc::now();
    let mut objects: Vec<AutomationObjectInfo> = Vec::new();
    let mut totals = AutomationInventoryTotals::default();

    {
        let tasks = state.tasks.read().await;
        for task in tasks.all() {
            if task.action == "goal" {
                continue;
            }
            totals.tasks += 1;
            let detail = task
                .result
                .as_ref()
                .and_then(|result| {
                    let trimmed = result.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.chars().take(140).collect::<String>())
                    }
                })
                .or_else(|| {
                    task.cron
                        .as_ref()
                        .map(|cron| format!("Recurring schedule: {}", cron))
                });
            objects.push(AutomationObjectInfo {
                id: task.id.to_string(),
                kind: "task".to_string(),
                title: task.description.clone(),
                subtitle: Some(task.action.clone()),
                status: automation_task_status_label(&task.status),
                detail,
                created_at: Some(task.created_at.to_rfc3339()),
                next_run_at: automation_task_next_run_at(task, now),
                view: "tasks".to_string(),
                url: None,
                enabled: None,
                connected: None,
            });
        }
    }

    let (config_dir, data_dir, integrations_info, watchers) = {
        let agent = state.agent.read().await;
        (
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.integrations.list().await,
            agent.watcher_manager.list().await,
        )
    };

    for watcher in &watchers {
        totals.watchers += 1;
        let detail = if let Some(trigger_result) = watcher.trigger_result.as_ref() {
            let preview = trigger_result.chars().take(140).collect::<String>();
            Some(format!(
                "{} | Trigger result: {}",
                automation_watcher_condition_label(&watcher.condition),
                preview
            ))
        } else {
            Some(automation_watcher_condition_label(&watcher.condition))
        };
        objects.push(AutomationObjectInfo {
            id: watcher.id.to_string(),
            kind: "watcher".to_string(),
            title: watcher.description.clone(),
            subtitle: Some(watcher.poll_action.clone()),
            status: automation_watcher_status_label(&watcher.status),
            detail,
            created_at: Some(watcher.created_at.to_rfc3339()),
            next_run_at: automation_watcher_next_run_at(watcher),
            view: "watchers".to_string(),
            url: None,
            enabled: None,
            connected: None,
        });
    }

    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))
            .ok();
    for info in integrations_info {
        if integrations::external_integration_config(&info.id).is_none() {
            continue;
        }
        let (status, detail) = if info.id == "google_calendar" {
            let configured = integrations::calendar_oauth_pair(manager.as_ref()).is_some();
            let has_refresh_token = integrations::oauth_has_refresh_token(
                integrations::stored_secret(manager.as_ref(), "calendar_tokens"),
            );
            if has_refresh_token {
                match integrations::validate_calendar_oauth_connection(&config_dir).await {
                    Ok(()) => ("connected".to_string(), None),
                    Err(error) => ("error".to_string(), Some(error)),
                }
            } else if configured {
                (
                    "needs_auth".to_string(),
                    Some("Google sign-in required to finish connecting Calendar.".to_string()),
                )
            } else {
                ("not_configured".to_string(), None)
            }
        } else {
            match info.status {
                crate::integrations::IntegrationStatus::NotConfigured => {
                    ("not_configured".to_string(), None)
                }
                crate::integrations::IntegrationStatus::NeedsAuth => {
                    ("needs_auth".to_string(), None)
                }
                crate::integrations::IntegrationStatus::Connected => {
                    ("connected".to_string(), None)
                }
                crate::integrations::IntegrationStatus::Error(error) => {
                    ("error".to_string(), Some(error))
                }
            }
        };
        let enabled = manager
            .as_ref()
            .and_then(|m| {
                m.get_custom_secret(&integrations::integration_enabled_key(&info.id))
                    .ok()
                    .flatten()
            })
            .and_then(|v| integrations::parse_boolish(&v))
            .unwrap_or(status == "connected");
        totals.integrations += 1;
        objects.push(AutomationObjectInfo {
            id: info.id.clone(),
            kind: "integration".to_string(),
            title: info.name.clone(),
            subtitle: Some(info.description.clone()),
            status: status.clone(),
            detail: detail.or_else(|| {
                Some(if enabled {
                    "Enabled".to_string()
                } else {
                    "Disabled".to_string()
                })
            }),
            created_at: None,
            next_run_at: None,
            view: "settings".to_string(),
            url: None,
            enabled: Some(enabled),
            connected: Some(status == "connected"),
        });
    }

    if integrations::external_integration_config("gmail").is_some() {
        let has_refresh_token = integrations::oauth_has_refresh_token(integrations::stored_secret(
            manager.as_ref(),
            "gmail_tokens",
        ));
        let configured = integrations::gmail_oauth_pair(manager.as_ref()).is_some();
        let (status, detail) = if has_refresh_token {
            match integrations::validate_gmail_oauth_connection(&config_dir).await {
                Ok(()) => ("connected".to_string(), None),
                Err(error) => ("error".to_string(), Some(error)),
            }
        } else if configured {
            (
                "needs_auth".to_string(),
                Some("Google sign-in required to finish connecting Gmail.".to_string()),
            )
        } else {
            ("not_configured".to_string(), None)
        };
        let enabled = manager
            .as_ref()
            .and_then(|m| {
                m.get_custom_secret(&integrations::integration_enabled_key("gmail"))
                    .ok()
                    .flatten()
            })
            .and_then(|v| integrations::parse_boolish(&v))
            .unwrap_or(status == "connected");
        totals.integrations += 1;
        objects.push(AutomationObjectInfo {
            id: "gmail".to_string(),
            kind: "integration".to_string(),
            title: "Gmail".to_string(),
            subtitle: Some("Connect Gmail to read, triage, and reply to email".to_string()),
            status: status.clone(),
            detail: detail.or_else(|| {
                Some(if enabled {
                    "Enabled".to_string()
                } else {
                    "Disabled".to_string()
                })
            }),
            created_at: None,
            next_run_at: None,
            view: "settings".to_string(),
            url: None,
            enabled: Some(enabled),
            connected: Some(status == "connected"),
        });
    }

    for app in state.app_registry.list().await {
        let row = app.as_object().cloned().unwrap_or_default();
        totals.apps += 1;
        let running = row
            .get("running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        objects.push(AutomationObjectInfo {
            id: row
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            kind: "app".to_string(),
            title: row
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("App")
                .to_string(),
            subtitle: Some(
                row.get("runtime_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            ),
            status: if running {
                "running".to_string()
            } else {
                "stopped".to_string()
            },
            detail: Some(
                if row
                    .get("access_guard_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    "Access guard enabled".to_string()
                } else {
                    "Public in local workspace".to_string()
                },
            ),
            created_at: row
                .get("created_at")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            next_run_at: None,
            view: "apps".to_string(),
            url: row
                .get("access_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            enabled: Some(running),
            connected: None,
        });
    }

    totals.total = objects.len();
    Json(serde_json::json!({
        "objects": objects,
        "totals": totals
    }))
}

async fn list_automation_runs_endpoint(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (runs, supervisor_states) = {
        let agent = state.agent.read().await;
        (
            crate::core::list_automation_runs(&agent.storage, 30)
                .await
                .unwrap_or_default(),
            crate::core::list_automation_supervisor_states(&agent.storage)
                .await
                .unwrap_or_default(),
        )
    };
    let state_map: HashMap<String, crate::core::AutomationSupervisorState> = supervisor_states
        .into_iter()
        .map(|state| (state.automation_id.clone(), state))
        .collect();

    let items: Vec<AutomationRunInfo> = runs
        .into_iter()
        .map(|run| {
            let current_status = state_map
                .get(&run.automation_id)
                .map(|state| state.status.clone());
            AutomationRunInfo {
                id: run.id,
                automation_id: run.automation_id,
                kind: run.automation_kind.clone(),
                title: run.title,
                action: run.action,
                trigger: run.trigger,
                status: automation_run_status_label(&run.status),
                current_status,
                attempt: run.attempt,
                started_at: run.started_at,
                completed_at: run.completed_at,
                duration_ms: run.duration_ms,
                summary: run.critique.summary,
                output_preview: run.output_preview,
                error: run.error,
                next_retry_at: run.next_retry_at,
                conversation_id: run.origin.conversation_id,
                project_id: run.origin.project_id,
                view: match run.automation_kind.as_str() {
                    "task" => "tasks".to_string(),
                    "watcher" => "watchers".to_string(),
                    "app" => "apps".to_string(),
                    "integration" => "settings".to_string(),
                    _ => "trace".to_string(),
                },
            }
        })
        .collect();

    Json(serde_json::json!({
        "runs": items
    }))
}

// =============================================================================
// Goals API (goals are stored as tasks with action="goal")
// =============================================================================

/// List goals (paginated)
async fn list_goals(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let tasks = state.tasks.read().await;
    let all_goals: Vec<_> = tasks.all().iter().filter(|t| t.action == "goal").collect();
    let total = all_goals.len();
    let goals: Vec<serde_json::Value> = all_goals
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|t| {
            let mut g = serde_json::json!({
                "id": t.id.to_string(),
                "description": t.description,
                "status": format!("{:?}", t.status),
                "created_at": t.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            });
            if let Some(due) = t.scheduled_for {
                g["due_date"] = serde_json::json!(due.format("%Y-%m-%d").to_string());
            }
            if let Some(goal_id) = t.arguments.get("goal_id").and_then(|v| v.as_str()) {
                g["goal_id"] = serde_json::json!(goal_id);
                g["autopilot"] = serde_json::json!(true);
            } else {
                g["autopilot"] = serde_json::json!(false);
            }
            if let Some(goal_text) = t.arguments.get("goal").and_then(|v| v.as_str()) {
                g["goal"] = serde_json::json!(goal_text);
            }
            g
        })
        .collect();
    (
        StatusCode::OK,
        Json(
            serde_json::json!({ "goals": goals, "total": total, "limit": limit, "offset": offset }),
        ),
    )
        .into_response()
}

/// Create a goal
async fn create_goal(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let description = match request.get("description").and_then(|v| v.as_str()) {
        Some(d) if !d.trim().is_empty() => d.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing or empty description".to_string(),
                }),
            )
                .into_response()
        }
    };

    // Parse optional due date (YYYY-MM-DD)
    let due_date = request
        .get("due_date")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .map(|d| d.and_hms_opt(23, 59, 59).unwrap())
        .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc));

    let mut task = crate::core::Task::new(
        description.clone(),
        "goal".to_string(),
        serde_json::json!({}),
    );
    task.scheduled_for = due_date;

    // Persist to database
    {
        let agent = state.agent.read().await;
        if let Err(e) = agent.storage.insert_task(&task).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to save goal: {}", e),
                }),
            )
                .into_response();
        }
    }

    // Add to in-memory queue
    {
        let mut queue = state.tasks.write().await;
        queue.add(task);
    }

    // Auto-schedule reminder tasks if due date is set and > 1 day away
    if let Some(due) = due_date {
        let now = chrono::Utc::now();
        let days_until = (due - now).num_days();

        let mut reminders = Vec::new();
        // Reminder 1 day before
        if days_until > 1 {
            let remind_at = due - chrono::Duration::days(1);
            let mut r = crate::core::Task::new(
                format!("Reminder: \"{}\" is due tomorrow", description),
                "goal_reminder".to_string(),
                serde_json::json!({"goal": description, "days_left": 1}),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }
        // Reminder 3 days before (if goal is > 3 days out)
        if days_until > 3 {
            let remind_at = due - chrono::Duration::days(3);
            let mut r = crate::core::Task::new(
                format!("Reminder: \"{}\" is due in 3 days", description),
                "goal_reminder".to_string(),
                serde_json::json!({"goal": description, "days_left": 3}),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }

        if !reminders.is_empty() {
            let agent = state.agent.read().await;
            let mut queue = state.tasks.write().await;
            for r in reminders {
                let _ = agent.storage.insert_task(&r).await;
                queue.add(r);
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Delete a goal
async fn delete_goal_endpoint(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    // Best-effort cascade delete:
    // - the goal task itself (by task id OR by goal_id)
    // - any goal-loop tasks keyed by arguments.goal_id (plan + scheduled reports)
    // - reminder tasks that match the goal description (legacy reminders without goal_id)
    //
    // Why: goal-loop "plan" tasks use action="plan" (not "goal_loop_plan"), but they still
    // carry arguments.goal_id. Without this cascade, deleting the goal leaves orphan tasks
    // visible in "Next Up".
    let all_tasks = {
        let agent = state.agent.read().await;
        agent.storage.get_tasks().await.unwrap_or_default()
    };

    // Identify the goal task and canonical goal_id.
    let mut goal_task_id: Option<String> = None;
    let mut goal_id: Option<String> = None;
    let mut goal_desc: Option<String> = None;

    // 1) Treat `id` as a goal task id.
    if let Some(t) = all_tasks.iter().find(|t| t.id == id && t.action == "goal") {
        goal_task_id = Some(t.id.clone());
        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&t.arguments) {
            goal_id = args
                .get("goal_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            goal_desc = args
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if goal_desc.is_none() {
            goal_desc = Some(t.description.clone());
        }
    }

    // 2) Treat `id` as a goal_id (common UI identifier).
    if goal_task_id.is_none() {
        let mut found: Option<(&crate::storage::entities::task::Model, serde_json::Value)> = None;
        for t in &all_tasks {
            if t.action != "goal" {
                continue;
            }
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&t.arguments) else {
                continue;
            };
            if args.get("goal_id").and_then(|v| v.as_str()) == Some(id.as_str()) {
                found = Some((t, args));
                break;
            }
        }
        if let Some((t, args)) = found {
            goal_task_id = Some(t.id.clone());
            goal_id = Some(id.clone());
            goal_desc = args
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some(t.description.clone()));
        }
    }

    // If we still didn't find a goal task, we can still cascade-delete tasks by goal_id.
    if goal_id.is_none() {
        let mut any_ref = false;
        for t in &all_tasks {
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&t.arguments) else {
                continue;
            };
            if args.get("goal_id").and_then(|v| v.as_str()) == Some(id.as_str()) {
                any_ref = true;
                break;
            }
        }
        if any_ref {
            goal_id = Some(id.clone());
        }
    }

    if goal_task_id.is_none() && goal_id.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Goal not found".to_string(),
            }),
        )
            .into_response();
    }

    let mut ids_to_delete: Vec<String> = Vec::new();
    if let Some(gid) = goal_task_id.clone() {
        ids_to_delete.push(gid);
    }

    for t in &all_tasks {
        let tid = &t.id;
        if ids_to_delete.iter().any(|x| x == tid) {
            continue;
        }

        let args = serde_json::from_str::<serde_json::Value>(&t.arguments).ok();
        let arg_goal_id = args
            .as_ref()
            .and_then(|a| a.get("goal_id"))
            .and_then(|v| v.as_str());
        let arg_goal_desc = args
            .as_ref()
            .and_then(|a| a.get("goal"))
            .and_then(|v| v.as_str());

        let matches_goal_id = goal_id
            .as_deref()
            .and_then(|gid| arg_goal_id.map(|x| x == gid))
            .unwrap_or(false);
        let matches_goal_desc = goal_desc
            .as_deref()
            .and_then(|gd| arg_goal_desc.map(|x| x == gd))
            .unwrap_or(false);

        // Goal loop tasks we want to remove:
        // - Scheduled progress report task
        // - Goal reminders (legacy match by goal text)
        // - The "Goal Loop Plan: ..." task (action="plan", matches by description prefix + goal_id)
        let is_progress_report = t.action == "goal_progress_report";
        let is_goal_reminder = t.action == "goal_reminder";
        let is_goal_loop_plan = t.action == "plan" && t.description.starts_with("Goal Loop Plan:");
        let is_legacy_goal_loop_plan = t.action == "goal_loop_plan";

        if matches_goal_id && (is_progress_report || is_goal_loop_plan || is_legacy_goal_loop_plan)
        {
            ids_to_delete.push(t.id.clone());
            continue;
        }

        if is_goal_reminder && matches_goal_desc {
            ids_to_delete.push(t.id.clone());
            continue;
        }
    }

    // Delete from database
    {
        let agent = state.agent.read().await;
        for tid in &ids_to_delete {
            let _ = agent.storage.delete_task(tid).await;
        }
    }

    // Remove from in-memory queue
    {
        let mut queue = state.tasks.write().await;
        for tid in &ids_to_delete {
            if let Ok(uuid) = uuid::Uuid::parse_str(tid) {
                queue.remove(uuid);
            }
        }
    }

    let deleted_notifications = if let Some(goal_text) = goal_desc.as_deref() {
        let agent = state.agent.read().await;
        agent
            .storage
            .delete_goal_notifications(goal_text)
            .await
            .unwrap_or(0)
    } else {
        0
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "deleted_task_ids": ids_to_delete,
            "deleted_notifications": deleted_notifications,
        })),
    )
        .into_response()
}

/// Create a new task
async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Response {
    use crate::core::{status_for_task_approval, Task, TaskApproval};

    // Convert and validate cron expression if provided
    // Standard 5-field cron is converted to 6-field (with seconds) for Rust cron crate
    let cron_expr = request.cron.as_ref().map(|expr| {
        if expr.split_whitespace().count() == 5 {
            format!("0 {}", expr) // Prepend "0 " for seconds
        } else {
            expr.clone()
        }
    });

    if let Some(ref cron) = cron_expr {
        if cron.parse::<cron::Schedule>().is_err() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid cron expression: {}", cron),
                }),
            )
                .into_response();
        }
    }

    let approval = match request.approval.as_deref() {
        Some("require") => TaskApproval::RequireApproval,
        Some("notify") => TaskApproval::RequireApproval,
        _ => TaskApproval::Auto,
    };

    let status = status_for_task_approval(&approval);

    let task = Task {
        id: uuid::Uuid::new_v4(),
        description: request.description,
        action: request.action.clone(),
        arguments: request.arguments,
        approval,
        capabilities: vec![request.action],
        status,
        created_at: chrono::Utc::now(),
        scheduled_for: None,
        cron: cron_expr,
        result: None,
        proof_id: None,
        priority: None,
        urgency: None,
        importance: None,
        eisenhower_quadrant: None,
    };

    let is_scheduled = task.cron.is_some();
    let save_result = {
        let agent = state.agent.read().await;
        agent
            .add_or_update_similar_task(task.clone(), request.allow_duplicate)
            .await
    };
    let (task_id, reused_existing, removed_duplicates) = match save_result {
        Ok(outcome) => outcome,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to save task: {}", e),
                }),
            )
                .into_response();
        }
    };

    let message = if reused_existing {
        if is_scheduled {
            "Scheduled task updated"
        } else {
            "Task updated"
        }
    } else if is_scheduled {
        "Scheduled task created"
    } else {
        "Task created"
    };

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_created");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "id": task_id.to_string(),
            "message": message,
            "reused_existing": reused_existing,
            "removed_duplicates": removed_duplicates,
        })),
    )
        .into_response()
}

/// Update a task (description, arguments, cron)
async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateTaskRequest>,
) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let mut tasks = state.tasks.write().await;
    let Some(task) = tasks.get_mut(uuid) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found".to_string(),
            }),
        )
            .into_response();
    };

    let mut desc_to_save = None;
    let mut args_to_save = None;
    let mut cron_to_save = None;

    if let Some(description) = request.description {
        if !description.trim().is_empty() {
            task.description = description;
            desc_to_save = Some(task.description.clone());
        }
    }

    if let Some(arguments) = request.arguments {
        task.arguments = arguments;
        args_to_save =
            Some(serde_json::to_string(&task.arguments).unwrap_or_else(|_| "{}".to_string()));
    }

    if let Some(cron_value) = request.cron {
        let cron_clean = if cron_value.trim().is_empty() {
            None
        } else if cron_value.split_whitespace().count() == 5 {
            Some(format!("0 {}", cron_value))
        } else {
            Some(cron_value)
        };

        if let Some(ref cron) = cron_clean {
            if cron.parse::<cron::Schedule>().is_err() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid cron expression: {}", cron),
                    }),
                )
                    .into_response();
            }
        }

        task.cron = cron_clean;
        cron_to_save = task.cron.clone();
    }

    let save_result = {
        let agent = state.agent.read().await;
        agent
            .storage
            .update_task(&id, desc_to_save, args_to_save, cron_to_save, None)
            .await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to update task: {}", e),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_updated");
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Delete a task
async fn delete_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let mut tasks = state.tasks.write().await;
    let removed = tasks.remove(uuid);

    if removed {
        let delete_result = {
            let agent = state.agent.read().await;
            agent.storage.delete_task(&id).await
        };

        if let Err(e) = delete_result {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to delete task: {}", e),
                }),
            )
                .into_response();
        }

        (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found".to_string(),
            }),
        )
            .into_response()
    }
}

/// Approve a task for execution
async fn approve_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let agent = state.agent.read().await;
    match agent.approve_task_request(uuid, "api").await {
        Ok(Some(_)) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found or is not awaiting approval".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to approve task: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Reject a task
async fn reject_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let agent = state.agent.read().await;
    match agent
        .reject_task_request(uuid, "api", "Task was rejected and will not be executed.")
        .await
    {
        Ok(Some(_)) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found or is not awaiting approval".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to reject task: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Cancel a queued or running task.
async fn cancel_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let mut tasks = state.tasks.write().await;
    let Some(task) = tasks.get_mut(uuid) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found".to_string(),
            }),
        )
            .into_response();
    };

    if !matches!(
        task.status,
        TaskStatus::Pending
            | TaskStatus::AwaitingApproval
            | TaskStatus::Paused
            | TaskStatus::InProgress
    ) {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "Only queued, paused, approval-pending, or running tasks can be cancelled."
                    .to_string(),
            }),
        )
            .into_response();
    }

    task.status = TaskStatus::Cancelled;
    let save_result = {
        let agent = state.agent.read().await;
        agent
            .storage
            .update_task_status(
                &id,
                &serde_json::to_string(&task.status).unwrap_or_else(|_| "Cancelled".to_string()),
            )
            .await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to cancel task: {}", e),
            }),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Pause a queued recurring or deferred task without deleting it.
async fn pause_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let status_json = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };

        if !matches!(
            task.status,
            TaskStatus::Pending | TaskStatus::AwaitingApproval
        ) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "Only queued or approval-pending tasks can be paused.".to_string(),
                }),
            )
                .into_response();
        }

        task.status = TaskStatus::Paused;
        serde_json::to_string(&task.status).unwrap_or_else(|_| "\"Paused\"".to_string())
    };

    let save_result = {
        let agent = state.agent.read().await;
        agent.storage.update_task_status(&id, &status_json).await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to pause task: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "paused": true})),
    )
        .into_response()
}

/// Resume a paused task so it can run again.
async fn resume_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let (status_json, scheduled_for_rfc3339) = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };

        if !matches!(task.status, TaskStatus::Paused) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "Only paused tasks can be resumed.".to_string(),
                }),
            )
                .into_response();
        }

        task.status = TaskStatus::Pending;
        let now = chrono::Utc::now();
        if task.cron.is_some()
            || task
                .scheduled_for
                .as_ref()
                .map(|dt| *dt <= now)
                .unwrap_or(false)
        {
            task.scheduled_for = Some(now);
        }

        (
            serde_json::to_string(&task.status).unwrap_or_else(|_| "\"Pending\"".to_string()),
            task.scheduled_for.as_ref().map(|dt| dt.to_rfc3339()),
        )
    };

    let save_result: anyhow::Result<()> = {
        let agent = state.agent.read().await;
        if let Err(err) = agent.storage.update_task_status(&id, &status_json).await {
            Err(err)
        } else if let Some(scheduled_for) = scheduled_for_rfc3339 {
            agent
                .storage
                .update_task(&id, None, None, None, Some(scheduled_for))
                .await
        } else {
            Ok(())
        }
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to resume task: {}", e),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_resumed");
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "resumed": true})),
    )
        .into_response()
}

/// Retry a failed or cancelled task.
async fn retry_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid task id".to_string(),
                }),
            )
                .into_response();
        }
    };

    let (status_json, scheduled_for_rfc3339) = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };

        if !matches!(
            task.status,
            TaskStatus::Failed { .. } | TaskStatus::Cancelled
        ) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "Only failed or cancelled tasks can be retried.".to_string(),
                }),
            )
                .into_response();
        }

        task.status = TaskStatus::Pending;
        task.result = None;
        task.proof_id = None;
        task.scheduled_for = if task.cron.is_some() || task.scheduled_for.is_some() {
            Some(chrono::Utc::now())
        } else {
            None
        };

        (
            serde_json::to_string(&task.status).unwrap_or_else(|_| "Pending".to_string()),
            task.scheduled_for.as_ref().map(|dt| dt.to_rfc3339()),
        )
    };

    let save_result = {
        let agent = state.agent.read().await;
        agent
            .storage
            .retry_task(&id, &status_json, scheduled_for_rfc3339)
            .await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to retry task: {}", e),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_retried");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Task queued for retry"
        })),
    )
        .into_response()
}

/// Plan a task using the LLM (returns a structured plan)
async fn plan_task(
    State(state): State<AppState>,
    Json(request): Json<PlanTaskRequest>,
) -> Response {
    const MAX_ACTIONS_FOR_PLAN: usize = 8;

    let (llm, actions) = {
        let agent = state.agent.read().await;
        let actions = match agent.runtime.list_actions().await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to list actions: {}", e),
                    }),
                )
                    .into_response();
            }
        };
        (agent.llm.clone(), actions)
    };

    let light_catalog = actions
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
            })
        })
        .collect::<Vec<_>>();

    let selector_prompt = r#"You are a task planner for an AI agent.
Return ONLY valid JSON. Do not include any extra text.

Output schema:
{
  "summary": "short summary",
  "needed_actions": ["action_name", "action_name"]
}

Rules:
- Use only the provided actions.
- Keep the list minimal (only what is necessary).
"#;

    let mut selector_message = format!(
        "Task description: {}\n\nAvailable actions (names + descriptions):\n{}",
        request.description,
        serde_json::to_string_pretty(&light_catalog).unwrap_or_default()
    );
    if let Some(prompt) = request.prompt.as_ref() {
        if !prompt.trim().is_empty() {
            selector_message.push_str("\n\nRefinement request:\n");
            selector_message.push_str(prompt);
        }
    }

    let selector_response = match llm
        .chat(selector_prompt, &selector_message, &[], &actions)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("LLM planning failed: {}", e),
                }),
            )
                .into_response();
        }
    };

    let selector_json = extract_json(&selector_response.content).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Planner returned invalid JSON for action selection".to_string(),
            }),
        )
            .into_response()
    });

    let selector_json = match selector_json {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let needed_action_names: Vec<String> = selector_json
        .get("needed_actions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut needed_actions = actions
        .iter()
        .filter(|s| needed_action_names.iter().any(|n| n == &s.name))
        .cloned()
        .collect::<Vec<_>>();

    if needed_actions.is_empty() {
        needed_actions = actions.iter().take(MAX_ACTIONS_FOR_PLAN).cloned().collect();
    } else if needed_actions.len() > MAX_ACTIONS_FOR_PLAN {
        needed_actions.truncate(MAX_ACTIONS_FOR_PLAN);
    }

    let detailed_catalog = needed_actions
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "input_schema": s.input_schema,
            })
        })
        .collect::<Vec<_>>();

    let plan_prompt = r#"You are a task planner for an AI agent.
Return ONLY valid JSON. Do not include any extra text.

Output schema:
{
  "summary": "short summary",
  "steps": [
    {
      "action": "action_name",
      "arguments": { "key": "value" },
      "rationale": "why this step is needed"
    }
  ],
  "notes": "optional"
}

Rules:
- Use only the provided actions.
- Provide JSON that is directly runnable.
- Keep steps minimal and ordered.
"#;

    let mut plan_message = format!(
        "Task description: {}\n\nAvailable actions (with schemas):\n{}",
        request.description,
        serde_json::to_string_pretty(&detailed_catalog).unwrap_or_default()
    );
    if let Some(prompt) = request.prompt.as_ref() {
        if !prompt.trim().is_empty() {
            plan_message.push_str("\n\nRefinement request:\n");
            plan_message.push_str(prompt);
        }
    }

    let plan_response = match llm
        .chat(plan_prompt, &plan_message, &[], &needed_actions)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("LLM planning failed: {}", e),
                }),
            )
                .into_response();
        }
    };

    let plan = extract_json(&plan_response.content);

    match plan {
        Some(plan) => (StatusCode::OK, Json(PlanTaskResponse { plan })).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Planner returned invalid JSON".to_string(),
            }),
        )
            .into_response(),
    }
}

fn extract_json(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .or_else(|| {
            let start = text.find('{')?;
            let end = text.rfind('}')?;
            serde_json::from_str::<serde_json::Value>(&text[start..=end]).ok()
        })
}

fn risk_level_label(level: &RiskLevel) -> &'static str {
    match level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

async fn load_autonomy_settings(agent: &Agent) -> AutonomySettings {
    agent.load_autonomy_settings().await
}

async fn save_autonomy_settings(agent: &Agent, settings: &AutonomySettings) -> Result<(), String> {
    agent.save_autonomy_settings(settings).await
}

fn recommendation(
    title: &str,
    description: &str,
    action_kind: &str,
    payload: serde_json::Value,
    trust_policy: &TrustPolicy,
) -> RecommendedAction {
    let id = uuid::Uuid::new_v4().to_string();
    let trust = score_action_risk(action_kind, &payload, trust_policy);
    RecommendedAction {
        id,
        title: title.to_string(),
        description: description.to_string(),
        action_kind: action_kind.to_string(),
        payload,
        trust,
    }
}

async fn apply_autopilot_mode(
    agent: &Agent,
    settings: &mut AutonomySettings,
    mode_id: &str,
) -> Result<serde_json::Value, String> {
    agent.apply_autopilot_mode(settings, mode_id).await
}

async fn run_chat_suggestion_scan(state: &AppState, trigger: &str) -> serde_json::Value {
    let Some(_scan_guard) = try_start_chat_suggestion_scan() else {
        return serde_json::json!({
            "status": "running",
            "message": "Chat suggestion scan already in progress"
        });
    };

    let (storage, encrypted_storage) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.encrypted_storage.clone())
    };
    let now = chrono::Utc::now();
    let now_rfc3339 = now.to_rfc3339();
    let mut scan_state = load_chat_suggestion_scan_state(&storage).await;

    if trigger != "manual" && !chat_suggestion_scan_is_due(&scan_state, now) {
        return serde_json::json!({
            "status": "not_due",
            "next_due_at": scan_state.next_due_at,
        });
    }

    scan_state.last_started_at = Some(now_rfc3339.clone());
    scan_state.last_status = Some("running".to_string());
    scan_state.last_error = None;
    save_chat_suggestion_scan_state(&storage, &scan_state).await;

    let has_user_chat = storage.has_user_chat_messages().await.unwrap_or(false);
    if !has_user_chat {
        scan_state.last_completed_at = Some(now_rfc3339.clone());
        scan_state.last_status = Some("no_user_chat".to_string());
        scan_state.next_due_at = Some(chat_suggestion_due_at(now));
        scan_state.defer_count = 0;
        scan_state.last_examined_chats = 0;
        scan_state.last_created_suggestions = 0;
        scan_state.last_low_signal_skips = 0;
        scan_state.last_artifact_skips = 0;
        scan_state.last_backlog_hint = 0;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "no_user_chat",
            "next_due_at": scan_state.next_due_at,
        });
    }

    if server_busy_for_chat_suggestions(state).await {
        scan_state.defer_count = scan_state.defer_count.saturating_add(1);
        scan_state.last_status = Some("deferred_busy".to_string());
        scan_state.next_due_at = Some(chat_suggestion_deferred_due_at(now, scan_state.defer_count));
        scan_state.last_error = None;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "deferred_busy",
            "next_due_at": scan_state.next_due_at,
            "defer_count": scan_state.defer_count,
        });
    }

    let mut suggestions = load_chat_suggestions(&storage).await;
    let internal_channels = ["arkpulse", "sentinel", "system", "autonomy"];
    let conversations = match storage
        .list_conversations_after_cursor(
            scan_state.cursor_updated_at.as_deref(),
            scan_state.cursor_conversation_id.as_deref(),
            CHAT_SUGGESTION_SCAN_FETCH_LIMIT,
            None,
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            scan_state.last_status = Some("error".to_string());
            scan_state.last_error = Some(error.to_string());
            scan_state.next_due_at = Some(chat_suggestion_deferred_due_at(now, 1));
            save_chat_suggestion_scan_state(&storage, &scan_state).await;
            return serde_json::json!({
                "status": "error",
                "message": error.to_string(),
            });
        }
    };

    if conversations.is_empty() {
        scan_state.last_completed_at = Some(now_rfc3339.clone());
        scan_state.last_status = Some("no_candidates".to_string());
        scan_state.next_due_at = Some(chat_suggestion_due_at(now));
        scan_state.defer_count = 0;
        scan_state.cursor_updated_at = None;
        scan_state.cursor_conversation_id = None;
        scan_state.last_examined_chats = 0;
        scan_state.last_created_suggestions = 0;
        scan_state.last_low_signal_skips = 0;
        scan_state.last_artifact_skips = 0;
        scan_state.last_backlog_hint = 0;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "no_candidates",
            "next_due_at": scan_state.next_due_at,
        });
    }

    let mut examined_chats = 0usize;
    let mut created_suggestions = 0usize;
    let mut low_signal_skips = 0usize;
    let mut artifact_skips = 0usize;

    for conversation in &conversations {
        if internal_channels.contains(&conversation.channel.as_str()) || conversation.archived {
            scan_state.cursor_updated_at = Some(conversation.updated_at.clone());
            scan_state.cursor_conversation_id = Some(conversation.id.clone());
            continue;
        }

        let recent_messages = encrypted_storage
            .get_recent_messages_decrypted(
                &conversation.id,
                CHAT_SUGGESTION_RECENT_MESSAGES_PER_CHAT as u64,
            )
            .await
            .unwrap_or_default();
        let latest_user = recent_messages
            .iter()
            .rev()
            .find(|message| message.role.eq_ignore_ascii_case("user"))
            .cloned();

        scan_state.cursor_updated_at = Some(conversation.updated_at.clone());
        scan_state.cursor_conversation_id = Some(conversation.id.clone());

        let Some(latest_user) = latest_user else {
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                None,
                None,
                &now_rfc3339,
            );
            continue;
        };

        let existing_watermark = scan_state
            .conversation_watermarks
            .iter()
            .find(|entry| entry.conversation_id == conversation.id);
        let already_scanned = existing_watermark.is_some_and(|entry| {
            entry.last_user_message_id.as_deref() == Some(latest_user.id.as_str())
                && entry.last_scanned_updated_at >= conversation.updated_at
        });
        if already_scanned {
            continue;
        }

        if examined_chats >= CHAT_SUGGESTION_SCAN_BATCH_LIMIT {
            break;
        }
        examined_chats += 1;

        if conversation_has_recent_app_artifact(&storage, &conversation.id).await {
            artifact_skips += 1;
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                Some(&latest_user.id),
                Some(&latest_user.timestamp),
                &now_rfc3339,
            );
            continue;
        }

        if !conversation_has_signal(&recent_messages) {
            low_signal_skips += 1;
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                Some(&latest_user.id),
                Some(&latest_user.timestamp),
                &now_rfc3339,
            );
            continue;
        }

        let Some(source_message) = extract_latest_signal_user_message(&recent_messages) else {
            low_signal_skips += 1;
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                Some(&latest_user.id),
                Some(&latest_user.timestamp),
                &now_rfc3339,
            );
            continue;
        };

        if let Some(mut suggestion) =
            infer_chat_automation_suggestion(conversation, &source_message)
        {
            let duplicate_idx = suggestions
                .iter()
                .position(|existing| existing.fingerprint == suggestion.fingerprint);
            if let Some(idx) = duplicate_idx {
                suggestions[idx].updated_at = now_rfc3339.clone();
                suggestions[idx].source_message_id = suggestion.source_message_id.clone();
                suggestions[idx].source_snippet = suggestion.source_snippet.clone();
                suggestions[idx].conversation_title = suggestion.conversation_title.clone();
                suggestions[idx].conversation_channel = suggestion.conversation_channel.clone();
                suggestions[idx].detail = suggestion.detail.clone();
                suggestions[idx].rationale = suggestion.rationale.clone();
                if suggestions[idx].status == "open" {
                    suggestions[idx].goal_title = suggestion.goal_title.clone();
                    suggestions[idx].goal_detail = suggestion.goal_detail.clone();
                }
            } else if suggestions
                .iter()
                .filter(|item| item.status == "open")
                .count()
                < CHAT_SUGGESTION_OPEN_LIMIT
            {
                suggestion.created_at = now_rfc3339.clone();
                suggestion.updated_at = now_rfc3339.clone();
                suggestions.push(suggestion);
                created_suggestions += 1;
            }
        }

        upsert_chat_suggestion_watermark(
            &mut scan_state,
            &conversation.id,
            &conversation.updated_at,
            Some(&latest_user.id),
            Some(&latest_user.timestamp),
            &now_rfc3339,
        );
    }

    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;

    scan_state.last_completed_at = Some(now_rfc3339.clone());
    scan_state.last_status = Some("completed".to_string());
    scan_state.last_error = None;
    scan_state.next_due_at = Some(chat_suggestion_due_at(now));
    scan_state.defer_count = 0;
    scan_state.last_examined_chats = examined_chats;
    scan_state.last_created_suggestions = created_suggestions;
    scan_state.last_low_signal_skips = low_signal_skips;
    scan_state.last_artifact_skips = artifact_skips;
    scan_state.last_backlog_hint = conversations.len().saturating_sub(examined_chats);
    if conversations.len() < CHAT_SUGGESTION_SCAN_FETCH_LIMIT as usize {
        scan_state.cursor_updated_at = None;
        scan_state.cursor_conversation_id = None;
    }
    save_chat_suggestion_scan_state(&storage, &scan_state).await;

    serde_json::json!({
        "status": "completed",
        "examined_chats": examined_chats,
        "created_suggestions": created_suggestions,
        "low_signal_skips": low_signal_skips,
        "artifact_skips": artifact_skips,
        "next_due_at": scan_state.next_due_at,
    })
}

async fn accept_chat_suggestion(
    state: &AppState,
    suggestion_id: &str,
) -> Result<serde_json::Value, String> {
    let storage = { state.agent.read().await.storage.clone() };
    let mut suggestions = load_chat_suggestions(&storage).await;
    let Some(idx) = suggestions
        .iter()
        .position(|suggestion| suggestion.id == suggestion_id)
    else {
        return Err("Suggestion not found".to_string());
    };

    if suggestions[idx].status != "open" {
        return Err("Suggestion is no longer open".to_string());
    }

    let started_at = chrono::Utc::now();
    let started_at_text = started_at.to_rfc3339();
    let suggestion = suggestions[idx].clone();
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace::default()));
    let trace_id = uuid::Uuid::new_v4().to_string();
    let prompt = build_chat_suggestion_execution_prompt(&suggestion);
    let suggestion_record_id = suggestion.id.clone();
    let run_snapshot = suggestions::capture_run_snapshot(state).await;

    {
        let mut trace = trace_ref.write().await;
        trace.id = trace_id.clone();
    }

    suggestions[idx].status = "accepted".to_string();
    suggestions[idx].updated_at = started_at_text.clone();
    suggestions[idx].accepted_at = Some(started_at_text.clone());
    suggestions[idx].run_status = Some("running".to_string());
    suggestions[idx].last_run_started_at = Some(started_at_text.clone());
    suggestions[idx].last_run_completed_at = None;
    suggestions[idx].last_run_error = None;
    suggestions[idx].accepted_trace_id = Some(trace_id.clone());
    suggestions[idx].accepted_goal_id = None;
    suggestions[idx].accepted_outcomes.clear();
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;
    let _ = trace::persist_live_trace_snapshot(&state.trace_history, &trace_ref).await;

    trace::spawn_live_trace_mirror(state.trace_history.clone(), trace_ref.clone());
    {
        let state_for_run = state.clone();
        let trace_ref_for_run = trace_ref.clone();
        let storage_for_run = storage.clone();
        let prompt_for_run = prompt.clone();
        let suggestion_id_for_run = suggestion_record_id.clone();
        let trace_id_for_run = trace_id.clone();
        let suggestion_kind_for_run = suggestion.kind.clone();
        let run_snapshot_for_run = run_snapshot;
        tokio::spawn(async move {
            let (token_tx, mut token_rx) =
                tokio::sync::mpsc::channel::<crate::core::StreamEvent>(256);
            let drain = tokio::spawn(async move { while token_rx.recv().await.is_some() {} });
            let run_result = {
                let agent = state_for_run.agent.read().await;
                agent
                    .process_message_stream_with_meta(
                        &prompt_for_run,
                        "autonomy",
                        None,
                        None,
                        trace_ref_for_run.clone(),
                        token_tx,
                    )
                    .await
            };
            let _ = drain.await;
            let snapshot = trace::persist_live_trace_snapshot(
                &state_for_run.trace_history,
                &trace_ref_for_run,
            )
            .await
            .unwrap_or_else(ExecutionTrace::default);
            let resolved_trace_id = if snapshot.id.trim().is_empty() {
                trace_id_for_run.clone()
            } else {
                snapshot.id.clone()
            };
            let outcomes = suggestions::collect_run_outcomes(
                &state_for_run,
                &run_snapshot_for_run,
                &suggestion_kind_for_run,
            )
            .await;
            let completed_at = chrono::Utc::now().to_rfc3339();
            match run_result {
                Ok(_) => {
                    suggestions::update_chat_suggestion_after_run(
                        &storage_for_run,
                        &suggestion_id_for_run,
                        &resolved_trace_id,
                        "completed",
                        &completed_at,
                        None,
                        outcomes,
                    )
                    .await;
                }
                Err(error) => {
                    let err_text = error.to_string();
                    suggestions::update_chat_suggestion_after_run(
                        &storage_for_run,
                        &suggestion_id_for_run,
                        &resolved_trace_id,
                        "failed",
                        &completed_at,
                        Some(err_text),
                        outcomes,
                    )
                    .await;
                }
            }
        });
    }

    Ok(serde_json::json!({
        "status": "started",
        "trace_id": trace_id.clone(),
        "trace_path": format!("/trace/{}", trace_id),
        "run": {
            "kind": "suggestion_execution",
            "title": suggestion.title,
            "status": "running",
            "started_at": started_at_text,
            "summary": format!(
                "Launched a real {} execution run. Open the live trace to watch steps, tool output, and any app build/runtime logs.",
                suggestion_kind_title(&suggestion.kind).to_ascii_lowercase()
            ),
            "trace_id": trace_id
        }
    }))
}

async fn dismiss_chat_suggestion(
    state: &AppState,
    suggestion_id: &str,
) -> Result<serde_json::Value, String> {
    let storage = { state.agent.read().await.storage.clone() };
    let mut suggestions = load_chat_suggestions(&storage).await;
    let Some(idx) = suggestions
        .iter()
        .position(|suggestion| suggestion.id == suggestion_id)
    else {
        return Err("Suggestion not found".to_string());
    };

    if suggestions[idx].status != "open" {
        return Err("Suggestion is no longer open".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    suggestions[idx].status = "dismissed".to_string();
    suggestions[idx].updated_at = now.clone();
    suggestions[idx].dismissed_at = Some(now);
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;

    Ok(serde_json::json!({ "status": "dismissed" }))
}

async fn build_autonomy_briefing(
    agent: &Agent,
    settings: &AutonomySettings,
) -> AutonomyBriefingResponse {
    let mut suggested_automations = load_chat_suggestions(&agent.storage).await;
    suggested_automations.retain(|suggestion| suggestion.status == "open");
    suggested_automations.sort_by(|a, b| {
        parse_rfc3339_utc(&b.updated_at)
            .cmp(&parse_rfc3339_utc(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    suggested_automations.truncate(6);
    let mut suggestion_scan = load_chat_suggestion_scan_state(&agent.storage).await;
    suggestion_scan.tracked_chats = suggestion_scan.conversation_watermarks.len();
    suggestion_scan.conversation_watermarks.clear();

    let (
        pending_tasks,
        awaiting_approval,
        paused_tasks,
        failed_tasks,
        in_progress_tasks,
        total_tasks,
    ) = {
        let tasks = agent.tasks.read().await;
        let total = tasks.all().len();
        let pending = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Pending))
            .count();
        let awaiting = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::AwaitingApproval))
            .count();
        let paused = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Paused))
            .count();
        let failed = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .count();
        let in_progress = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::InProgress))
            .count();
        (pending, awaiting, paused, failed, in_progress, total)
    };

    let unread_alerts = agent
        .storage
        .list_notifications(50, 0, true)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.level == "warning" || n.level == "error")
        .count();

    let security_logs = agent
        .storage
        .list_security_logs(80)
        .await
        .unwrap_or_default();
    let auth_failures = security_logs
        .iter()
        .filter(|s| s.event_type.to_ascii_lowercase().contains("auth"))
        .count();
    let security_spikes = security_logs
        .iter()
        .filter(|s| s.severity == "error" || s.severity == "critical")
        .count();

    let active_watchers = agent
        .watcher_manager
        .list()
        .await
        .into_iter()
        .filter(|w| matches!(w.status, crate::core::watcher::WatcherStatus::Active))
        .count();

    let completed_runs = {
        let trace = agent.trace_history.read().await;
        trace.iter().filter(|t| t.completed_at.is_some()).count()
    };

    let mut top_risks = Vec::new();
    if awaiting_approval > 0 {
        top_risks.push(serde_json::json!({
            "type": "approval_queue",
            "severity": "high",
            "title": "Approvals waiting",
            "detail": format!("{} task(s) are blocked pending approval", awaiting_approval),
        }));
    }
    if failed_tasks > 0 {
        top_risks.push(serde_json::json!({
            "type": "execution_failures",
            "severity": "high",
            "title": "Recent task failures",
            "detail": format!("{} failed task(s) need triage", failed_tasks),
        }));
    }
    if paused_tasks > 0 {
        top_risks.push(serde_json::json!({
            "type": "paused_tasks",
            "severity": "medium",
            "title": "Paused automations",
            "detail": format!("{} task(s) are paused and waiting to be resumed", paused_tasks),
        }));
    }
    if unread_alerts > 0 {
        top_risks.push(serde_json::json!({
            "type": "alerts",
            "severity": "medium",
            "title": "Unread alerts",
            "detail": format!("{} warning/error notification(s) are unread", unread_alerts),
        }));
    }
    if auth_failures > 0 {
        top_risks.push(serde_json::json!({
            "type": "auth_failures",
            "severity": "critical",
            "title": "Authentication pressure",
            "detail": format!("{} auth-related security events were logged recently", auth_failures),
        }));
    }
    let mut top_opportunities = Vec::new();
    if completed_runs > 0 {
        top_opportunities.push(serde_json::json!({
            "type": "throughput",
            "title": "Strong execution throughput",
            "detail": format!("{} run(s) completed recently - capture lessons into reusable routines", completed_runs),
        }));
    }
    if active_watchers > 0 {
        top_opportunities.push(serde_json::json!({
            "type": "automation",
            "title": "Automation already active",
            "detail": format!("{} watcher(s) are actively monitoring external conditions", active_watchers),
        }));
    }
    if pending_tasks == 0 && paused_tasks == 0 && in_progress_tasks == 0 {
        top_opportunities.push(serde_json::json!({
            "type": "capacity",
            "title": "High strategic capacity",
            "detail": "No active queue pressure - good window for high-leverage planning",
        }));
    }
    if !suggested_automations.is_empty() {
        top_opportunities.push(serde_json::json!({
            "type": "chat_suggestions",
            "title": "Uncaptured chat opportunities",
            "detail": format!(
                "{} suggestion draft(s) are waiting in Mission Control. Chat scan status: {}.",
                suggested_automations.len(),
                chat_suggestion_display_status(suggestion_scan.last_status.as_deref().unwrap_or("scheduled"))
            ),
        }));
    }
    if top_opportunities.is_empty() {
        top_opportunities.push(serde_json::json!({
            "type": "stability",
            "title": "Stable operating window",
            "detail": "Use this period to improve automation and documentation coverage",
        }));
    }

    let mut recommended_actions = Vec::new();
    if awaiting_approval > 0 {
        recommended_actions.push(recommendation(
            "Resolve Approval Queue",
            "Review blocked tasks and make explicit approve/reject decisions.",
            "chat_prompt",
            serde_json::json!({"prompt":"Show tasks awaiting approval with recommended decisions and expected impact."}),
            &settings.trust_policy,
        ));
    }
    if unread_alerts > 0 || security_spikes > 0 {
        recommended_actions.push(recommendation(
            "Enable Ops Mode",
            "Apply the Ops preset: create monitoring watchers and incident-focused routines, and make Ops the active autonomy mode.",
            "activate_mode",
            serde_json::json!({"mode_id":"ops"}),
            &settings.trust_policy,
        ));
    }
    recommended_actions.push(recommendation(
        "Send Daily Command Brief",
        "Generate today's executive brief and push it to your preferred channel.",
        "daily_brief_now",
        serde_json::json!({}),
        &settings.trust_policy,
    ));
    let swarm_ready = agent.swarm.is_some() && !agent.config.swarm.specialists.is_empty();
    if recommended_actions.len() < 3 && swarm_ready {
        recommended_actions.push(recommendation(
            "Delegate One Strategic Problem",
            "Use swarm delegation to split a complex objective into specialist outputs.",
            "delegate",
            serde_json::json!({
                "task":"Decompose my top objective into execution tracks with risks and first actions."
            }),
            &settings.trust_policy,
        ));
    }
    recommended_actions.truncate(3);

    AutonomyBriefingResponse {
        generated_at: chrono::Utc::now().to_rfc3339(),
        scope: settings.context_scope.as_storage_str().to_string(),
        top_risks,
        top_opportunities,
        trust_summary: serde_json::json!({
            "auto_execute_max_score": settings.trust_policy.auto_execute_max_score,
            "blocked_actions": settings.trust_policy.blocked_actions,
            "approval_actions": settings.trust_policy.always_require_approval_actions,
            "queue": {
                "pending_tasks": pending_tasks,
                "awaiting_approval": awaiting_approval,
                "paused_tasks": paused_tasks,
                "in_progress_tasks": in_progress_tasks,
                "total_tasks": total_tasks,
            }
        }),
        recommended_actions,
        suggested_automations,
        suggestion_scan,
    }
}

async fn run_recommended_action(
    agent: &Agent,
    settings: &mut AutonomySettings,
    action: &RecommendedAction,
    dry_run: bool,
) -> Result<serde_json::Value, String> {
    let trust = score_action_risk(&action.action_kind, &action.payload, &settings.trust_policy);
    if trust
        .reasons
        .iter()
        .any(|r| r.to_ascii_lowercase().contains("blocked"))
    {
        return Err("Skill blocked by trust policy".to_string());
    }

    if dry_run {
        return Ok(serde_json::json!({
            "dry_run": true,
            "action_id": action.id,
            "risk": { "level": risk_level_label(&trust.level), "score": trust.score, "requires_approval": trust.requires_approval, "reasons": trust.reasons },
        }));
    }

    if trust.requires_approval {
        let mut approval_task = Task::new(
            format!("Approval required: {}", action.title),
            "autonomy_action".to_string(),
            serde_json::json!({
                "autonomy_action_kind": action.action_kind.clone(),
                "autonomy_action_payload": action.payload.clone(),
                "_approval": {
                    "title": action.title.clone(),
                    "summary": action.description.clone(),
                    "reason": trust.reasons.join("; "),
                    "rule_name": "elevated_action_requires_explicit_approval",
                    "risk_level": risk_level_label(&trust.level),
                    "risk_score": trust.score,
                    "source": "autonomy"
                }
            }),
        );
        approval_task.approval = TaskApproval::RequireApproval;
        approval_task.status = TaskStatus::AwaitingApproval;
        let (task_id, reused_existing, removed_duplicates) = agent
            .add_or_update_similar_task(approval_task, false)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "status": "queued_for_approval",
            "action_id": action.id,
            "task_id": task_id,
            "reused_existing": reused_existing,
            "removed_duplicates": removed_duplicates,
            "risk": { "level": risk_level_label(&trust.level), "score": trust.score },
        }));
    }

    let result = agent
        .execute_autonomy_action_payload(settings, &action.action_kind, &action.payload)
        .await;
    if result.is_ok() {
        agent.record_self_tune_autonomous_success().await;
    }
    result
}

async fn start_codex_cli_oauth() -> Response {
    let runtime = codex_oauth_runtime();

    match spawn_codex_oauth_probe().await {
        Ok(()) => {
            let snapshot = runtime.read().await.clone();
            let auth_url = snapshot.auth_url.unwrap_or_default();
            let device_code = snapshot.device_code.unwrap_or_default();
            let message = if !auth_url.is_empty() && !device_code.is_empty() {
                format!(
                    "Open the URL below and enter code {}. After completion, click Check Status.",
                    device_code
                )
            } else {
                "OAuth flow started. Waiting for device code...".to_string()
            };

            let opened_browser = if !auth_url.is_empty() {
                open_url_in_default_browser(&auth_url).await.is_ok()
            } else {
                false
            };

            (
                StatusCode::OK,
                Json(CodexCliOAuthStartResponse {
                    started: true,
                    running: snapshot.active,
                    opened_browser,
                    auth_url,
                    device_code,
                    message,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::OK,
            Json(CodexCliOAuthStartResponse {
                started: false,
                running: false,
                opened_browser: false,
                auth_url: String::new(),
                device_code: String::new(),
                message: format!("OAuth failed: {}", e),
            }),
        )
            .into_response(),
    }
}

async fn codex_cli_oauth_status() -> Response {
    let has_api_key = read_codex_cli_api_key()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let runtime = codex_oauth_runtime();
    let snapshot = runtime.read().await.clone();
    let auth_url = snapshot.auth_url.unwrap_or_default();
    let device_code = snapshot.device_code.unwrap_or_default();

    let message = if has_api_key {
        "OpenAI Subscription connected and ready.".to_string()
    } else if snapshot.active {
        if !auth_url.is_empty() && !device_code.is_empty() {
            format!(
                "Waiting for OAuth completion. Open URL and enter code {}, then click Check Status again.",
                device_code
            )
        } else {
            "OAuth flow is running, waiting for authorization...".to_string()
        }
    } else if let Some(err) = &snapshot.last_error {
        format!("OAuth failed: {}", err)
    } else if !snapshot.last_output.is_empty() && snapshot.last_output.contains("successfully") {
        "OpenAI Subscription connected and ready.".to_string()
    } else {
        "OpenAI Subscription is not connected. Click 'Connect via Browser' to start OAuth."
            .to_string()
    };

    (
        StatusCode::OK,
        Json(CodexCliOAuthStatusResponse {
            connected: has_api_key,
            has_api_key,
            running: snapshot.active,
            auth_url,
            device_code,
            message,
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct SecretsVaultRevealRequest {}

#[derive(Debug, Deserialize)]
struct SecretsVaultUpsertRequest {
    key: String,
    value: String,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SecretsVaultDeleteRequest {
    key: String,
    #[serde(default)]
    password: Option<String>,
}

fn is_internal_secret_key(key: &str) -> bool {
    key.starts_with("integration_enabled:") || key.starts_with("action_envmap:")
}

fn is_valid_user_secret_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 160 {
        return false;
    }
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
}

fn mask_secret_value(value: &str) -> String {
    let len = value.chars().count();
    if len == 0 {
        return String::new();
    }
    if len <= 6 {
        return "*".repeat(len);
    }
    let prefix: String = value.chars().take(3).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(3)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}...{}", prefix, suffix)
}

fn settings_secret_source_for_custom_key(key: &str) -> &'static str {
    match key {
        "moltbook_api_key" => "moltbook",
        "search_serper_key" | "search_brave_key" => "search",
        crate::core::observability::OBSERVABILITY_AUTH_TOKEN_SECRET_KEY => "observability",
        _ => "custom",
    }
}

fn custom_settings_secret_is_deletable(_key: &str) -> bool {
    true
}

fn push_settings_secret_entry(
    entries: &mut Vec<serde_json::Value>,
    key: String,
    value: &str,
    source: &str,
    deletable: bool,
) {
    if value.trim().is_empty() {
        return;
    }
    entries.push(serde_json::json!({
        "key": key,
        "masked": mask_secret_value(value),
        "length": value.chars().count(),
        "source": source,
        "deletable": deletable,
    }));
}

fn collect_settings_secret_entries(
    secrets: &crate::core::config::Secrets,
) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();

    if let Some(value) = secrets.llm_api_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "llm_api_key".to_string(),
            value,
            "model-primary",
            false,
        );
    }
    if let Some(value) = secrets.llm_fallback_api_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "llm_fallback_api_key".to_string(),
            value,
            "model-fallback",
            false,
        );
    }
    if let Some(value) = secrets.telegram_bot_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "telegram_bot_token".to_string(),
            value,
            "telegram",
            false,
        );
    }
    if let Some(value) = secrets.whatsapp_access_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "whatsapp_access_token".to_string(),
            value,
            "whatsapp",
            false,
        );
    }
    if let Some(value) = secrets.tunnel_ngrok_authtoken.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "tunnel_ngrok_authtoken".to_string(),
            value,
            "tunnel",
            false,
        );
    }
    if let Some(value) = secrets.tunnel_tailscale_auth_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "tunnel_tailscale_auth_key".to_string(),
            value,
            "tunnel",
            false,
        );
    }
    if let Some(value) = secrets.api_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "http_api_key".to_string(),
            value,
            "api",
            false,
        );
    }

    let mut media_keys: Vec<_> = secrets.media_provider_keys.iter().collect();
    media_keys.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (provider, value) in media_keys {
        push_settings_secret_entry(
            &mut entries,
            format!("media_provider:{}", provider),
            value,
            "media",
            false,
        );
    }

    let mut model_keys: Vec<_> = secrets.model_pool_keys.iter().collect();
    model_keys.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (slot_id, value) in model_keys {
        push_settings_secret_entry(
            &mut entries,
            format!("model_slot:{}", slot_id),
            value,
            "model-slot",
            false,
        );
    }

    let mut mcp_auth: Vec<_> = secrets.mcp_auth.iter().collect();
    mcp_auth.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (server_id, auth) in mcp_auth {
        if let Some(token) = auth.token.as_deref() {
            push_settings_secret_entry(
                &mut entries,
                format!("mcp:{}:token", server_id),
                token,
                "mcp",
                false,
            );
        }
        if let Some(password) = auth.password.as_deref() {
            push_settings_secret_entry(
                &mut entries,
                format!("mcp:{}:password", server_id),
                password,
                "mcp",
                false,
            );
        }
    }

    let mut custom_entries: Vec<_> = secrets
        .custom
        .iter()
        .filter(|(key, _)| !is_internal_secret_key(key))
        .collect();
    custom_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in custom_entries {
        push_settings_secret_entry(
            &mut entries,
            key.clone(),
            value,
            settings_secret_source_for_custom_key(key),
            custom_settings_secret_is_deletable(key),
        );
    }

    entries.sort_by_key(|row| {
        row.get("key")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string()
    });
    entries
}

fn require_master_password_for_secrets(
    config_dir: &FsPath,
    data_dir: &FsPath,
    password: Option<&str>,
) -> std::result::Result<(), String> {
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(config_dir, data_dir);
    let custom_master_password_set =
        master_mgr.is_password_set() && !master_mgr.is_bootstrap_password_active().unwrap_or(false);
    if !custom_master_password_set {
        return Ok(());
    }
    let supplied = password.unwrap_or("").trim();
    if supplied.is_empty() {
        return Err("Master password is required.".to_string());
    }
    master_mgr
        .unlock(supplied)
        .map(|_| ())
        .map_err(|_| "Master password is incorrect.".to_string())
}

async fn list_settings_secrets(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };
    let secrets = match manager.load_secrets() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load encrypted secrets: {}", e),
                }),
            )
                .into_response();
        }
    };

    let entries = collect_settings_secret_entries(&secrets);

    let count = entries.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "entries": entries,
            "count": count
        })),
    )
        .into_response()
}

async fn reveal_settings_secrets(
    State(state): State<AppState>,
    Json(request): Json<SecretsVaultRevealRequest>,
) -> Response {
    let _ = state;
    let _ = request;
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "Full secret reveal is disabled. Secrets Vault only returns masked snippets."
                .to_string(),
        }),
    )
        .into_response()
}

async fn upsert_settings_secret(
    State(state): State<AppState>,
    Json(request): Json<SecretsVaultUpsertRequest>,
) -> Response {
    let key = request.key.trim();
    if !is_valid_user_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid key. Use letters, numbers, '_', '-', ':' or '.'.".to_string(),
            }),
        )
            .into_response();
    }
    if is_internal_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "This key is reserved for internal settings.".to_string(),
            }),
        )
            .into_response();
    }

    let value = request.value.trim();
    if value.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Value cannot be empty. Use delete to remove a secret.".to_string(),
            }),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    if let Err(msg) =
        require_master_password_for_secrets(&config_dir, &data_dir, request.password.as_deref())
    {
        return (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: msg })).into_response();
    }
    let manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = manager.set_custom_secret(key, Some(value.to_string())) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to store secret: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "key": key,
            "masked": mask_secret_value(value),
        })),
    )
        .into_response()
}

async fn delete_settings_secret(
    State(state): State<AppState>,
    Json(request): Json<SecretsVaultDeleteRequest>,
) -> Response {
    let key = request.key.trim();
    if !is_valid_user_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid key.".to_string(),
            }),
        )
            .into_response();
    }
    if is_internal_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "This key is reserved for internal settings.".to_string(),
            }),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    if let Err(msg) =
        require_master_password_for_secrets(&config_dir, &data_dir, request.password.as_deref())
    {
        return (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: msg })).into_response();
    }
    let manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = manager.set_custom_secret(key, None) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to delete secret: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "key": key,
            "deleted": true
        })),
    )
        .into_response()
}

/// Get the HTTP API key (masked + full for copying)
async fn get_api_key_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    match auth::sync_http_api_key_state(&state, true).await {
        Ok((Some(info), rotated)) => {
            let now = auth::unix_now_ts();
            let remaining_seconds = (info.expires_at - now).max(0);
            Json(serde_json::json!({
                "set": true,
                "masked": auth::mask_api_key_value(&info.key),
                "key": info.key,
                "issued_at_unix": info.issued_at,
                "expires_at_unix": info.expires_at,
                "ttl_seconds": crate::core::config::HTTP_API_KEY_TTL_SECS,
                "remaining_seconds": remaining_seconds,
                "rotated": rotated,
            }))
            .into_response()
        }
        Ok((None, _)) => Json(serde_json::json!({
            "set": false,
            "masked": null,
            "key": null,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "set": false,
                "masked": null,
                "key": null,
                "error": e,
            })),
        )
            .into_response(),
    }
}

/// Regenerate the HTTP API key
async fn regenerate_api_key_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let secure_config =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir));
    match secure_config.and_then(|sc| sc.regenerate_api_key_info()) {
        Ok(info) => {
            {
                let mut key_guard = state.api_key.write().await;
                *key_guard = Some(info.key.clone());
            }
            {
                let mut exp_guard = state.api_key_expires_at.write().await;
                *exp_guard = Some(info.expires_at);
            }
            {
                let mut agent = state.agent.write().await;
                agent.api_key = Some(info.key.clone());
            }
            let now = auth::unix_now_ts();
            let remaining_seconds = (info.expires_at - now).max(0);
            Json(serde_json::json!({
                "ok": true,
                "masked": auth::mask_api_key_value(&info.key),
                "key": info.key,
                "issued_at_unix": info.issued_at,
                "expires_at_unix": info.expires_at,
                "ttl_seconds": crate::core::config::HTTP_API_KEY_TTL_SECS,
                "remaining_seconds": remaining_seconds,
                "rotated": true,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            })),
        )
            .into_response(),
    }
}

fn resolve_project_root() -> PathBuf {
    let app_path = FsPath::new("/app");
    if app_path.join("Cargo.toml").exists() {
        return app_path.to_path_buf();
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            if dir.join("Cargo.toml").exists() {
                return dir.to_path_buf();
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
    }
    PathBuf::from(".")
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn compute_p95(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let idx = (((values.len() as f64) * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(values.len().saturating_sub(1));
    Some(values[idx])
}

async fn load_evolution_canary_state(
    storage: &crate::storage::Storage,
) -> Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> {
    let raw = storage
        .get(crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY)
        .await
        .ok()
        .flatten()?;
    serde_json::from_slice::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(&raw)
        .ok()
}

async fn load_last_self_evolve_result(
    storage: &crate::storage::Storage,
) -> Option<serde_json::Value> {
    let raw = storage
        .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_LAST_RESULT_KEY)
        .await
        .ok()
        .flatten()?;
    serde_json::from_slice::<serde_json::Value>(&raw).ok()
}

async fn load_deploy_guard_default(storage: &crate::storage::Storage) -> bool {
    storage
        .get(crate::core::self_evolve::strategy_runtime::APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|s| s.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

async fn build_evolution_settings_response(
    storage: &crate::storage::Storage,
) -> EvolutionSettingsResponse {
    let canary_state = load_evolution_canary_state(storage).await;
    let last_result = load_last_self_evolve_result(storage).await;

    let canary = if let Some(state) = canary_state.as_ref() {
        EvolutionCanarySummary {
            enabled: state.enabled,
            rollout_percent: state.rollout_percent,
            baseline_version: state.baseline_version.clone(),
            candidate_version: state.candidate_version.clone(),
        }
    } else {
        EvolutionCanarySummary {
            enabled: false,
            rollout_percent: 0,
            baseline_version: "routing-policy-default-v1".to_string(),
            candidate_version: "-".to_string(),
        }
    };

    let mut replay_gate_result: Option<String> = None;
    let mut promotion_mode = if canary.enabled {
        "canary".to_string()
    } else {
        "none".to_string()
    };
    let last_promotion_result = if let Some(obj) = last_result.as_ref().and_then(|v| v.as_object())
    {
        if let Some(mode) = obj.get("promotion_mode").and_then(|v| v.as_str()) {
            if !mode.trim().is_empty() {
                promotion_mode = mode.to_string();
            }
        }
        if let Some(replay) = obj.get("replay_evaluation").and_then(|v| v.as_object()) {
            if replay
                .get("promote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                replay_gate_result = Some("passed".to_string());
            } else if let Some(reason) = replay.get("reason").and_then(|v| v.as_str()) {
                replay_gate_result = Some(reason.to_string());
            }
        }
        let promoted = obj
            .get("promoted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let gate = obj
            .get("promotion_gate")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if promoted {
            "Promoted candidate policy".to_string()
        } else if !gate.trim().is_empty() {
            format!("Not promoted ({})", gate)
        } else {
            "Evolution completed".to_string()
        }
    } else {
        "No evolution runs yet".to_string()
    };

    EvolutionSettingsResponse {
        self_evolve_enabled: true,
        canary,
        last_promotion_result,
        replay_gate_result,
        promotion_mode,
        deploy_guard_default: load_deploy_guard_default(storage).await,
    }
}

fn aggregate_version_metrics(
    logs: &[crate::storage::entities::operational_log::Model],
    selector: impl Fn(&crate::storage::entities::operational_log::Model) -> Option<&str>,
) -> Vec<EvolutionVersionMetric> {
    let mut buckets: HashMap<String, Vec<&crate::storage::entities::operational_log::Model>> =
        HashMap::new();
    for row in logs {
        if row.event_type != "tool_call" {
            continue;
        }
        let Some(version) = selector(row).map(|v| v.trim()).filter(|v| !v.is_empty()) else {
            continue;
        };
        buckets.entry(version.to_string()).or_default().push(row);
    }

    let mut out = Vec::with_capacity(buckets.len());
    for (version, rows) in buckets {
        let samples = rows.len();
        if samples == 0 {
            continue;
        }
        let successes = rows.iter().filter(|row| row.success).count();
        let errors = samples.saturating_sub(successes);
        let latencies: Vec<i64> = rows.iter().filter_map(|row| row.latency_ms).collect();
        out.push(EvolutionVersionMetric {
            version,
            samples,
            success_rate: round4(successes as f64 / samples as f64),
            error_rate: round4(errors as f64 / samples as f64),
            p95_latency_ms: compute_p95(latencies),
        });
    }
    out.sort_by(|a, b| {
        b.samples
            .cmp(&a.samples)
            .then_with(|| a.version.cmp(&b.version))
    });
    out
}

async fn read_recent_lineage(limit: usize) -> Vec<serde_json::Value> {
    let path = resolve_project_root().join(ROUTING_POLICY_LINEAGE_REL_PATH);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let mut parsed = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            parsed.push(value);
        }
    }
    if parsed.len() <= limit {
        return parsed;
    }
    parsed.split_off(parsed.len().saturating_sub(limit))
}

async fn build_evolution_dev_response(
    storage: &crate::storage::Storage,
    limit: u64,
) -> EvolutionDevResponse {
    let logs = storage
        .list_operational_logs_by_event("tool_call", limit)
        .await
        .unwrap_or_default();
    let policy_metrics = aggregate_version_metrics(&logs, |row| row.policy_version.as_deref());
    let strategy_metrics = aggregate_version_metrics(&logs, |row| row.strategy_version.as_deref());
    EvolutionDevResponse {
        canary_state: load_evolution_canary_state(storage).await,
        last_result: load_last_self_evolve_result(storage).await,
        lineage_recent: read_recent_lineage(40).await,
        policy_metrics,
        strategy_metrics,
    }
}

async fn get_evolution_settings(State(state): State<AppState>) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    Json(build_evolution_settings_response(&storage).await).into_response()
}

async fn update_evolution_settings(
    State(state): State<AppState>,
    Json(request): Json<EvolutionSettingsUpdateRequest>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    if let Some(enabled) = request.deploy_guard_default {
        let raw = if enabled {
            b"true".as_slice()
        } else {
            b"false".as_slice()
        };
        if let Err(e) = storage
            .set(
                crate::core::self_evolve::strategy_runtime::APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY,
                raw,
            )
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update evolution settings: {}", e),
                }),
            )
                .into_response();
        }
    }
    Json(build_evolution_settings_response(&storage).await).into_response()
}

async fn get_evolution_dev(
    State(state): State<AppState>,
    Query(query): Query<EvolutionDevQuery>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let limit = query.limit.unwrap_or(5000).clamp(100, 100_000);
    Json(build_evolution_dev_response(&storage, limit).await).into_response()
}

async fn persist_evolution_action_trace(
    state: &AppState,
    action: &str,
    message: &str,
    detail_payload: serde_json::Value,
) -> Option<String> {
    let started_at = chrono::Utc::now();
    let trace_id = uuid::Uuid::new_v4().to_string();
    let detail_data = serde_json::to_string_pretty(&detail_payload).ok();
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
        id: trace_id.clone(),
        message: format!("Evolution action: {}", action),
        channel: "evolution".to_string(),
        started_at: Some(started_at),
        completed_at: Some(started_at),
        steps: vec![
            crate::core::ExecutionStep {
                icon: "[evolve]".to_string(),
                title: "Evolution Manual Action".to_string(),
                detail: "Applied a manual evolution control from the Evolution panel.".to_string(),
                step_type: "info".to_string(),
                data: Some(
                    serde_json::to_string_pretty(&serde_json::json!({
                        "trace_kind": "self_evolve.manual_action.request",
                        "action": action,
                        "message": message,
                    }))
                    .unwrap_or_default(),
                ),
                timestamp: started_at,
                duration_ms: Some(0),
            },
            crate::core::ExecutionStep {
                icon: "[ok]".to_string(),
                title: "Evolution Decision Applied".to_string(),
                detail: message.to_string(),
                step_type: "success".to_string(),
                data: detail_data,
                timestamp: started_at,
                duration_ms: Some(0),
            },
        ],
        proof_id: None,
        response: Some(message.to_string()),
        model: Some("internal:evolution".to_string()),
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost_usd: 0.0,
        complexity: Some("evolution".to_string()),
    }));

    let agent = state.agent.read().await;
    agent.persist_completed_trace(&trace_ref).await;
    Some(trace_id)
}

async fn run_evolution_dev_action(
    State(state): State<AppState>,
    Json(request): Json<EvolutionDevActionRequest>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let action = request.action.trim().to_ascii_lowercase();

    let message = match action.as_str() {
        "disable_canary" => {
            let mut canary = match load_evolution_canary_state(&storage).await {
                Some(state) => state,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "No canary state found.".to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            canary.enabled = false;
            let bytes = match serde_json::to_vec(&canary) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to serialize canary state: {}", e),
                        }),
                    )
                        .into_response();
                }
            };
            if let Err(e) = storage
                .set(
                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                    &bytes,
                )
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to disable canary: {}", e),
                    }),
                )
                    .into_response();
            }
            "Canary rollout disabled.".to_string()
        }
        "promote_candidate" => {
            let candidate_bytes = match storage
                .get(crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY)
                .await
            {
                Ok(Some(v)) => v,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "No candidate policy found to promote.".to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            if let Err(e) = storage
                .set(
                    crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY,
                    &candidate_bytes,
                )
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to promote candidate: {}", e),
                    }),
                )
                    .into_response();
            }
            if let Some(mut canary) = load_evolution_canary_state(&storage).await {
                canary.enabled = false;
                canary.baseline_version = canary.candidate_version.clone();
                if let Ok(bytes) = serde_json::to_vec(&canary) {
                    let _ = storage
                        .set(
                            crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                            &bytes,
                        )
                        .await;
                }
            }
            "Candidate policy promoted to baseline.".to_string()
        }
        "rollback_baseline" => {
            let snapshot = match storage
                .get(
                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY,
                )
                .await
            {
                Ok(Some(v)) => v,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "No baseline snapshot available for rollback.".to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            if let Err(e) = storage
                .set(
                    crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY,
                    &snapshot,
                )
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to rollback baseline policy: {}", e),
                    }),
                )
                    .into_response();
            }
            if let Some(mut canary) = load_evolution_canary_state(&storage).await {
                canary.enabled = false;
                if let Ok(bytes) = serde_json::to_vec(&canary) {
                    let _ = storage
                        .set(
                            crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                            &bytes,
                        )
                        .await;
                }
            }
            "Rolled back to the stored baseline snapshot.".to_string()
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Unsupported action. Use disable_canary, promote_candidate, or rollback_baseline."
                        .to_string(),
                }),
            )
                .into_response();
        }
    };

    let evolution = build_evolution_settings_response(&storage).await;
    let dev = build_evolution_dev_response(&storage, 5000).await;
    let trace_id = persist_evolution_action_trace(
        &state,
        &action,
        &message,
        serde_json::json!({
            "trace_kind": "self_evolve.manual_action.result",
            "action": action.clone(),
            "message": message.clone(),
            "self_evolve_enabled": evolution.self_evolve_enabled,
            "deploy_guard_default": evolution.deploy_guard_default,
            "canary_state": dev.canary_state.clone(),
            "last_result": dev.last_result.clone(),
        }),
    )
    .await;
    Json(serde_json::json!({
        "status": "ok",
        "message": message,
        "trace_id": trace_id,
        "evolution": evolution,
        "dev": dev
    }))
    .into_response()
}

/// Get current settings
async fn get_settings(State(state): State<AppState>) -> Json<SettingsResponse> {
    let (config, storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.config.clone(),
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    let profile = state.user_profile.read().await;
    let daily_brief_task = {
        let tasks = state.tasks.read().await;
        tasks
            .all()
            .iter()
            .find(|task| task.action == "daily_brief")
            .cloned()
    };
    let moltbook_settings = moltbook::load_moltbook_settings(&storage).await;
    let daily_brief_channel = match storage.get(DAILY_BRIEF_CHANNEL_KEY).await {
        Ok(Some(bytes)) => String::from_utf8(bytes).unwrap_or_else(|_| "telegram".to_string()),
        _ => "telegram".to_string(),
    };
    let stored_daily_brief_enabled =
        parse_bool_pref(storage.get(DAILY_BRIEF_ENABLED_KEY).await.ok().flatten());
    let stored_daily_brief_time = storage
        .get(DAILY_BRIEF_TIME_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| normalize_daily_brief_time(&value));
    let daily_brief_time = stored_daily_brief_time
        .or_else(|| {
            daily_brief_task
                .as_ref()
                .and_then(|task| task.cron.as_deref())
                .and_then(daily_brief_time_from_cron)
        })
        .unwrap_or_else(|| DEFAULT_DAILY_BRIEF_TIME.to_string());
    let daily_brief_enabled = stored_daily_brief_enabled || daily_brief_task.is_some();
    let moltbook_last_run_at = storage
        .get(moltbook::MOLTBOOK_LAST_RUN_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok());
    let moltbook_last_status = storage
        .get(moltbook::MOLTBOOK_LAST_STATUS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok());
    let search_cfg = tokio::fs::read_to_string(config_dir.join("search.toml"))
        .await
        .ok()
        .and_then(|c| toml::from_str::<crate::actions::SearchConfig>(&c).ok());

    // Primary LLM - has_key is true if a real api_key is set (not the placeholder)
    let (provider, model, base_url, has_key) = match &config.llm {
        LlmProvider::Ollama { base_url, model } => (
            "ollama".to_string(),
            model.clone(),
            Some(base_url.clone()),
            false,
        ),
        LlmProvider::Anthropic { api_key, model } => (
            "anthropic".to_string(),
            model.clone(),
            None,
            !api_key.is_empty() && api_key != "[ENCRYPTED]",
        ),
        LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } => {
            let provider = provider_label_for_openai(base_url);
            let display_base_url = match base_url.as_deref() {
                Some(url) if is_codex_cli_base_url(url) => None,
                _ => base_url.clone(),
            };
            (
                provider.to_string(),
                model.clone(),
                display_base_url,
                !api_key.is_empty() && api_key != "[ENCRYPTED]",
            )
        }
    };

    // Fallback LLM
    let (fallback_provider, fallback_model, fallback_base_url, has_fallback_key) =
        match &config.llm_fallback {
            Some(LlmProvider::Ollama { base_url, model }) => (
                Some("ollama".to_string()),
                Some(model.clone()),
                Some(base_url.clone()),
                false,
            ),
            Some(LlmProvider::Anthropic { api_key, model }) => (
                Some("anthropic".to_string()),
                Some(model.clone()),
                None,
                !api_key.is_empty() && api_key != "[ENCRYPTED]",
            ),
            Some(LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            }) => {
                let provider = provider_label_for_openai(base_url);
                let display_base_url = match base_url.as_deref() {
                    Some(url) if is_codex_cli_base_url(url) => None,
                    _ => base_url.clone(),
                };
                (
                    Some(provider.to_string()),
                    Some(model.clone()),
                    display_base_url,
                    !api_key.is_empty() && api_key != "[ENCRYPTED]",
                )
            }
            None => (None, None, None, false),
        };

    let telegram_last_chat_id = storage
        .get("telegram:last_chat_id")
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|chat_id| *chat_id != 0);
    let whatsapp_last_sender = storage
        .get("whatsapp:last_sender")
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let (telegram_enabled, telegram_users, has_telegram_token, telegram_delivery_ready) =
        match &config.telegram {
            Some(tg) => (
                true,
                tg.allowed_users.clone(),
                !tg.bot_token.is_empty() && tg.bot_token != "[ENCRYPTED]",
                !tg.bot_token.is_empty()
                    && tg.bot_token != "[ENCRYPTED]"
                    && (telegram_last_chat_id.is_some() || !tg.allowed_users.is_empty()),
            ),
            None => (false, vec![], false, false),
        };

    let (
        whatsapp_enabled,
        whatsapp_mode_str,
        whatsapp_phone_id,
        whatsapp_bridge,
        whatsapp_dm,
        whatsapp_numbers,
        has_whatsapp_token,
        whatsapp_delivery_ready,
    ) = match &config.whatsapp {
        Some(wa) => {
            let mode = match wa.mode {
                crate::channels::whatsapp::WhatsAppMode::Baileys => "baileys",
                crate::channels::whatsapp::WhatsAppMode::CloudApi => "cloud_api",
            };
            (
                true,
                mode.to_string(),
                wa.phone_number_id.clone(),
                wa.bridge_url.clone(),
                wa.dm_policy.clone(),
                wa.allowed_numbers.clone(),
                !wa.access_token.is_empty() && wa.access_token != "[ENCRYPTED]",
                !wa.access_token.is_empty()
                    && wa.access_token != "[ENCRYPTED]"
                    && whatsapp_last_sender.is_some(),
            )
        }
        None => (
            false,
            "baileys".to_string(),
            String::new(),
            "http://127.0.0.1:8999".to_string(),
            "pairing".to_string(),
            vec![],
            false,
            false,
        ),
    };

    // Settings are complete if name is set AND at least one LLM is configured
    // Check both legacy single-provider AND model pool (new way)
    let has_legacy_llm = !model.trim().is_empty()
        && match &config.llm {
            LlmProvider::Ollama { base_url, .. } => !base_url.trim().is_empty(),
            LlmProvider::Anthropic { .. } => has_key,
            LlmProvider::OpenAI { base_url, .. } => {
                has_key && (base_url.is_none() || !base_url.as_ref().unwrap().trim().is_empty())
            }
        };
    let has_model_pool = !config.model_pool.slots.is_empty();
    let settings_complete = !config.name.trim().is_empty() && (has_legacy_llm || has_model_pool);

    // Build model pool summary
    let model_pool_summary: Vec<ModelSlotSummary> = config
        .model_pool
        .slots
        .iter()
        .map(|slot| {
            let (prov, mdl, burl, has_key) = match &slot.provider {
                LlmProvider::Ollama { base_url, model } => (
                    "ollama".to_string(),
                    model.clone(),
                    Some(base_url.clone()),
                    false,
                ),
                LlmProvider::Anthropic { api_key, model } => (
                    "anthropic".to_string(),
                    model.clone(),
                    None,
                    !api_key.is_empty() && api_key != "[ENCRYPTED]",
                ),
                LlmProvider::OpenAI {
                    api_key,
                    model,
                    base_url,
                } => {
                    let p = provider_label_for_openai(base_url);
                    (
                        p.to_string(),
                        model.clone(),
                        base_url.clone(),
                        !api_key.is_empty() && api_key != "[ENCRYPTED]",
                    )
                }
            };
            let role_str = match &slot.role {
                ModelRole::Primary => "primary",
                ModelRole::Fast => "fast",
                ModelRole::Code => "code",
                ModelRole::Research => "research",
                ModelRole::Fallback => "fallback",
            };
            ModelSlotSummary {
                id: slot.id.clone(),
                label: slot.label.clone(),
                role: role_str.to_string(),
                provider: prov,
                model: mdl,
                base_url: burl,
                has_api_key: has_key,
                enabled: slot.enabled,
            }
        })
        .collect();

    Json(SettingsResponse {
        bot_name: config.name.clone(),
        personality: config.personality.clone(),
        timezone: profile.timezone.clone(),
        language: profile.language.clone(),
        tone: profile.tone.clone(),
        email_format: profile.email_format.clone(),
        daily_brief_enabled,
        daily_brief_time,
        daily_brief_channel,
        llm_provider: provider,
        llm_model: model,
        llm_base_url: base_url,
        has_api_key: has_key,
        llm_fallback_provider: fallback_provider,
        llm_fallback_model: fallback_model,
        llm_fallback_base_url: fallback_base_url,
        has_fallback_api_key: has_fallback_key,
        model_pool: model_pool_summary,
        smart_routing: config.model_pool.smart_routing,
        app_deploy_model_id: config.app_deploy_model_id.clone(),
        telegram_enabled,
        has_telegram_token,
        telegram_delivery_ready,
        telegram_allowed_users: telegram_users,
        whatsapp_enabled,
        whatsapp_mode: whatsapp_mode_str,
        has_whatsapp_token,
        whatsapp_delivery_ready,
        whatsapp_phone_number_id: whatsapp_phone_id,
        whatsapp_bridge_url: whatsapp_bridge,
        whatsapp_dm_policy: whatsapp_dm,
        whatsapp_allowed_numbers: whatsapp_numbers,
        auto_approve: config.auto_approve.clone(),
        search_primary: search_cfg
            .as_ref()
            .and_then(|c| c.primary.clone())
            .unwrap_or_else(|| "playwright".to_string()),
        search_fallback1: search_cfg
            .as_ref()
            .and_then(|c| c.fallback1.clone())
            .unwrap_or_else(|| "duckduckgo".to_string()),
        search_fallback2: search_cfg
            .as_ref()
            .and_then(|c| c.fallback2.clone())
            .unwrap_or_else(|| "none".to_string()),
        search_serper_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.serper.is_some())
            .unwrap_or(false),
        search_searxng_url: search_cfg
            .as_ref()
            .and_then(|cfg| match &cfg.searxng {
                Some(crate::actions::SearchBackend::SearXNG { base_url }) => Some(base_url),
                _ => None,
            })
            .cloned(),
        search_brave_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.brave.is_some())
            .unwrap_or(false),
        settings_complete,
        moltbook_enabled: moltbook_settings.enabled,
        moltbook_mode: moltbook_settings.mode,
        moltbook_sync_frequency: moltbook_settings.sync_frequency,
        moltbook_write_enabled: moltbook_settings.write_enabled,
        moltbook_defer_when_busy: moltbook_settings.defer_when_busy,
        moltbook_last_run_at,
        moltbook_last_status,
        tunnel_active: state.tunnel.read().await.active,
        deployment_mode: config.deployment_mode.as_str().to_string(),
        public_app_bind_addr: config.public_apps.bind_addr.clone(),
        public_app_base_url: config.public_apps.base_url.clone(),
        memory_retention_enabled: config.memory.retention_enabled,
        memory_retention_min_age_days: config.memory.retention_min_age_days,
        memory_retention_keep_last: config.memory.retention_keep_last,
        memory_retention_max_importance: config.memory.retention_max_importance,
        memory_retention_max_access_count: config.memory.retention_max_access_count,
        memory_retention_require_consolidated: config.memory.retention_require_consolidated,
        memory_retention_run_interval_days: config.memory.retention_run_interval_days,
        memory_retention_idle_threshold_secs: config.memory.retention_idle_threshold_secs,
        memory_retention_max_delete_per_run: config.memory.retention_max_delete_per_run,
        memory_retention_protect_fact_sources: config.memory.retention_protect_fact_sources,
        observability: observability::build_observability_settings_response(
            &config.observability,
            &config_dir,
            &data_dir,
        ),
    })
}

/// Get media generation settings (which providers are configured)
async fn get_media_settings(State(state): State<AppState>) -> Json<MediaSettingsResponse> {
    let agent = state.agent.read().await;

    // Check which media providers are configured (have API keys)
    let mut configured = Vec::new();
    for (provider, key) in &agent.config.media_gen.provider_api_keys {
        if !key.is_empty() && key != "[ENCRYPTED]" {
            configured.push(provider.clone());
        }
    }

    // Also check via integration (for runtime-configured providers)
    if let Some(media_gen) = agent.integrations.get("media_gen") {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            media_gen.execute("list_providers", &serde_json::json!({})),
        )
        .await
        {
            Ok(Ok(result)) => {
                if let Some(providers) = result.get("providers").and_then(|p| p.as_array()) {
                    for p in providers {
                        if p.get("configured")
                            .and_then(|c| c.as_bool())
                            .unwrap_or(false)
                        {
                            if let Some(name) = p.get("provider").and_then(|n| n.as_str()) {
                                if !configured.contains(&name.to_string()) {
                                    configured.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => tracing::warn!("media_gen list_providers failed: {}", e),
            Err(_) => tracing::warn!("media_gen list_providers timed out after 3s"),
        }
    }

    // Get default/fallback providers from config
    let media_config = &agent.config.media_gen;

    Json(MediaSettingsResponse {
        configured,
        default_image_provider: media_config.default_image_provider.clone(),
        image_model: media_config.image_model.clone(),
        fallback_image_provider: media_config.fallback_image_provider.clone(),
        default_video_provider: media_config.default_video_provider.clone(),
        fallback_video_provider: media_config.fallback_video_provider.clone(),
    })
}

/// Update settings
async fn update_settings(
    State(state): State<AppState>,
    Json(settings): Json<SettingsUpdate>,
) -> Response {
    let search_primary = settings.search_primary.clone();
    let search_fallback1 = settings.search_fallback1.clone();
    let search_fallback2 = settings.search_fallback2.clone();
    let search_serper_key = settings.search_serper_key.clone();
    let search_searxng_url = settings.search_searxng_url.clone();
    let search_brave_key = settings.search_brave_key.clone();
    let moltbook_api_key = settings.moltbook_api_key.clone();
    let observability_auth_token = settings
        .observability
        .as_ref()
        .and_then(|observability| observability.auth_token.clone());

    let mut needs_restart = false;
    let mut wa_start_bridge = false;
    let mut wa_stop_bridge = false;
    let mut wa_restart_bridge = false;
    let mut llm_connectivity_probe: Option<LlmProvider> = None;
    let mut media_provider_updates: Vec<(String, String)> = Vec::new();
    let deferred_storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let mut deferred_profile_bytes: Option<Vec<u8>> = None;
    let mut deferred_search_config_dir: Option<PathBuf> = None;
    let mut deferred_moltbook_settings: Option<MoltbookSettings> = None;
    let existing_daily_brief_tasks = {
        let tasks = state.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|task| task.action == "daily_brief")
            .cloned()
            .collect::<Vec<_>>()
    };
    let stored_daily_brief_enabled = parse_bool_pref(
        deferred_storage
            .get(DAILY_BRIEF_ENABLED_KEY)
            .await
            .ok()
            .flatten(),
    );
    let stored_daily_brief_time = deferred_storage
        .get(DAILY_BRIEF_TIME_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| normalize_daily_brief_time(&value))
        .or_else(|| {
            existing_daily_brief_tasks
                .first()
                .and_then(|task| task.cron.as_deref())
                .and_then(daily_brief_time_from_cron)
        })
        .unwrap_or_else(|| DEFAULT_DAILY_BRIEF_TIME.to_string());
    let stored_daily_brief_channel = deferred_storage
        .get(DAILY_BRIEF_CHANNEL_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| "telegram".to_string());
    let requested_daily_brief_time = if let Some(value) = settings.daily_brief_time.as_ref() {
        let Some(normalized) = normalize_daily_brief_time(value) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Daily brief time must use HH:MM in 24-hour format".to_string(),
                }),
            )
                .into_response();
        };
        normalized
    } else {
        stored_daily_brief_time
    };
    let requested_daily_brief_enabled = settings
        .daily_brief_enabled
        .unwrap_or(!existing_daily_brief_tasks.is_empty() || stored_daily_brief_enabled);

    if let Some(timezone) = settings.timezone.as_ref() {
        if !timezone.trim().is_empty() && timezone.parse::<chrono_tz::Tz>().is_err() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid timezone. Use an IANA name like America/New_York".to_string(),
                }),
            )
                .into_response();
        }
    }

    if settings.timezone.is_some()
        || settings.language.is_some()
        || settings.tone.is_some()
        || settings.email_format.is_some()
    {
        let mut profile = state.user_profile.write().await;
        if let Some(timezone) = &settings.timezone {
            if timezone.trim().is_empty() {
                profile.timezone = None;
            } else {
                profile.timezone = Some(timezone.clone());
            }
        }
        if let Some(language) = &settings.language {
            profile.language = if language.trim().is_empty() {
                None
            } else {
                Some(language.clone())
            };
        }
        if let Some(tone) = &settings.tone {
            profile.tone = if tone.trim().is_empty() {
                None
            } else {
                Some(tone.clone())
            };
        }
        if let Some(email_format) = &settings.email_format {
            profile.email_format = if email_format.trim().is_empty() {
                None
            } else {
                Some(email_format.clone())
            };
        }
        if let Ok(bytes) = serde_json::to_vec(&*profile) {
            deferred_profile_bytes = Some(bytes);
        }
    }

    let requested_daily_brief_channel = if let Some(channel) = settings.daily_brief_channel.as_ref()
    {
        let normalized = channel.trim().to_lowercase();
        if normalized != "telegram" && normalized != "whatsapp" && normalized != "email" {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Daily brief channel must be 'telegram', 'whatsapp', or 'email'"
                        .to_string(),
                }),
            )
                .into_response();
        }
        normalized
    } else {
        stored_daily_brief_channel
    };
    let deferred_daily_brief_channel = Some(requested_daily_brief_channel.clone());
    let deferred_daily_brief_enabled = Some(requested_daily_brief_enabled);
    let deferred_daily_brief_time = Some(requested_daily_brief_time.clone());

    if settings.moltbook_enabled.is_some()
        || settings.moltbook_mode.is_some()
        || settings.moltbook_sync_frequency.is_some()
        || settings.moltbook_write_enabled.is_some()
        || settings.moltbook_defer_when_busy.is_some()
    {
        let mut current = moltbook::load_moltbook_settings(&deferred_storage).await;
        if let Some(v) = settings.moltbook_enabled {
            current.enabled = v;
        }
        if let Some(v) = settings.moltbook_mode.as_ref() {
            current.mode = moltbook::normalize_moltbook_mode(v);
        }
        if let Some(v) = settings.moltbook_sync_frequency.as_ref() {
            current.sync_frequency = moltbook::normalize_moltbook_frequency(v);
        }
        if let Some(v) = settings.moltbook_write_enabled {
            current.write_enabled = v;
        }
        if let Some(v) = settings.moltbook_defer_when_busy {
            current.defer_when_busy = v;
        }
        deferred_moltbook_settings = Some(current);
    }

    let result = {
        let mut agent_guard = state.agent.write().await;

        // Snapshot current Telegram/WhatsApp config for change detection
        let old_telegram = agent_guard
            .config
            .telegram
            .as_ref()
            .map(|t| (t.bot_token.clone(), t.allowed_users.clone()));
        let old_whatsapp = agent_guard.config.whatsapp.as_ref().map(|w| {
            (
                w.mode.clone(),
                w.access_token.clone(),
                w.phone_number_id.clone(),
                w.bridge_url.clone(),
            )
        });

        // Update bot name if provided
        if let Some(name) = &settings.bot_name {
            if !name.is_empty() {
                agent_guard.config.name = name.clone();
            }
        }

        // Update personality if provided
        if let Some(personality) = &settings.personality {
            if !personality.is_empty() {
                agent_guard.config.personality = personality.clone();
            }
        }

        // Episodic memory retention settings (safe-by-default).
        if settings.memory_retention_enabled.is_some()
            || settings.memory_retention_min_age_days.is_some()
            || settings.memory_retention_keep_last.is_some()
            || settings.memory_retention_max_importance.is_some()
            || settings.memory_retention_max_access_count.is_some()
            || settings.memory_retention_require_consolidated.is_some()
            || settings.memory_retention_run_interval_days.is_some()
            || settings.memory_retention_idle_threshold_secs.is_some()
            || settings.memory_retention_max_delete_per_run.is_some()
            || settings.memory_retention_protect_fact_sources.is_some()
        {
            if let Some(v) = settings.memory_retention_enabled {
                agent_guard.config.memory.retention_enabled = v;
            }
            if let Some(v) = settings.memory_retention_min_age_days {
                // Keep a conservative floor.
                agent_guard.config.memory.retention_min_age_days = v.max(30);
            }
            if let Some(v) = settings.memory_retention_keep_last {
                // Always keep at least some recent episodes; never exceed max_episodes.
                let keep = v.max(500);
                agent_guard.config.memory.retention_keep_last =
                    keep.min(agent_guard.config.memory.max_episodes.max(1));
            }
            if let Some(v) = settings.memory_retention_max_importance {
                agent_guard.config.memory.retention_max_importance = v.clamp(0.0, 1.0);
            }
            if let Some(v) = settings.memory_retention_max_access_count {
                agent_guard.config.memory.retention_max_access_count = v.max(0);
            }
            if let Some(v) = settings.memory_retention_require_consolidated {
                agent_guard.config.memory.retention_require_consolidated = v;
            }
            if let Some(v) = settings.memory_retention_run_interval_days {
                agent_guard.config.memory.retention_run_interval_days = v.max(1);
            }
            if let Some(v) = settings.memory_retention_idle_threshold_secs {
                agent_guard.config.memory.retention_idle_threshold_secs = v.max(60);
            }
            if let Some(v) = settings.memory_retention_max_delete_per_run {
                agent_guard.config.memory.retention_max_delete_per_run = v.clamp(10, 20_000);
            }
            if let Some(v) = settings.memory_retention_protect_fact_sources {
                agent_guard.config.memory.retention_protect_fact_sources = v;
            }
        }

        // Get existing primary API key to preserve if not provided
        let existing_api_key_raw = match &agent_guard.config.llm {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            _ => None,
        };
        let mut existing_api_key = existing_api_key_raw.clone();

        // Get existing fallback API key to preserve if not provided
        let mut existing_fallback_api_key =
            agent_guard
                .config
                .llm_fallback
                .as_ref()
                .and_then(|fb| match fb {
                    LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
                    LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
                    _ => None,
                });
        if matches!(
            existing_api_key.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_fallback_api_key.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) {
            if let Ok(secure) = crate::core::config::SecureConfigManager::new_with_data_dir(
                &agent_guard.config_dir,
                Some(&agent_guard.data_dir),
            ) {
                if let Ok(secrets) = secure.load_secrets() {
                    if matches!(
                        existing_api_key.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_api_key = secrets.llm_api_key.clone();
                    }
                    if matches!(
                        existing_fallback_api_key.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_fallback_api_key = secrets.llm_fallback_api_key.clone();
                    }
                }
            }
        }

        let mut existing_telegram_token = agent_guard
            .config
            .telegram
            .as_ref()
            .map(|t| t.bot_token.clone());

        let mut existing_whatsapp_token = agent_guard
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.access_token.clone());

        if matches!(
            existing_telegram_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_whatsapp_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) {
            if let Ok(secure) = crate::core::config::SecureConfigManager::new_with_data_dir(
                &agent_guard.config_dir,
                Some(&agent_guard.data_dir),
            ) {
                if let Ok(secrets) = secure.load_secrets() {
                    if matches!(
                        existing_telegram_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_telegram_token = secrets.telegram_bot_token.clone();
                    }
                    if matches!(
                        existing_whatsapp_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_whatsapp_token = secrets.whatsapp_access_token.clone();
                    }
                }
            }
        }

        // Use new API key if provided, otherwise preserve existing (filter out "[ENCRYPTED]" placeholders)
        let new_api_key = settings
            .llm_api_key
            .clone()
            .filter(|k| !k.is_empty() && k != "[ENCRYPTED]");
        let api_key = new_api_key
            .clone()
            .or(existing_api_key.filter(|k| k != "[ENCRYPTED]"))
            .unwrap_or_default();

        // Fallback API key
        let fallback_api_key = settings
            .llm_fallback_api_key
            .clone()
            .filter(|k| !k.is_empty() && k != "[ENCRYPTED]")
            .or(existing_fallback_api_key.filter(|k| k != "[ENCRYPTED]"))
            .unwrap_or_default();

        // Handle empty base_url as None
        let base_url = settings.llm_base_url.clone().and_then(|u| {
            let trimmed = u.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        // Determine if the user actually changed LLM settings or is just saving other fields.
        // If the user didn't send a new API key AND a valid LLM config already exists in memory,
        // reuse the existing config so non-LLM saves (WhatsApp, Telegram, etc.) aren't blocked.
        // Also treat as unchanged if the Model Pool has a primary model (user manages LLM there).
        let has_model_pool_primary = agent_guard
            .config
            .model_pool
            .slots
            .iter()
            .any(|s| matches!(s.role, crate::core::config::ModelRole::Primary) && s.enabled);
        let llm_unchanged = new_api_key.is_none()
            && (has_model_pool_primary
                || (!matches!(agent_guard.config.llm, LlmProvider::Ollama { .. })
                    && !matches!(
                        existing_api_key_raw.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    )));

        let new_llm = if llm_unchanged {
            // Preserve current LLM config as-is - user didn't change it
            agent_guard.config.llm.clone()
        } else {
            // Validate LLM fields
            if settings.llm_model.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "LLM model is required".to_string(),
                    }),
                )
                    .into_response();
            }

            let mut api_key_for_provider = api_key.clone();
            if (settings.llm_provider.as_str() == "codex-cli"
                || settings.llm_provider.as_str() == "openai-subscription")
                && api_key_for_provider.trim().is_empty()
            {
                api_key_for_provider = read_codex_cli_api_key().unwrap_or_default();
            }
            if settings.llm_provider.as_str() != "ollama" && api_key_for_provider.trim().is_empty()
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "API key is required for the selected provider".to_string(),
                    }),
                )
                    .into_response();
            }

            if settings.llm_provider.as_str() == "ollama" {
                let url = base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                if url.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Ollama base URL is required".to_string(),
                        }),
                    )
                        .into_response();
                }
            }
            if settings.llm_provider.as_str() == "openai-compatible"
                && base_url.as_deref().unwrap_or("").trim().is_empty()
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Base URL is required for OpenAI-Compatible providers".to_string(),
                    }),
                )
                    .into_response();
            }
            let compat_base_url =
                match normalize_openai_base_url(settings.llm_provider.as_str(), base_url.clone()) {
                    Ok(url) => url,
                    Err(error) => {
                        return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                            .into_response();
                    }
                };

            // Build new LLM provider
            match settings.llm_provider.as_str() {
                "ollama" => LlmProvider::Ollama {
                    base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
                    model: settings.llm_model,
                },
                "anthropic" => LlmProvider::Anthropic {
                    api_key: api_key_for_provider.clone(),
                    model: settings.llm_model,
                },
                "openai" => LlmProvider::OpenAI {
                    api_key: api_key_for_provider.clone(),
                    model: settings.llm_model,
                    base_url: None,
                },
                "openai-compatible" | "openrouter" => LlmProvider::OpenAI {
                    api_key: api_key_for_provider.clone(),
                    model: settings.llm_model,
                    base_url: compat_base_url,
                },
                "codex-cli" | "openai-subscription" => LlmProvider::OpenAI {
                    api_key: api_key_for_provider.clone(),
                    model: settings.llm_model,
                    base_url: compat_base_url,
                },
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Unknown provider: {}", settings.llm_provider),
                        }),
                    )
                        .into_response();
                }
            }
        };

        // Build fallback LLM provider (optional)
        let fallback_base_url = settings.llm_fallback_base_url.clone().and_then(|u| {
            let trimmed = u.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let new_llm_fallback: Option<LlmProvider> = if let Some(fb_provider) =
            &settings.llm_fallback_provider
        {
            if !fb_provider.is_empty()
                && settings
                    .llm_fallback_model
                    .as_ref()
                    .map(|m| !m.is_empty())
                    .unwrap_or(false)
            {
                let fb_model = settings.llm_fallback_model.clone().unwrap_or_default();
                let fallback_compat_base_url = match normalize_openai_base_url(
                    fb_provider.as_str(),
                    fallback_base_url.clone(),
                ) {
                    Ok(url) => url,
                    Err(error) => {
                        return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                            .into_response();
                    }
                };
                match fb_provider.as_str() {
                    "ollama" => Some(LlmProvider::Ollama {
                        base_url: fallback_base_url
                            .unwrap_or_else(|| "http://localhost:11434".to_string()),
                        model: fb_model,
                    }),
                    "anthropic" => Some(LlmProvider::Anthropic {
                        api_key: fallback_api_key.clone(),
                        model: fb_model,
                    }),
                    "openai" => Some(LlmProvider::OpenAI {
                        api_key: fallback_api_key.clone(),
                        model: fb_model,
                        base_url: None,
                    }),
                    "openai-compatible" | "openrouter" | "codex-cli" | "openai-subscription" => {
                        Some(LlmProvider::OpenAI {
                            api_key: fallback_api_key.clone(),
                            model: fb_model,
                            base_url: fallback_compat_base_url,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        // Build telegram config
        let new_telegram = if settings.telegram_enabled {
            let token = settings
                .telegram_bot_token
                .clone()
                .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .or(existing_telegram_token.filter(|t| t != "[ENCRYPTED]"));

            if token.as_deref().unwrap_or("").trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Telegram bot token is required when Telegram is enabled"
                            .to_string(),
                    }),
                )
                    .into_response();
            }

            Some(TelegramConfig {
                bot_token: token.unwrap(),
                allowed_users: settings.telegram_allowed_users.clone().unwrap_or_default(),
                dm_policy: "pairing".to_string(),
            })
        } else {
            None
        };

        // Build WhatsApp config
        let new_whatsapp = if settings.whatsapp_enabled {
            let mode_str = settings.whatsapp_mode.as_deref().unwrap_or("baileys");
            let mode = match mode_str {
                "cloud_api" => crate::channels::whatsapp::WhatsAppMode::CloudApi,
                _ => crate::channels::whatsapp::WhatsAppMode::Baileys,
            };

            let token = settings
                .whatsapp_access_token
                .clone()
                .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .or(existing_whatsapp_token.filter(|t| t != "[ENCRYPTED]"))
                .unwrap_or_default();

            let phone_id = settings
                .whatsapp_phone_number_id
                .clone()
                .unwrap_or_default();

            let verify_tok = settings
                .whatsapp_verify_token
                .clone()
                .unwrap_or_else(|| "agentark_verify".to_string());

            let bridge_url = settings
                .whatsapp_bridge_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:8999".to_string());

            let dm_policy = settings
                .whatsapp_dm_policy
                .clone()
                .unwrap_or_else(|| "pairing".to_string());

            // Cloud API mode requires access token and phone number ID
            if mode == crate::channels::whatsapp::WhatsAppMode::CloudApi {
                if token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "WhatsApp access token is required for Cloud API mode"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if phone_id.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "WhatsApp Phone Number ID is required for Cloud API mode"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
            }

            Some(crate::channels::whatsapp::WhatsAppChannelConfig {
                mode,
                access_token: token,
                phone_number_id: phone_id,
                verify_token: verify_tok,
                bridge_url,
                allowed_numbers: settings
                    .whatsapp_allowed_numbers
                    .clone()
                    .unwrap_or_default(),
                dm_policy,
            })
        } else {
            None
        };

        // Defer network connectivity probing until after lock is released to avoid
        // blocking all agent reads/writes while waiting on upstream APIs.
        if !llm_unchanged {
            llm_connectivity_probe = Some(new_llm.clone());
        }

        // Update model pool routing behavior (doesn't require restart).
        if let Some(v) = settings.smart_routing {
            agent_guard.config.model_pool.smart_routing = v;
        }
        if let Some(v) = settings.app_deploy_model_id.as_ref() {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                agent_guard.config.app_deploy_model_id = None;
            } else {
                agent_guard.config.app_deploy_model_id = Some(trimmed.to_string());
            }
        }
        if let Some(mode) = settings.deployment_mode.as_ref() {
            let normalized = mode.trim().to_ascii_lowercase();
            let parsed_mode = match normalized.as_str() {
                "" | "trusted_local" | "trusted-local" => DeploymentMode::TrustedLocal,
                "internet_facing" | "internet-facing" => DeploymentMode::InternetFacing,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "deployment_mode must be 'trusted_local' or 'internet_facing'"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            if agent_guard.config.deployment_mode != parsed_mode {
                agent_guard.config.deployment_mode = parsed_mode;
                needs_restart = true;
            }
            if parsed_mode == DeploymentMode::InternetFacing
                && agent_guard.config.public_apps.bind_addr.is_none()
            {
                agent_guard.config.public_apps.bind_addr = Some("127.0.0.1:8992".to_string());
                needs_restart = true;
            }
        }
        if let Some(bind_addr) = settings.public_app_bind_addr.as_ref() {
            let normalized = bind_addr.trim();
            let next = if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            };
            if agent_guard.config.public_apps.bind_addr != next {
                agent_guard.config.public_apps.bind_addr = next;
                needs_restart = true;
            }
        }
        if let Some(base_url) = settings.public_app_base_url.as_ref() {
            let next = normalize_optional_url(Some(base_url.as_str()));
            if agent_guard.config.public_apps.base_url != next {
                agent_guard.config.public_apps.base_url = next;
                needs_restart = true;
            }
        }

        // Update config
        agent_guard.config.llm = new_llm.clone();
        agent_guard.config.llm_fallback = new_llm_fallback;
        agent_guard.config.telegram = new_telegram.clone();
        agent_guard.config.whatsapp = new_whatsapp.clone();

        // Detect if Telegram config changed (needs process restart)
        let new_tg_snapshot = new_telegram
            .as_ref()
            .map(|t| (t.bot_token.clone(), t.allowed_users.clone()));
        if old_telegram != new_tg_snapshot {
            needs_restart = true;
        }

        // Detect WhatsApp config change (managed via bridge process, no full restart needed)
        let wa_was_enabled = old_whatsapp.is_some();
        let wa_is_enabled = new_whatsapp.is_some();
        if !wa_was_enabled && wa_is_enabled {
            wa_start_bridge = true; // User just enabled WhatsApp
        } else if wa_was_enabled && !wa_is_enabled {
            wa_stop_bridge = true; // User just disabled WhatsApp
        } else if wa_was_enabled && wa_is_enabled {
            // Check if config changed (bridge URL, mode, etc.) - needs restart
            let new_wa_snapshot = new_whatsapp.as_ref().map(|w| {
                (
                    w.mode.clone(),
                    w.access_token.clone(),
                    w.phone_number_id.clone(),
                    w.bridge_url.clone(),
                )
            });
            if old_whatsapp != new_wa_snapshot {
                wa_restart_bridge = true;
            }
        }

        // Update auto_approve list (with validation - dangerous actions are rejected)
        if let Some(ref list) = settings.auto_approve {
            let (allowed, rejected) = crate::core::config::AgentConfig::validate_auto_approve(list);
            if !rejected.is_empty() {
                tracing::warn!("Rejected auto-approve entries: {:?}", rejected);
            }
            agent_guard.config.auto_approve = allowed;
        }

        // Save media provider API keys to config (they will be encrypted by SecureConfigManager)
        for (provider, key) in &settings.media_providers {
            if !key.is_empty() && key != "[ENCRYPTED]" {
                agent_guard
                    .config
                    .media_gen
                    .provider_api_keys
                    .insert(provider.clone(), key.clone());
                media_provider_updates.push((provider.clone(), key.clone()));
            }
        }

        // Update default/fallback media providers
        if let Some(ref provider) = settings.default_image_provider {
            agent_guard.config.media_gen.default_image_provider = Some(provider.clone());
        }
        if let Some(ref model) = settings.image_model {
            agent_guard.config.media_gen.image_model = Some(model.clone());
        }
        if let Some(ref provider) = settings.fallback_image_provider {
            agent_guard.config.media_gen.fallback_image_provider = Some(provider.clone());
        }
        if let Some(ref provider) = settings.default_video_provider {
            agent_guard.config.media_gen.default_video_provider = Some(provider.clone());
        }
        if let Some(ref provider) = settings.fallback_video_provider {
            agent_guard.config.media_gen.fallback_video_provider = Some(provider.clone());
        }

        if let Some(observability) = settings.observability.as_ref() {
            if let Some(enabled) = observability.enabled {
                agent_guard.config.observability.enabled = enabled;
            }
            if let Some(provider) = observability.provider.as_ref() {
                agent_guard.config.observability.provider =
                    crate::core::observability::normalize_observability_provider(provider);
            }
            if let Some(endpoint) = observability.endpoint.as_ref() {
                agent_guard.config.observability.endpoint = endpoint.trim().to_string();
            }
            if let Some(service_name) = observability.service_name.as_ref() {
                agent_guard.config.observability.service_name = service_name.trim().to_string();
            }
            if let Some(header_name) = observability.header_name.as_ref() {
                agent_guard.config.observability.header_name =
                    crate::core::observability::normalize_observability_header_name(header_name);
            }
            if let Some(privacy_mode) = observability.privacy_mode.as_ref() {
                agent_guard.config.observability.privacy_mode =
                    crate::core::observability::normalize_observability_privacy_mode(privacy_mode);
            }
        }

        // Runtime media provider syncing is done after lock release.

        if search_primary.is_some()
            || search_fallback1.is_some()
            || search_fallback2.is_some()
            || search_serper_key.is_some()
            || search_searxng_url.is_some()
            || search_brave_key.is_some()
        {
            deferred_search_config_dir = Some(agent_guard.config_dir.clone());
        }

        // Save to disk
        let mut save_result = agent_guard
            .config
            .save(&agent_guard.config_dir, Some(&agent_guard.data_dir));

        if save_result.is_ok()
            && (observability_auth_token.is_some()
                || moltbook_api_key.is_some()
                || search_serper_key.is_some()
                || search_brave_key.is_some())
        {
            let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
                &agent_guard.config_dir,
                Some(&agent_guard.data_dir),
            );
            save_result = manager.and_then(|manager| {
                manager.update_custom_secrets(|custom| {
                    if let Some(auth_token) = observability_auth_token.as_ref() {
                        if auth_token.trim().is_empty() {
                            custom.remove(
                                crate::core::observability::OBSERVABILITY_AUTH_TOKEN_SECRET_KEY,
                            );
                        } else {
                            custom.insert(
                                crate::core::observability::OBSERVABILITY_AUTH_TOKEN_SECRET_KEY
                                    .to_string(),
                                auth_token.trim().to_string(),
                            );
                        }
                    }
                    if let Some(api_key) = moltbook_api_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("moltbook_api_key");
                        } else {
                            custom
                                .insert("moltbook_api_key".to_string(), api_key.trim().to_string());
                        }
                    }
                    if let Some(api_key) = search_serper_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_serper_key");
                        } else {
                            custom.insert(
                                "search_serper_key".to_string(),
                                api_key.trim().to_string(),
                            );
                        }
                    }
                    if let Some(api_key) = search_brave_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_brave_key");
                        } else {
                            custom
                                .insert("search_brave_key".to_string(), api_key.trim().to_string());
                        }
                    }
                    Ok(())
                })
            });
        }

        // Reinitialize LLM client (skip if unchanged / managed by model pool)
        if !llm_unchanged {
            match crate::core::LlmClient::new(&new_llm) {
                Ok(new_client) => {
                    agent_guard.llm = new_client;
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to initialize LLM: {}", e),
                        }),
                    )
                        .into_response();
                }
            }
        }

        save_result
    };

    if let Some(bytes) = deferred_profile_bytes {
        if let Err(e) = deferred_storage.set("user_profile", &bytes).await {
            tracing::warn!("Failed to persist user profile updates: {}", e);
        }
    }
    if let Some(channel) = deferred_daily_brief_channel.as_ref() {
        if let Err(e) = deferred_storage
            .set(DAILY_BRIEF_CHANNEL_KEY, channel.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist daily brief channel: {}", e);
        }
    }
    if let Some(enabled) = deferred_daily_brief_enabled {
        let stored_value = if enabled { "true" } else { "false" };
        if let Err(e) = deferred_storage
            .set(DAILY_BRIEF_ENABLED_KEY, stored_value.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist daily brief enabled flag: {}", e);
        }
    }
    if let Some(time_value) = deferred_daily_brief_time.as_ref() {
        if let Err(e) = deferred_storage
            .set(DAILY_BRIEF_TIME_KEY, time_value.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist daily brief time: {}", e);
        }
    }

    if let Some(config_dir) = deferred_search_config_dir.as_ref() {
        let mut search_config = tokio::fs::read_to_string(config_dir.join("search.toml"))
            .await
            .ok()
            .and_then(|c| toml::from_str::<crate::actions::SearchConfig>(&c).ok())
            .unwrap_or_default();

        if let Some(url) = &search_searxng_url {
            search_config.searxng = Some(crate::actions::SearchBackend::SearXNG {
                base_url: url.clone(),
            });
        }
        if let Some(key) = &search_serper_key {
            search_config.serper = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Serper {
                    api_key: String::new(),
                })
            };
        }
        if let Some(key) = &search_brave_key {
            search_config.brave = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Brave {
                    api_key: String::new(),
                })
            };
        }

        let all_backends = [
            search_primary.as_deref(),
            search_fallback1.as_deref(),
            search_fallback2.as_deref(),
        ];
        if all_backends.iter().any(|b| b == &Some("playwright"))
            && search_config.playwright.is_none()
        {
            let bridge_url = std::env::var("PLAYWRIGHT_BRIDGE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:3100".to_string());
            search_config.playwright =
                Some(crate::actions::SearchBackend::Playwright { bridge_url });
        }

        search_config.primary = search_primary.clone();
        search_config.fallback1 = search_fallback1.clone();
        search_config.fallback2 = search_fallback2.clone();

        let search_path = config_dir.join("search.toml");
        if let Ok(content) = toml::to_string_pretty(&search_config) {
            if let Err(e) = tokio::fs::write(&search_path, content).await {
                tracing::warn!("Failed to save search config: {}", e);
            }
        }
    }

    if let Some(moltbook_cfg) = deferred_moltbook_settings.as_ref() {
        if let Err(e) = moltbook::save_moltbook_settings(&deferred_storage, moltbook_cfg).await {
            tracing::warn!("Failed to persist Moltbook settings: {}", e);
        } else {
            if moltbook_cfg.enabled {
                let bootstrap_next = chrono::Utc::now() + chrono::Duration::minutes(5);
                let _ = deferred_storage
                    .set(
                        moltbook::MOLTBOOK_NEXT_RUN_KEY,
                        bootstrap_next.to_rfc3339().as_bytes(),
                    )
                    .await;
            } else {
                let _ = deferred_storage
                    .delete(moltbook::MOLTBOOK_NEXT_RUN_KEY)
                    .await;
            }
            moltbook::append_moltbook_activity(
                &deferred_storage,
                "settings",
                "info",
                "settings_updated",
                serde_json::json!({
                    "enabled": moltbook_cfg.enabled,
                    "mode": moltbook_cfg.mode,
                    "sync_frequency": moltbook_cfg.sync_frequency,
                    "write_enabled": moltbook_cfg.write_enabled,
                    "defer_when_busy": moltbook_cfg.defer_when_busy
                }),
            )
            .await;
        }
    }

    match result {
        Ok(_) => {
            if !media_provider_updates.is_empty() {
                let agent_ref = state.agent.clone();
                let updates = media_provider_updates.clone();
                tokio::spawn(async move {
                    let agent = agent_ref.read().await;
                    if let Some(media_gen) = agent.integrations.get("media_gen") {
                        for (provider, api_key) in updates {
                            let payload = serde_json::json!({
                                "provider": provider,
                                "api_key": api_key
                            });
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(3),
                                media_gen.execute("configure_provider", &payload),
                            )
                            .await
                            {
                                Ok(Ok(_)) => {}
                                Ok(Err(e)) => tracing::warn!(
                                    "Failed to sync media provider config to runtime: {}",
                                    e
                                ),
                                Err(_) => tracing::warn!(
                                    "Timed out syncing media provider config to runtime"
                                ),
                            }
                        }
                    }
                });
            }

            if let Some(provider) = llm_connectivity_probe {
                tokio::spawn(async move {
                    if let Err(e) = test_llm_connection(&provider).await {
                        tracing::warn!("LLM provider connectivity probe failed after save: {}", e);
                    }
                });
            }

            for task in &existing_daily_brief_tasks {
                if let Err(e) = deferred_storage.delete_task(&task.id.to_string()).await {
                    tracing::warn!(
                        "Failed to delete previous daily brief task {}: {}",
                        task.id,
                        e
                    );
                }
            }
            {
                let mut queue = state.tasks.write().await;
                for task in &existing_daily_brief_tasks {
                    queue.remove(task.id);
                }
            }
            if requested_daily_brief_enabled {
                let Some(daily_brief_cron) =
                    daily_brief_cron_from_time(&requested_daily_brief_time)
                else {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to build the daily brief schedule".to_string(),
                        }),
                    )
                        .into_response();
                };
                let mut task = Task::new(
                    "Morning summary brief".to_string(),
                    "daily_brief".to_string(),
                    serde_json::json!({ "report_to": requested_daily_brief_channel.clone() }),
                );
                task.capabilities = vec!["daily_brief".to_string()];
                task.cron = Some(daily_brief_cron);
                if let Err(e) = deferred_storage.insert_task(&task).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to save daily brief schedule: {}", e),
                        }),
                    )
                        .into_response();
                }
                let mut queue = state.tasks.write().await;
                queue.add(task);
            }

            // Handle WhatsApp bridge lifecycle (no full process restart needed)
            if wa_start_bridge || wa_restart_bridge {
                let wb = state.whatsapp_bridge.clone();
                tokio::spawn(async move {
                    if wa_restart_bridge {
                        stop_whatsapp_bridge(wb.clone()).await;
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    tracing::info!("Starting WhatsApp bridge (user enabled in settings)...");
                    match spawn_whatsapp_bridge(wb).await {
                        Ok(()) => tracing::info!("WhatsApp bridge started successfully"),
                        Err(e) => tracing::error!("Failed to start WhatsApp bridge: {}", e),
                    }
                });
            } else if wa_stop_bridge {
                let wb = state.whatsapp_bridge.clone();
                tokio::spawn(async move {
                    tracing::info!("Stopping WhatsApp bridge (user disabled in settings)...");
                    stop_whatsapp_bridge(wb).await;
                });
            }

            if needs_restart {
                tracing::info!("Telegram config changed - scheduling automatic restart in 2s");
                tokio::spawn(async {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    tracing::info!("Restarting process to apply Telegram config changes...");
                    std::process::exit(0); // Docker restart: unless-stopped will bring us back
                });
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "message": "Settings saved. Restarting to apply channel changes...",
                        "restart_scheduled": true
                    })),
                )
                    .into_response()
            } else {
                let msg = if wa_start_bridge {
                    "Settings saved. WhatsApp bridge starting..."
                } else if wa_stop_bridge {
                    "Settings saved. WhatsApp bridge stopped."
                } else if wa_restart_bridge {
                    "Settings saved. WhatsApp bridge restarting..."
                } else {
                    "Settings saved"
                };
                (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "ok", "message": msg})),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save settings: {}", e),
            }),
        )
            .into_response(),
    }
}

async fn test_llm_connection(provider: &LlmProvider) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    match provider {
        LlmProvider::Ollama { base_url, .. } => {
            let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
            let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("Ollama returned {}", resp.status()))
            }
        }
        LlmProvider::OpenAI {
            api_key, base_url, ..
        } => {
            let base = effective_openai_base_url(base_url.as_deref()).trim_end_matches('/');
            let url = format!("{}/models", base);
            let resp = client
                .get(url)
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                let label = provider_label_for_openai(base_url);
                Err(format!("{} returned {}", label, resp.status()))
            }
        }
        LlmProvider::Anthropic { api_key, .. } => {
            let url = "https://api.anthropic.com/v1/models";
            let resp = client
                .get(url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("Anthropic returned {}", resp.status()))
            }
        }
    }
}

/// Restart the server (Docker will auto-restart due to restart policy)
async fn restart_server() -> Response {
    tracing::info!("Restart requested via API - shutting down for restart");

    // Spawn a task to exit after a short delay (allows response to be sent)
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        std::process::exit(0);
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Server is restarting..."
        })),
    )
        .into_response()
}

// ============================================================================
// OAuth & Integrations
// ============================================================================

/// Shared form field metadata for integration and tunnel settings.
#[derive(Debug, Serialize)]
pub struct IntegrationConfigField {
    pub key: String,
    pub label: String,
    /// "text" | "password" | "textarea" | "select"
    pub input_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

// ==================== SSH API ====================

async fn ssh_list_connections(State(state): State<AppState>) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::ssh_list_connections(&config_dir).await {
        Ok(text) => (
            StatusCode::OK,
            Json(serde_json::json!({ "connections": text })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ssh_add_connection(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };

    let name = match request.get("name").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing connection name".to_string(),
                }),
            )
                .into_response()
        }
    };
    let host = match request.get("host").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing host".to_string(),
                }),
            )
                .into_response()
        }
    };
    let port = request.get("port").and_then(|v| v.as_u64()).unwrap_or(22) as u16;
    let username = match request.get("username").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing username".to_string(),
                }),
            )
                .into_response()
        }
    };
    let key_name = match request.get("key_name").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing key_name".to_string(),
                }),
            )
                .into_response()
        }
    };

    let conn = crate::actions::ssh::SshConnection {
        name: name.clone(),
        host,
        port,
        username,
        key_name,
    };

    match crate::actions::ssh::add_connection(&config_dir, conn) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "name": name })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ssh_remove_connection(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::remove_connection(&config_dir, &name) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Connection '{}' not found", name),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ssh_list_keys(State(state): State<AppState>) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::list_key_names(&config_dir) {
        Ok(keys) => (StatusCode::OK, Json(serde_json::json!({ "keys": keys }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ssh_upload_key(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };

    let name = match request.get("name").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing key name".to_string(),
                }),
            )
                .into_response()
        }
    };
    let pem_content = match request.get("pem_content").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing pem_content".to_string(),
                }),
            )
                .into_response()
        }
    };

    match crate::actions::ssh::store_key(&config_dir, &name, &pem_content) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "name": name })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ssh_remove_key(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::remove_key(&config_dir, &name) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Key '{}' not found", name),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn ssh_test_connection(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    let args = serde_json::json!({
        "connection": request.get("connection").and_then(|v| v.as_str()).unwrap_or(""),
        "command": "echo ok"
    });
    match crate::actions::ssh::ssh_execute(&config_dir, &args).await {
        Ok(output) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "output": output })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Model Pool API ====================

/// List all model pool slots
async fn list_models(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let slots: Vec<ModelSlotSummary> = agent
        .config
        .model_pool
        .slots
        .iter()
        .map(|slot| {
            let (prov, mdl, burl, has_key) = match &slot.provider {
                LlmProvider::Ollama { base_url, model } => (
                    "ollama".to_string(),
                    model.clone(),
                    Some(base_url.clone()),
                    false,
                ),
                LlmProvider::Anthropic { api_key, model } => (
                    "anthropic".to_string(),
                    model.clone(),
                    None,
                    !api_key.is_empty(),
                ),
                LlmProvider::OpenAI {
                    api_key,
                    model,
                    base_url,
                } => {
                    let p = provider_label_for_openai(base_url);
                    let display_base_url = match base_url.as_deref() {
                        Some(url) if is_codex_cli_base_url(url) => None,
                        _ => base_url.clone(),
                    };
                    (
                        p.to_string(),
                        model.clone(),
                        display_base_url,
                        !api_key.is_empty(),
                    )
                }
            };
            let role_str = match &slot.role {
                ModelRole::Primary => "primary",
                ModelRole::Fast => "fast",
                ModelRole::Code => "code",
                ModelRole::Research => "research",
                ModelRole::Fallback => "fallback",
            };
            ModelSlotSummary {
                id: slot.id.clone(),
                label: slot.label.clone(),
                role: role_str.to_string(),
                provider: prov,
                model: mdl,
                base_url: burl,
                has_api_key: has_key,
                enabled: slot.enabled,
            }
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": slots,
            "smart_routing": agent.config.model_pool.smart_routing,
        })),
    )
        .into_response()
}

/// Discover available models from a provider API
async fn discover_provider_models(
    Path(provider): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let models: Vec<serde_json::Value> = match provider.as_str() {
        "openai" | "openai-subscription" => {
            let api_key = if provider == "openai-subscription" {
                read_codex_cli_api_key().unwrap_or_default()
            } else {
                params.get("api_key").cloned().unwrap_or_default()
            };
            if api_key.is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "No API key available" })),
                )
                    .into_response();
            }
            let base = params
                .get("base_url")
                .map(|s| s.as_str())
                .unwrap_or("https://api.openai.com/v1");
            let resp = client
                .get(format!("{}/models", base))
                .header("Authorization", format!("Bearer {}", api_key))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    let mut ids: Vec<String> = body["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    // Filter to chat-completions-capable models
                    // Exclude codex models (Responses API only, not Chat Completions)
                    ids.retain(|id| {
                        (id.starts_with("gpt-")
                            || id.starts_with("o1")
                            || id.starts_with("o3")
                            || id.starts_with("o4")
                            || id.starts_with("chatgpt-"))
                            && !id.contains("codex")
                            && !id.contains("realtime")
                            && !id.contains("audio")
                            && !id.contains("tts")
                            && !id.contains("whisper")
                            && !id.contains("dall-e")
                            && !id.contains("embedding")
                            && !id.contains("moderation")
                            && !id.ends_with("-instruct")
                    });
                    ids.sort();
                    ids.dedup();
                    // Sort: prefer newer/larger models first
                    ids.sort_by(|a, b| {
                        let rank = |s: &str| -> u8 {
                            if s.contains("5.2") {
                                0
                            } else if s.contains("5.1") {
                                1
                            } else if s.starts_with("gpt-5") && !s.contains('.') {
                                2
                            } else if s.starts_with("o4") {
                                3
                            } else if s.starts_with("o3") {
                                4
                            } else if s.contains("4.1") {
                                5
                            } else if s.contains("4o") {
                                6
                            } else {
                                10
                            }
                        };
                        rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
                    });
                    ids.into_iter()
                        .map(|id| serde_json::json!({ "id": id }))
                        .collect()
                }
                Ok(r) => {
                    let status = r.status().as_u16();
                    let text = r.text().await.unwrap_or_default();
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({ "error": format!("Provider returned {}: {}", status, text) })),
                    )
                        .into_response();
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({ "error": format!("Failed to reach provider: {}", e) })),
                    )
                        .into_response();
                }
            }
        }
        "anthropic" => {
            let api_key = params.get("api_key").cloned().unwrap_or_default();
            if api_key.is_empty() {
                // Return well-known Anthropic models as fallback
                let known = vec![
                    "claude-opus-4-20250514",
                    "claude-sonnet-4-20250514",
                    "claude-3-7-sonnet-latest",
                    "claude-3-5-haiku-latest",
                ];
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "models": known.into_iter().map(|id| serde_json::json!({ "id": id })).collect::<Vec<_>>()
                    })),
                )
                    .into_response();
            }
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    let mut ids: Vec<String> = body["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    ids.sort();
                    ids.into_iter()
                        .map(|id| serde_json::json!({ "id": id }))
                        .collect()
                }
                _ => {
                    // Fallback to well-known
                    vec![
                        "claude-opus-4-20250514",
                        "claude-sonnet-4-20250514",
                        "claude-3-7-sonnet-latest",
                        "claude-3-5-haiku-latest",
                    ]
                    .into_iter()
                    .map(|id| serde_json::json!({ "id": id }))
                    .collect()
                }
            }
        }
        "ollama" => {
            let base = params
                .get("base_url")
                .map(|s| s.as_str())
                .unwrap_or("http://localhost:11434");
            let resp = client.get(format!("{}/api/tags", base)).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    body["models"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m["name"].as_str().map(|s| serde_json::json!({ "id": s }))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => vec![],
            }
        }
        "openrouter" => {
            let resp = client
                .get("https://openrouter.ai/api/v1/models")
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    body["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m["id"].as_str().map(
                                        |id| serde_json::json!({ "id": id, "name": m["name"] }),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => vec![],
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Unknown provider: {}", provider) })),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({ "models": models })),
    )
        .into_response()
}

/// Add a new model slot
async fn add_model(
    State(state): State<AppState>,
    Json(request): Json<ModelSlotRequest>,
) -> Response {
    let result = {
        let mut agent = state.agent.write().await;

        let role = match request.role.as_str() {
            "primary" => ModelRole::Primary,
            "fast" => ModelRole::Fast,
            "code" => ModelRole::Code,
            "research" => ModelRole::Research,
            "fallback" => ModelRole::Fallback,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Unknown role: {}", request.role),
                    }),
                )
                    .into_response();
            }
        };

        let provider = match provider_from_model_slot_request(&request, None) {
            Ok(provider) => provider,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };

        // Generate unique ID
        let slot_id = format!(
            "{}_{}",
            request.role,
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("x")
        );

        let slot = ModelSlot {
            id: slot_id.clone(),
            label: request.label.clone(),
            role,
            provider: provider.clone(),
            enabled: request.enabled.unwrap_or(true),
        };

        agent.config.model_pool.slots.push(slot.clone());

        // Create LlmClient and add to runtime pool
        match crate::core::LlmClient::new(&provider) {
            Ok(client) => {
                agent.model_pool.insert(slot_id.clone(), (slot, client));
                // Update primary_model_id if this is a primary and none exists
                if request.role == "primary" && agent.primary_model_id.is_empty() {
                    agent.primary_model_id = slot_id.clone();
                }
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to initialize model: {}", e),
                    }),
                )
                    .into_response();
            }
        }

        // Also keep legacy llm in sync if this is primary
        if request.role == "primary" {
            agent.config.llm = provider.clone();
            if let Ok(client) = crate::core::LlmClient::new(&provider) {
                agent.llm = client;
            }
        }

        let provider_for_probe = provider.clone();
        agent
            .config
            .save(&agent.config_dir, Some(&agent.data_dir))
            .map(|_| provider_for_probe)
    };

    match result {
        Ok(provider_for_probe) => {
            // Push LLM config to Mem0 sidecar in background
            let agent = state.agent.read().await;
            if let Some((slot, _)) = agent.model_pool.values().next() {
                let provider = slot.provider.clone();
                let mem0 = agent.mem0.clone();
                tokio::spawn(async move {
                    if let Err(e) = mem0.configure(&provider).await {
                        tracing::warn!("Mem0 configure after model add failed: {}", e);
                    } else if let Err(e) = mem0.warmup().await {
                        tracing::warn!("Mem0 warmup after model add failed: {}", e);
                    }
                });
            }
            drop(agent);
            let connectivity = match test_llm_connection(&provider_for_probe).await {
                Ok(_) => serde_json::json!({ "ok": true }),
                Err(error) => serde_json::json!({ "ok": false, "error": error }),
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Model added",
                    "connectivity": connectivity
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Update a model slot
async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ModelSlotRequest>,
) -> Response {
    let result = {
        let mut agent = state.agent.write().await;

        let slot_idx = agent
            .config
            .model_pool
            .slots
            .iter()
            .position(|s| s.id == id);
        let Some(idx) = slot_idx else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Model slot not found".to_string(),
                }),
            )
                .into_response();
        };

        let role = match request.role.as_str() {
            "primary" => ModelRole::Primary,
            "fast" => ModelRole::Fast,
            "code" => ModelRole::Code,
            "research" => ModelRole::Research,
            "fallback" => ModelRole::Fallback,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Unknown role: {}", request.role),
                    }),
                )
                    .into_response();
            }
        };

        // Preserve existing API key if not provided.
        // If in-memory slot key is placeholder, recover the real value from encrypted secrets.
        let mut existing_key = match &agent.config.model_pool.slots[idx].provider {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            _ => None,
        };
        if matches!(
            existing_key.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) {
            if let Ok(secure) = crate::core::config::SecureConfigManager::new_with_data_dir(
                &agent.config_dir,
                Some(&agent.data_dir),
            ) {
                if let Ok(secrets) = secure.load_secrets() {
                    existing_key = secrets.model_pool_keys.get(&id).cloned();
                }
            }
        }
        let provider = match provider_from_model_slot_request(&request, existing_key) {
            Ok(provider) => provider,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };

        let enabled = request
            .enabled
            .unwrap_or(agent.config.model_pool.slots[idx].enabled);

        let slot = ModelSlot {
            id: id.clone(),
            label: request.label.clone(),
            role,
            provider: provider.clone(),
            enabled,
        };

        agent.config.model_pool.slots[idx] = slot.clone();

        // Update runtime pool
        agent.model_pool.remove(&id);
        if enabled {
            if let Ok(client) = crate::core::LlmClient::new(&provider) {
                agent.model_pool.insert(id.clone(), (slot, client));
            }
        }

        // Keep legacy llm in sync
        if request.role == "primary" {
            agent.config.llm = provider.clone();
            if let Ok(client) = crate::core::LlmClient::new(&provider) {
                agent.llm = client;
            }
        }

        let provider_for_probe = provider.clone();
        agent
            .config
            .save(&agent.config_dir, Some(&agent.data_dir))
            .map(|_| provider_for_probe)
    };

    match result {
        Ok(provider_for_probe) => {
            // Push updated LLM config to Mem0 sidecar in background
            let agent = state.agent.read().await;
            if let Some((slot, _)) = agent.model_pool.values().next() {
                let provider = slot.provider.clone();
                let mem0 = agent.mem0.clone();
                tokio::spawn(async move {
                    if let Err(e) = mem0.configure(&provider).await {
                        tracing::warn!("Mem0 configure after model update failed: {}", e);
                    } else if let Err(e) = mem0.warmup().await {
                        tracing::warn!("Mem0 warmup after model update failed: {}", e);
                    }
                });
            }
            drop(agent);
            let connectivity = match test_llm_connection(&provider_for_probe).await {
                Ok(_) => serde_json::json!({ "ok": true }),
                Err(error) => serde_json::json!({ "ok": false, "error": error }),
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Model updated",
                    "connectivity": connectivity
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Delete a model slot
async fn delete_model(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let result = {
        let mut agent = state.agent.write().await;

        let slot_idx = agent
            .config
            .model_pool
            .slots
            .iter()
            .position(|s| s.id == id);
        let Some(idx) = slot_idx else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Model slot not found".to_string(),
                }),
            )
                .into_response();
        };

        agent.config.model_pool.slots.remove(idx);
        agent.model_pool.remove(&id);
        if agent
            .config
            .app_deploy_model_id
            .as_ref()
            .is_some_and(|slot_id| slot_id == &id)
        {
            agent.config.app_deploy_model_id = None;
        }

        // If we removed the primary, promote the next one
        if agent.primary_model_id == id {
            agent.primary_model_id = agent.model_pool.keys().next().cloned().unwrap_or_default();
        }

        agent.config.save(&agent.config_dir, Some(&agent.data_dir))
    };

    match result {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Model removed"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

// ==================== Swarm API ====================

/// Swarm status overview
async fn swarm_status(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;

    if let Some(ref swarm) = agent.swarm {
        let status = swarm.status().await;
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "enabled": status.enabled,
                "total_agents": status.total_agents,
                "active_agents": status.active_agents,
                "agents": status.agents,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "enabled": true,
                "total_agents": 0,
                "active_agents": 0,
                "agents": [],
            })),
        )
            .into_response()
    }
}

/// List swarm agents (from DB for persistent view)
async fn swarm_list_agents(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let live_status = if let Some(ref swarm) = agent.swarm {
        Some(swarm.status().await)
    } else {
        None
    };

    match agent.storage.get_swarm_agents().await {
        Ok(agents) => {
            let live_by_id: std::collections::HashMap<
                String,
                crate::core::swarm::agent_trait::AgentInfo,
            > = live_status
                .as_ref()
                .map(|status| {
                    status
                        .agents
                        .iter()
                        .cloned()
                        .map(|info| (info.id.to_string(), info))
                        .collect()
                })
                .unwrap_or_default();
            let agent_infos: Vec<serde_json::Value> = agents
                .iter()
                .map(|a| {
                    let provider = crate::core::swarm::persistence::parse_llm_provider(
                        &a.llm_provider,
                        &agent.config.llm,
                    );
                    let provider_label = match &provider {
                        LlmProvider::Anthropic { .. } => "anthropic",
                        LlmProvider::OpenAI { base_url, .. } => {
                            let base = base_url.as_deref().unwrap_or_default();
                            if base.eq_ignore_ascii_case("codex://cli") {
                                "openai-subscription"
                            } else if base.contains("openrouter.ai") {
                                "openrouter"
                            } else if base.trim().is_empty() {
                                "openai"
                            } else {
                                "openai-compatible"
                            }
                        }
                        LlmProvider::Ollama { .. } => "ollama",
                    };
                    let llm_model = match &provider {
                        LlmProvider::Anthropic { model, .. } => model.clone(),
                        LlmProvider::OpenAI { model, .. } => model.clone(),
                        LlmProvider::Ollama { model, .. } => model.clone(),
                    };
                    let llm_base_url = match &provider {
                        LlmProvider::Anthropic { .. } => None,
                        LlmProvider::OpenAI { base_url, .. } => base_url.clone(),
                        LlmProvider::Ollama { base_url, .. } => Some(base_url.clone()),
                    };
                    let live = live_by_id.get(&a.id);
                    serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "agent_type": a.agent_type,
                        "llm_provider": provider_label,
                        "llm_model": live.map(|info| info.llm_model.clone()).unwrap_or(llm_model),
                        "llm_base_url": llm_base_url,
                        "capabilities": crate::core::swarm::persistence::parse_capabilities(&a.capabilities)
                            .into_iter()
                            .map(|cap| cap.description)
                            .collect::<Vec<_>>(),
                        "system_prompt": a.system_prompt,
                        "enabled": a.enabled == 1,
                        "status": live
                            .map(|info| format!("{:?}", info.status))
                            .unwrap_or_else(|| "Idle".to_string()),
                        "created_at": a.created_at,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "agents": agent_infos })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Add a new swarm agent request
#[derive(Debug, Deserialize)]
pub struct AddSwarmAgentRequest {
    pub name: String,
    pub agent_type: String,
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn build_swarm_agent_spec(
    id: Option<String>,
    request: &AddSwarmAgentRequest,
    existing_provider: Option<&LlmProvider>,
) -> std::result::Result<(LlmProvider, crate::core::swarm::SpecialistConfig), String> {
    let swarm_base_url =
        normalize_openai_base_url(request.llm_provider.as_str(), request.llm_base_url.clone())?;
    let requested_api_key = request.llm_api_key.clone().unwrap_or_default();
    let preserved_api_key = existing_provider
        .and_then(|provider| match provider {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            LlmProvider::Ollama { .. } => None,
        })
        .unwrap_or_default();
    let effective_api_key = if requested_api_key.trim().is_empty() {
        preserved_api_key
    } else {
        requested_api_key
    };

    let llm_provider = match request.llm_provider.as_str() {
        "anthropic" => LlmProvider::Anthropic {
            api_key: effective_api_key,
            model: request.llm_model.clone(),
        },
        "openai" | "openai-compatible" | "openrouter" | "codex-cli" | "openai-subscription" => {
            LlmProvider::OpenAI {
                api_key: effective_api_key,
                model: request.llm_model.clone(),
                base_url: if request.llm_provider == "openai" {
                    None
                } else {
                    swarm_base_url
                },
            }
        }
        _ => LlmProvider::Ollama {
            base_url: request
                .llm_base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            model: request.llm_model.clone(),
        },
    };

    let specialist_config = crate::core::swarm::SpecialistConfig {
        id,
        name: request.name.clone(),
        agent_type: crate::core::swarm::persistence::parse_agent_type(
            &request.agent_type,
            request.system_prompt.as_deref(),
        ),
        llm_provider: llm_provider.clone(),
        system_prompt_override: request.system_prompt.clone(),
        max_memory_retrieval: 3,
        capabilities: crate::core::swarm::persistence::capability_strings_to_models(
            &request.capabilities,
        ),
        enabled: true,
    };

    Ok((llm_provider, specialist_config))
}

/// Add a specialist agent to the swarm
async fn swarm_add_agent(
    State(state): State<AppState>,
    Json(request): Json<AddSwarmAgentRequest>,
) -> Response {
    let agent_id = uuid::Uuid::new_v4().to_string();
    let (llm_provider, specialist_config) =
        match build_swarm_agent_spec(Some(agent_id.clone()), &request, None) {
            Ok(spec) => spec,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };

    let mut agent = state.agent.write().await;
    let db_agent = crate::storage::entities::swarm_agent::Model {
        id: agent_id.clone(),
        name: request.name.clone(),
        agent_type: request.agent_type.clone(),
        llm_provider: serde_json::to_string(&llm_provider).unwrap_or_default(),
        capabilities: serde_json::to_string(
            &crate::core::swarm::persistence::capability_models_to_strings(
                &specialist_config.capabilities,
            ),
        )
        .unwrap_or_else(|_| "[]".to_string()),
        system_prompt: request.system_prompt.clone(),
        enabled: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    if let Err(e) = agent.storage.insert_swarm_agent(&db_agent).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save agent: {}", e),
            }),
        )
            .into_response();
    }

    let response = if let Some(ref swarm) = agent.swarm {
        match swarm
            .add_specialist(specialist_config.clone(), vec![])
            .await
        {
            Ok(id) => serde_json::json!({
                "status": "ok",
                "agent_id": id.to_string(),
                "message": format!("Agent '{}' added to swarm", request.name),
            }),
            Err(e) => serde_json::json!({
                "status": "ok",
                "agent_id": agent_id,
                "message": format!("Agent '{}' saved but swarm add failed: {}. Will be loaded on restart.", request.name, e),
            }),
        }
    } else {
        serde_json::json!({
            "status": "ok",
            "agent_id": agent_id,
            "message": format!("Agent '{}' saved. Swarm will activate it on next initialization.", request.name),
        })
    };

    if let Some(idx) = agent
        .config
        .swarm
        .specialists
        .iter()
        .position(|item| item.id.as_deref() == Some(agent_id.as_str()))
    {
        agent.config.swarm.specialists[idx] = specialist_config;
    } else {
        agent.config.swarm.specialists.push(specialist_config);
    }
    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save swarm config: {}", e),
            }),
        )
            .into_response();
    }

    (StatusCode::OK, Json(response)).into_response()
}

/// Update a specialist agent
async fn swarm_update_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AddSwarmAgentRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let existing = match agent.storage.get_swarm_agents().await {
        Ok(items) => match items.into_iter().find(|item| item.id == id) {
            Some(model) => model,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Agent not found".to_string(),
                    }),
                )
                    .into_response();
            }
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load agent: {}", e),
                }),
            )
                .into_response();
        }
    };
    let existing_provider = crate::core::swarm::persistence::parse_llm_provider(
        &existing.llm_provider,
        &agent.config.llm,
    );
    let (llm_provider, specialist_config) =
        match build_swarm_agent_spec(Some(id.clone()), &request, Some(&existing_provider)) {
            Ok(spec) => spec,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };

    let db_agent = crate::storage::entities::swarm_agent::Model {
        id: existing.id.clone(),
        name: request.name.clone(),
        agent_type: request.agent_type.clone(),
        llm_provider: serde_json::to_string(&llm_provider).unwrap_or_default(),
        capabilities: serde_json::to_string(
            &crate::core::swarm::persistence::capability_models_to_strings(
                &specialist_config.capabilities,
            ),
        )
        .unwrap_or_else(|_| "[]".to_string()),
        system_prompt: request.system_prompt.clone(),
        enabled: 1,
        created_at: existing.created_at.clone(),
    };

    if let Err(e) = agent.storage.update_swarm_agent(&db_agent).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to update agent: {}", e),
            }),
        )
            .into_response();
    }

    if let Some(ref swarm) = agent.swarm {
        let live_id = crate::core::swarm::AgentId(id.clone());
        let _ = swarm.remove_specialist(&live_id).await;
        if let Err(e) = swarm
            .add_specialist(specialist_config.clone(), vec![])
            .await
        {
            tracing::warn!("Failed to re-register updated swarm agent '{}': {}", id, e);
        }
    }

    if let Some(idx) = agent
        .config
        .swarm
        .specialists
        .iter()
        .position(|item| item.id.as_deref() == Some(id.as_str()))
    {
        agent.config.swarm.specialists[idx] = specialist_config;
    } else {
        agent.config.swarm.specialists.push(specialist_config);
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save swarm config: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "agent_id": id,
            "message": "Agent updated",
        })),
    )
        .into_response()
}

/// Remove a swarm agent
async fn swarm_remove_agent(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mut agent = state.agent.write().await;

    // Remove from DB
    if let Err(e) = agent.storage.delete_swarm_agent(&id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to delete: {}", e),
            }),
        )
            .into_response();
    }

    // Remove from live swarm
    if let Some(ref swarm) = agent.swarm {
        let agent_id = crate::core::swarm::AgentId(id.clone());
        let _ = swarm.remove_specialist(&agent_id).await;
    }

    agent.config.swarm.specialists.retain(|item| {
        item.id
            .as_deref()
            .map(|value| value != id.as_str())
            .unwrap_or(true)
    });
    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save swarm config: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Agent removed",
        })),
    )
        .into_response()
}

/// Get swarm config
async fn swarm_get_config(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let config = &agent.config.swarm;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "enabled": true,
            "max_specialists": config.max_specialists,
            "default_timeout_secs": config.default_timeout_secs,
        })),
    )
        .into_response()
}

/// Update swarm config request
#[derive(Debug, Deserialize)]
pub struct UpdateSwarmConfigRequest {
    pub max_specialists: Option<usize>,
    pub default_timeout_secs: Option<u64>,
}

/// Update swarm config
async fn swarm_update_config(
    State(state): State<AppState>,
    Json(request): Json<UpdateSwarmConfigRequest>,
) -> Response {
    let mut agent = state.agent.write().await;

    if let Some(max) = request.max_specialists {
        agent.config.swarm.max_specialists = max;
    }
    if let Some(timeout) = request.default_timeout_secs {
        agent.config.swarm.default_timeout_secs = timeout;
    }

    // Save config
    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Swarm config updated. Restart to apply changes.",
            "enabled": true,
        })),
    )
        .into_response()
}

/// List recent swarm delegations
async fn swarm_list_delegations(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let agent = state.agent.read().await;
    let limit_param = params.get("limit").map(|value| value.trim().to_string());
    let delegations = match limit_param.as_deref() {
        Some("all") => agent.storage.get_all_delegations().await,
        _ => {
            let limit = limit_param
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(50)
                .clamp(1, 200);
            agent.storage.get_recent_delegations(limit).await
        }
    };
    match delegations {
        Ok(delegations) => {
            let items: Vec<serde_json::Value> = delegations
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "agent_id": d.agent_id,
                        "task": d.task_description,
                        "success": d.success == 1,
                        "confidence": d.confidence,
                        "execution_time_ms": d.execution_time_ms,
                        "result": d.result,
                        "created_at": d.created_at,
                        "completed_at": d.completed_at,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "delegations": items })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Conversation Endpoints ====================

async fn list_conversations(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let agent = state.agent.read().await;
    let project_id = params.get("project_id").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let total = agent
        .storage
        .count_conversations(project_id)
        .await
        .unwrap_or(0);
    match agent
        .storage
        .list_conversations(limit, offset, project_id)
        .await
    {
        Ok(convs) => {
            // Filter out internal/system conversations (arkpulse, sentinel, etc.)
            let internal_channels = ["arkpulse", "sentinel", "system"];
            let list: Vec<serde_json::Value> = convs
                .iter()
                .filter(|c| !internal_channels.contains(&c.channel.as_str()))
                .map(|c| {
                    serde_json::json!({
                        "id": c.id, "title": c.title, "channel": c.channel,
                        "project_id": c.project_id, "created_at": c.created_at,
                        "updated_at": c.updated_at, "message_count": c.message_count,
                        "archived": c.archived,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"conversations": list, "total": total, "limit": limit, "offset": offset}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn create_conversation_endpoint(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let title = request
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("New Chat")
        .to_string();
    let channel = request
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("web")
        .to_string();
    let project_id = request
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    let conv = crate::storage::entities::conversation::Model {
        id: id.clone(),
        title,
        channel,
        project_id,
        created_at: now.clone(),
        updated_at: now,
        message_count: 0,
        archived: false,
    };

    let agent = state.agent.read().await;
    match agent.storage.create_conversation(&conv).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "status": "ok"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn get_conversation_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.get_conversation(&id).await {
        Ok(Some(conv)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": conv.id, "title": conv.title, "channel": conv.channel,
                "project_id": conv.project_id, "created_at": conv.created_at,
                "updated_at": conv.updated_at, "message_count": conv.message_count,
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Conversation not found".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn update_conversation_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let title = body.get("title").and_then(|v| v.as_str());
    if title.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Missing title".to_string(),
            }),
        )
            .into_response();
    }
    let agent = state.agent.read().await;
    match agent.storage.update_conversation(&id, title, None).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "title": title})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn delete_conversation_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_conversation(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn get_conversation_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    match agent
        .encrypted_storage
        .get_messages_decrypted(&id, limit, offset)
        .await
    {
        Ok(msgs) => {
            let list: Vec<serde_json::Value> = msgs.iter().map(|m| serde_json::json!({
                "id": m.id, "role": m.role, "content": m.content,
                "timestamp": m.timestamp, "model_used": m.model_used, "trace_id": m.trace_id,
            })).collect();
            (StatusCode::OK, Json(serde_json::json!({"messages": list}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Project Endpoints ====================

async fn list_projects_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.list_projects().await {
        Ok(projects) => {
            let list: Vec<serde_json::Value> = projects
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id, "name": p.name, "description": p.description,
                        "system_prompt": p.system_prompt, "personality": p.personality,
                        "tools_filter": p.tools_filter, "active": p.active,
                        "created_at": p.created_at, "updated_at": p.updated_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"projects": list}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn create_project_endpoint(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let name = match request.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Name required".to_string(),
                }),
            )
                .into_response()
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let proj = crate::storage::entities::project::Model {
        id: id.clone(),
        name,
        description: request
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        system_prompt: request
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        personality: request
            .get("personality")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        tools_filter: request
            .get("tools_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        active: true,
        created_at: now.clone(),
        updated_at: now,
    };
    let agent = state.agent.read().await;
    match agent.storage.create_project(&proj).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "status": "ok"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn get_project_endpoint(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.get_project(&id).await {
        Ok(Some(p)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": p.id, "name": p.name, "description": p.description,
                "system_prompt": p.system_prompt, "personality": p.personality,
                "tools_filter": p.tools_filter, "active": p.active,
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Project not found".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn update_project_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let agent = state.agent.read().await;
    let existing = match agent.storage.get_project(&id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Not found".to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };
    let updated = crate::storage::entities::project::Model {
        id: id.clone(),
        name: request
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&existing.name)
            .to_string(),
        description: request
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or(&existing.description)
            .to_string(),
        system_prompt: request
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing.system_prompt),
        personality: request
            .get("personality")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing.personality),
        tools_filter: request
            .get("tools_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing.tools_filter),
        active: request
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(existing.active),
        created_at: existing.created_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    match agent.storage.update_project(&updated).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn delete_project_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_project(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Autonomy Endpoints ====================

async fn get_autonomy_settings(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "settings": settings,
        })),
    )
        .into_response()
}

async fn update_autonomy_settings(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;

    if let Some(scope) = request.get("context_scope").and_then(|v| v.as_str()) {
        settings.context_scope = ConversationScope::from_storage(Some(scope));
        let _ = agent
            .storage
            .set(
                "conversation_scope_mode",
                settings.context_scope.as_storage_str().as_bytes(),
            )
            .await;
    }
    if let Some(enabled) = request
        .get("voice_briefing_enabled")
        .and_then(|v| v.as_bool())
    {
        settings.voice_briefing_enabled = enabled;
    }
    if let Some(mode) = request.get("autonomy_mode").and_then(|v| v.as_str()) {
        let normalized = mode.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "off" | "assist" | "auto") {
            settings.autonomy_mode = normalized;
        }
    }
    if let Some(always_ask) = request
        .get("always_ask_high_risk")
        .and_then(|v| v.as_bool())
    {
        settings.always_ask_high_risk = always_ask;
    }
    if let Some(only_approved) = request
        .get("only_approved_skills")
        .and_then(|v| v.as_bool())
    {
        settings.only_approved_skills = only_approved;
    }
    if request.get("quiet_hours_start").is_some() {
        settings.quiet_hours_start = request
            .get("quiet_hours_start")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    if request.get("quiet_hours_end").is_some() {
        settings.quiet_hours_end = request
            .get("quiet_hours_end")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    if request.get("daily_run_limit").is_some() {
        settings.daily_run_limit = match request.get("daily_run_limit") {
            Some(value) if value.is_null() => None,
            Some(value) => value.as_u64().map(|v| v.clamp(1, 1000) as u32),
            None => settings.daily_run_limit,
        };
    }
    if let Some(paused) = request.get("agent_paused").and_then(|v| v.as_bool()) {
        settings.agent_paused = paused;
    }
    if let Some(mode) = request.get("pause_mode").and_then(|v| v.as_str()) {
        let normalized = mode.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "autonomous_only" | "all_execution") {
            settings.pause_mode = normalized;
        }
    }
    if request.get("arkpulse_auth_failures_threshold").is_some() {
        if let Some(v) = request
            .get("arkpulse_auth_failures_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_auth_failures_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if request.get("arkpulse_rate_limit_hits_threshold").is_some() {
        if let Some(v) = request
            .get("arkpulse_rate_limit_hits_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_rate_limit_hits_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if request
        .get("arkpulse_unauthorized_channel_threshold")
        .is_some()
    {
        if let Some(v) = request
            .get("arkpulse_unauthorized_channel_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_unauthorized_channel_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if request
        .get("arkpulse_combined_security_threshold")
        .is_some()
    {
        if let Some(v) = request
            .get("arkpulse_combined_security_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_combined_security_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if let Some(active_mode_id) = request.get("active_mode_id").and_then(|v| v.as_str()) {
        settings.active_mode_id = if active_mode_id.trim().is_empty() {
            None
        } else {
            Some(active_mode_id.to_string())
        };
    }
    if let Some(trust_policy) = request.get("trust_policy") {
        if let Ok(parsed) = serde_json::from_value::<TrustPolicy>(trust_policy.clone()) {
            settings.trust_policy = parsed;
        }
    }
    if let Some(modes) = request.get("modes") {
        if let Ok(parsed) = serde_json::from_value::<Vec<AutopilotMode>>(modes.clone()) {
            settings.modes = parsed;
        }
    }

    match save_autonomy_settings(&agent, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","settings":settings})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

async fn list_autonomy_modes(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "modes": settings.modes,
            "active_mode_id": settings.active_mode_id,
        })),
    )
        .into_response()
}

async fn save_autonomy_modes(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let Some(modes) = request.get("modes") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "modes is required".to_string(),
            }),
        )
            .into_response();
    };
    let parsed = match serde_json::from_value::<Vec<AutopilotMode>>(modes.clone()) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid modes payload: {}", e),
                }),
            )
                .into_response();
        }
    };
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    settings.modes = parsed;
    match save_autonomy_settings(&agent, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","modes":settings.modes})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

async fn activate_autonomy_mode(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    match apply_autopilot_mode(&agent, &mut settings, &id).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","result":result})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

async fn get_context_policy(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "context_scope": settings.context_scope.as_storage_str(),
        })),
    )
        .into_response()
}

async fn set_context_policy(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let scope_raw = request
        .get("context_scope")
        .and_then(|v| v.as_str())
        .unwrap_or("per_channel");
    let scope = ConversationScope::from_storage(Some(scope_raw));
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    settings.context_scope = scope;
    let _ = agent
        .storage
        .set("conversation_scope_mode", scope.as_storage_str().as_bytes())
        .await;
    match save_autonomy_settings(&agent, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","context_scope":scope.as_storage_str()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

async fn get_autonomy_briefing(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    let briefing = build_autonomy_briefing(&agent, &settings).await;
    if let Ok(bytes) = serde_json::to_vec(&briefing) {
        let _ = agent.storage.set(AUTONOMY_LAST_BRIEF_KEY, &bytes).await;
    }
    (StatusCode::OK, Json(briefing)).into_response()
}

async fn accept_autonomy_suggestion(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match accept_chat_suggestion(&state, &id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) if error.contains("not found") => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error })).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
    }
}

async fn dismiss_autonomy_suggestion(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match dismiss_chat_suggestion(&state, &id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) if error.contains("not found") => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error })).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
    }
}

async fn execute_autonomy_action(
    State(state): State<AppState>,
    Json(request): Json<AutonomyExecuteActionRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    match run_recommended_action(&agent, &mut settings, &request.action, request.dry_run).await {
        Ok(result) => {
            let _ = save_autonomy_settings(&agent, &settings).await;
            if !request.dry_run {
                spawn_autonomy_analysis_tick(state.agent.clone(), "autonomy_action");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","result":result})),
            )
                .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

async fn start_goal_loop(
    State(state): State<AppState>,
    Json(request): Json<GoalLoopRequest>,
) -> Response {
    let goal = request.goal.trim();
    if goal.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "goal is required".to_string(),
            }),
        )
            .into_response();
    }

    // Parse optional due date (YYYY-MM-DD), stored as scheduled_for for reminders and visibility.
    let due_date = request
        .due_date
        .as_deref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .map(|d| d.and_hms_opt(23, 59, 59).unwrap())
        .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc));

    let agent = state.agent.read().await;
    let actions = agent.runtime.list_actions().await.unwrap_or_default();
    let action_names: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    let planner_prompt = format!(
        "Create a compact execution plan for this goal as strict JSON.\n\
Return format:\n\
{{\"summary\":\"...\",\"steps\":[{{\"title\":\"...\",\"action\":\"...\",\"arguments\":{{}},\"why\":\"...\"}}]}}\n\
Use only known actions when possible: {}\n\
Goal: {}\n\
Constraints: {}",
        action_names.join(", "),
        goal,
        request.constraints.clone().unwrap_or_else(|| "none".to_string())
    );

    let llm_plan = agent
        .llm
        .chat(
            "You are an execution planner that outputs strict JSON.",
            &planner_prompt,
            &[],
            &actions,
        )
        .await
        .ok();

    let parsed = llm_plan
        .as_ref()
        .and_then(|r| extract_json(&r.content))
        .unwrap_or_else(|| {
            serde_json::json!({
                "summary": format!("Execution loop for {}", goal),
                "steps": [
                    {
                        "title": "Research latest constraints and options",
                        "action": if action_names.iter().any(|a| a == "research") {
                            "research".to_string()
                        } else {
                            action_names
                                .first()
                                .cloned()
                                .unwrap_or_else(|| "daily_brief".to_string())
                        },
                        "arguments": { "query": goal }
                    }
                ]
            })
        });
    let parsed = request
        .plan_override
        .as_ref()
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(parsed);

    let report_cron = request
        .report_cron
        .clone()
        .unwrap_or_else(|| "0 0 9 * * *".to_string());

    if request.preview_only {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status":"preview",
                "plan_preview": parsed.clone(),
                "scheduled_report_cron": report_cron.clone(),
            })),
        )
            .into_response();
    }

    let goal_id = uuid::Uuid::new_v4().to_string();
    let mut goal_task = Task::new(
        format!("Goal: {}", goal),
        "goal".to_string(),
        serde_json::json!({
            "goal_id": goal_id,
            "goal": goal,
            "project_id": request.project_id,
        }),
    );
    goal_task.scheduled_for = due_date;
    // Goal task is a metadata anchor for grouping/progress, not an executable action.
    goal_task.status = TaskStatus::Completed;
    goal_task.result = Some("Goal registered.".to_string());
    if let Err(e) = agent.add_task(goal_task).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    // Auto-schedule reminder tasks if due date is set and > 1 day away.
    if let Some(due) = due_date {
        let now = chrono::Utc::now();
        let days_until = (due - now).num_days();

        let mut reminders: Vec<Task> = Vec::new();
        if days_until > 1 {
            let remind_at = due - chrono::Duration::days(1);
            let mut r = Task::new(
                format!("Reminder: \"{}\" is due tomorrow", goal),
                "goal_reminder".to_string(),
                serde_json::json!({ "goal": goal, "days_left": 1 }),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }
        if days_until > 3 {
            let remind_at = due - chrono::Duration::days(3);
            let mut r = Task::new(
                format!("Reminder: \"{}\" is due in 3 days", goal),
                "goal_reminder".to_string(),
                serde_json::json!({ "goal": goal, "days_left": 3 }),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }

        for r in reminders {
            let _ = agent.add_task(r).await;
        }
    }

    let steps = parsed
        .get("steps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let normalized_steps: Vec<serde_json::Value> = steps
        .iter()
        .map(|step| {
            let action_name = step
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("research");
            let safe_action = if action_names.iter().any(|n| n == action_name) {
                action_name
            } else {
                "research"
            };
            let mut args = step
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            args["goal_id"] = serde_json::json!(goal_id.clone());
            args["goal"] = serde_json::json!(goal);
            serde_json::json!({
                "action": safe_action,
                "arguments": args,
                "rationale": step.get("why").and_then(|v| v.as_str()).unwrap_or("goal-driven step"),
            })
        })
        .collect();

    let mut plan_task = Task::new(
        format!("Goal Loop Plan: {}", goal),
        "plan".to_string(),
        serde_json::json!({
            "goal_id": goal_id,
            "steps": normalized_steps,
            "summary": parsed.get("summary").cloned().unwrap_or_else(|| serde_json::json!("")),
        }),
    );
    plan_task.status = TaskStatus::Pending;
    if let Err(e) = agent.add_task(plan_task).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    let mut report_task = Task::new(
        format!("Goal Progress Report: {}", goal),
        "goal_progress_report".to_string(),
        serde_json::json!({ "goal_id": goal_id, "goal": goal }),
    );
    report_task.cron = Some(report_cron.clone());
    report_task.status = TaskStatus::Pending;
    report_task.approval = TaskApproval::Auto;
    let _ = agent.add_task(report_task).await;

    agent
        .emit_notification(
            "Goal loop started",
            &format!(
                "Goal '{}' entered execution loop with {} planned step(s).",
                goal,
                normalized_steps.len()
            ),
            "info",
            "autonomy_goal_loop",
        )
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status":"ok",
            "goal_id": goal_id,
            "plan_preview": parsed,
            "scheduled_report_cron": report_cron,
        })),
    )
        .into_response()
}

async fn goal_progress_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let goal_id = params.get("goal_id").map(|s| s.as_str());
    let agent = state.agent.read().await;
    let tasks = agent.tasks.read().await;
    let related: Vec<&Task> = tasks
        .all()
        .iter()
        .filter(|t| {
            goal_id
                .map(|g| t.arguments.get("goal_id").and_then(|v| v.as_str()) == Some(g))
                .unwrap_or_else(|| t.arguments.get("goal_id").is_some())
        })
        .collect();
    let completed = related
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed))
        .count();
    let pending = related
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
            )
        })
        .count();
    let failed = related
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
        .count();

    let items: Vec<serde_json::Value> = related
        .iter()
        .take(20)
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(),
                "description": t.description,
                "action": t.action,
                "status": format!("{:?}", t.status),
                "created_at": t.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                "result": t.result,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "goal_id": goal_id,
            "summary": {
                "total": related.len(),
                "completed": completed,
                "pending_or_running": pending,
                "failed": failed,
            },
            "items": items,
        })),
    )
        .into_response()
}

async fn run_goal_report_now(
    State(state): State<AppState>,
    Json(request): Json<GoalReportNowRequest>,
) -> Response {
    let goal_id = request.goal_id.trim();
    if goal_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "goal_id is required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;

    // Find goal text from an existing goal task.
    let (goal_text, project_id) = {
        let tasks = agent.tasks.read().await;
        let goal_task = tasks.all().iter().find(|t| {
            t.action == "goal"
                && t.arguments
                    .get("goal_id")
                    .and_then(|v| v.as_str())
                    .map(|v| v == goal_id)
                    .unwrap_or(false)
        });
        if let Some(t) = goal_task {
            let goal = t
                .arguments
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| t.description.trim_start_matches("Goal: ").to_string());
            let pid = t
                .arguments
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (goal, pid)
        } else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "goal_id not found".to_string(),
                }),
            )
                .into_response();
        }
    };

    let mut report_task = Task::new(
        format!("Goal Progress Report (manual): {}", goal_text),
        "goal_progress_report".to_string(),
        serde_json::json!({
            "goal_id": goal_id,
            "goal": goal_text,
            "project_id": project_id,
        }),
    );
    report_task.scheduled_for = Some(chrono::Utc::now());
    report_task.status = TaskStatus::Pending;
    report_task.approval = TaskApproval::Auto;

    if let Err(e) = agent.add_task(report_task).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response()
}

async fn get_live_incidents(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let mut incidents: Vec<serde_json::Value> = Vec::new();

    let security_logs = agent
        .storage
        .list_security_logs(80)
        .await
        .unwrap_or_default();
    let critical_security: Vec<_> = security_logs
        .iter()
        .filter(|s| s.severity == "error" || s.severity == "critical")
        .collect();
    if !critical_security.is_empty() {
        incidents.push(serde_json::json!({
            "id": format!("sec:{}", critical_security[0].event_type),
            "severity": "critical",
            "title": "Security anomaly detected",
            "detail": format!("{} high-severity security event(s) recorded.", critical_security.len()),
        }));
    }

    let failed_tasks: Vec<_> = {
        let tasks = agent.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
    };
    for task in &failed_tasks {
        incidents.push(serde_json::json!({
            "id": format!("task_fail:{}", task.id),
            "severity": "high",
            "title": "Task failure requires triage",
            "detail": task.description,
        }));
    }

    let failed_watchers: Vec<_> = agent
        .watcher_manager
        .list()
        .await
        .into_iter()
        .filter(|w| {
            matches!(
                w.status,
                crate::core::watcher::WatcherStatus::Failed { .. }
                    | crate::core::watcher::WatcherStatus::TimedOut
            )
        })
        .collect();
    for watcher in failed_watchers.iter().take(5) {
        incidents.push(serde_json::json!({
            "id": format!("watcher:{}", watcher.id),
            "severity": "medium",
            "title": "Watcher degraded",
            "detail": watcher.description,
        }));
    }

    incidents.sort_by(|a, b| {
        let sa = a.get("severity").and_then(|v| v.as_str()).unwrap_or("low");
        let sb = b.get("severity").and_then(|v| v.as_str()).unwrap_or("low");
        sb.cmp(sa)
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({ "incidents": incidents })),
    )
        .into_response()
}

async fn execute_incident_playbook(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    let action = if id.starts_with("sec:") {
        recommendation(
            "Contain Security Incident",
            "Start a security containment and mitigation workflow.",
            "create_task",
            serde_json::json!({
                "description":"Contain security incident and propose mitigations",
                "action":"research",
                "arguments":{"query":"Contain current security incident, identify source, and propose mitigations."},
                "approval":"require"
            }),
            &settings.trust_policy,
        )
    } else if id.starts_with("task_fail:") {
        recommendation(
            "Recover Failed Task",
            "Generate a concrete recovery plan for the failed execution.",
            "chat_prompt",
            serde_json::json!({"prompt":"Review failed tasks, identify root causes, and propose immediate recovery actions."}),
            &settings.trust_policy,
        )
    } else {
        recommendation(
            "Stabilize Incident",
            "Produce a stabilization checklist for this incident.",
            "chat_prompt",
            serde_json::json!({"prompt":"Create a stabilization checklist for the current incident and prioritize actions."}),
            &settings.trust_policy,
        )
    };

    match run_recommended_action(&agent, &mut settings, &action, false).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","result":result})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

async fn triage_inbox(
    State(state): State<AppState>,
    Json(request): Json<InboxTriageRequest>,
) -> Response {
    let labels = request
        .labels
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| {
            vec![
                "Act now".to_string(),
                "Delegate".to_string(),
                "Ignore".to_string(),
            ]
        });
    let agent = state.agent.read().await;

    let mut messages = request.messages.clone();
    if messages.is_empty() {
        let fallback = agent
            .storage
            .list_notifications(30, 0, true)
            .await
            .unwrap_or_default();
        messages = fallback
            .into_iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "from": n.source,
                    "subject": n.title,
                    "snippet": n.body,
                })
            })
            .collect();
    }

    if messages.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"triage":[],"draft_replies":[]})),
        )
            .into_response();
    }

    let payload = serde_json::json!({ "messages": messages, "labels": labels });
    let llm_response = agent.llm.chat(
        "You are an executive inbox triage assistant. Return strict JSON {\"triage\":[{\"message_id\":\"...\",\"label\":\"...\",\"reason\":\"...\",\"draft_reply\":\"...\"}]}.",
        &payload.to_string(),
        &[],
        &[],
    ).await.ok();
    if let Some(ref r) = llm_response {
        agent.record_llm_usage("web", "inbox_triage", r).await;
    }

    let parsed = llm_response
        .as_ref()
        .and_then(|r| extract_json(&r.content))
        .unwrap_or_else(|| {
            let triage: Vec<serde_json::Value> = payload
                .get("messages")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|m| {
                    let snippet = m.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
                    let label = if snippet.contains("urgent") || snippet.contains("asap") { "Act now" } else { "Delegate" };
                    serde_json::json!({
                        "message_id": m.get("id").cloned().unwrap_or_else(|| serde_json::json!("")),
                        "label": label,
                        "reason": "Heuristic fallback classification",
                        "draft_reply": if label == "Act now" { "Acknowledged. I will handle this today." } else { "Received. Delegating to the right owner and will track status." },
                    })
                })
                .collect();
            serde_json::json!({ "triage": triage })
        });

    let triage = parsed
        .get("triage")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "triage": triage,
            "labels": labels,
        })),
    )
        .into_response()
}

async fn get_outcome_timeline(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(120usize)
        .min(500);
    let agent = state.agent.read().await;
    let mut events: Vec<serde_json::Value> = Vec::new();

    {
        let trace = state.trace_history.read().await;
        for t in trace.iter().take(limit) {
            let ts = t
                .completed_at
                .unwrap_or_else(|| t.started_at.unwrap_or_else(chrono::Utc::now))
                .to_rfc3339();
            events.push(serde_json::json!({
                "id": format!("trace:{}", t.id),
                "source": "trace",
                "timestamp": ts,
                "title": t.message,
                "status": if t.completed_at.is_some() { "completed" } else { "in_progress" },
                "detail": t.response.as_deref().map(crate::security::redact_pii),
                "rollback": null
            }));
        }
    }
    {
        let tasks = agent.tasks.read().await;
        for t in tasks.all().iter().take(limit) {
            events.push(serde_json::json!({
                "id": format!("task:{}", t.id),
                "source": "task",
                "timestamp": t.created_at.to_rfc3339(),
                "title": t.description,
                "status": format!("{:?}", t.status),
                "detail": t.result.as_deref().map(crate::security::redact_pii),
                "rollback": {
                    "operation": if matches!(t.status, TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::Paused | TaskStatus::InProgress) { "cancel_task" } else { "none" }
                }
            }));
        }
    }

    for n in agent
        .storage
        .list_notifications(limit as u64, 0, false)
        .await
        .unwrap_or_default()
    {
        events.push(serde_json::json!({
            "id": format!("notification:{}", n.id),
            "source": "notification",
            "timestamp": n.created_at,
            "title": n.title,
            "status": if n.read { "read" } else { "unread" },
            "detail": crate::security::redact_pii(&n.body),
            "rollback": { "operation": "toggle_notification_read" }
        }));
    }

    for s in agent
        .storage
        .list_security_logs(limit as u64)
        .await
        .unwrap_or_default()
    {
        events.push(serde_json::json!({
            "id": format!("security:{}", s.id),
            "source": "security",
            "timestamp": s.created_at,
            "title": format!("{} [{}]", s.event_type, s.severity),
            "status": "logged",
            "detail": crate::security::redact_pii(&s.message),
            "rollback": null
        }));
    }

    for d in agent
        .storage
        .get_recent_delegations(limit as u64)
        .await
        .unwrap_or_default()
    {
        events.push(serde_json::json!({
            "id": format!("delegation:{}", d.id),
            "source": "delegation",
            "timestamp": d.created_at,
            "title": d.task_description,
            "status": if d.success == 1 { "success" } else { "failed" },
            "detail": d.result.as_deref().map(crate::security::redact_pii),
            "rollback": null
        }));
    }

    events.sort_by(|a, b| {
        b.get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(a.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""))
    });
    events.truncate(limit);

    (
        StatusCode::OK,
        Json(serde_json::json!({ "events": events })),
    )
        .into_response()
}

async fn rollback_timeline_event(
    State(state): State<AppState>,
    Json(request): Json<TimelineRollbackRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let event_id = request.event_id.trim();
    let operation = request.operation.unwrap_or_default();

    if let Some(task_id) = event_id.strip_prefix("task:") {
        let uuid = match uuid::Uuid::parse_str(task_id) {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid task id".to_string(),
                    }),
                )
                    .into_response()
            }
        };
        let mut tasks = agent.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };
        if !matches!(
            task.status,
            TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
        ) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Task cannot be cancelled from current state".to_string(),
                }),
            )
                .into_response();
        }
        task.status = TaskStatus::Cancelled;
        let status_json =
            serde_json::to_string(&task.status).unwrap_or_else(|_| "\"Cancelled\"".to_string());
        let _ = agent
            .storage
            .update_task_status(task_id, &status_json)
            .await;
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","operation":"cancel_task"})),
        )
            .into_response();
    }

    if let Some(watcher_id) = event_id.strip_prefix("watcher:") {
        let uuid = match uuid::Uuid::parse_str(watcher_id) {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid watcher id".to_string(),
                    }),
                )
                    .into_response()
            }
        };
        if agent.watcher_manager.cancel(uuid).await {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                    .await;
            }
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","operation":"cancel_watcher"})),
            )
                .into_response();
        }
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Watcher not found or not cancellable".to_string(),
            }),
        )
            .into_response();
    }

    if let Some(notification_id) = event_id.strip_prefix("notification:") {
        let read = operation != "mark_unread";
        if let Err(e) = agent
            .storage
            .set_notification_read(notification_id, read)
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
        return (StatusCode::OK, Json(serde_json::json!({"status":"ok","operation":"toggle_notification_read","read":read}))).into_response();
    }

    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "Unsupported rollback target".to_string(),
        }),
    )
        .into_response()
}

async fn query_knowledge_brain(
    State(state): State<AppState>,
    Json(request): Json<KnowledgeQueryRequest>,
) -> Response {
    if request.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "query is required".to_string(),
            }),
        )
            .into_response();
    }
    let limit = request.limit.unwrap_or(8).clamp(1, 20);
    let agent = state.agent.read().await;
    let docs = agent
        .search_documents(&request.query, limit)
        .await
        .unwrap_or_default();
    let facts = agent
        .encrypted_storage
        .get_facts_by_project_decrypted(limit as u64, 0, request.project_id.as_deref())
        .await
        .unwrap_or_default();

    let evidence_docs: Vec<serde_json::Value> = docs
        .iter()
        .map(|(doc_id, content, score)| {
            serde_json::json!({
                "document_id": doc_id,
                "score": score,
                "snippet": content.chars().take(260).collect::<String>(),
            })
        })
        .collect();
    let evidence_facts: Vec<serde_json::Value> = facts
        .iter()
        .take(limit)
        .map(|f| {
            serde_json::json!({
                "fact": f.fact,
                "confidence": f.confidence,
                "sources": f.sources,
            })
        })
        .collect();

    let synthesis_prompt = format!(
        "Answer the user query using the supplied evidence only.\n\
If confidence is low, explicitly say what knowledge should be imported.\n\
User query: {}\n\nEvidence docs: {}\n\nEvidence facts: {}",
        request.query,
        serde_json::to_string(&evidence_docs).unwrap_or_default(),
        serde_json::to_string(&evidence_facts).unwrap_or_default(),
    );
    let answer = match agent
        .llm
        .chat(
            "You are a grounded knowledge assistant. Cite document IDs inline like [doc:<id>].",
            &synthesis_prompt,
            &[],
            &[],
        )
        .await
    {
        Ok(r) => {
            agent
                .record_llm_usage("web", "knowledge_synthesis", &r)
                .await;
            crate::security::redact_pii(&r.content)
        }
        Err(_) => {
            if evidence_docs.is_empty() && evidence_facts.is_empty() {
                "I do not have enough indexed knowledge yet. Import documents, notes, or emails related to this topic.".to_string()
            } else {
                "I found relevant evidence, but synthesis failed. Try again with a narrower question.".to_string()
            }
        }
    };

    let missing_signals = if evidence_docs.len() < 2 {
        vec![
            "Import source documents for this topic".to_string(),
            "Ingest related emails or notes to improve recall".to_string(),
        ]
    } else {
        vec![]
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "answer": answer,
            "sources": {
                "documents": evidence_docs,
                "facts": evidence_facts,
            },
            "import_suggestions": missing_signals,
        })),
    )
        .into_response()
}

async fn suggest_knowledge_imports(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let traces = agent.trace_history.read().await;
    let mut token_counts: HashMap<String, usize> = HashMap::new();
    for t in traces.iter().take(60) {
        for word in t
            .message
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 5)
        {
            *token_counts.entry(word.to_string()).or_insert(0) += 1;
        }
    }
    let mut tokens: Vec<(String, usize)> = token_counts.into_iter().collect();
    tokens.sort_by(|a, b| b.1.cmp(&a.1));
    let suggestions: Vec<serde_json::Value> = tokens
        .into_iter()
        .take(8)
        .map(|(topic, count)| {
            serde_json::json!({
                "topic": topic,
                "signal_count": count,
                "suggested_import": format!("Add documents/notes related to '{}'", topic),
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "suggestions": suggestions })),
    )
        .into_response()
}

async fn load_nudge_feedback(agent: &Agent) -> HashMap<String, NudgeFeedbackPreference> {
    agent
        .storage
        .get(AUTONOMY_NUDGE_FEEDBACK_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| {
            serde_json::from_slice::<HashMap<String, NudgeFeedbackPreference>>(&raw).ok()
        })
        .unwrap_or_default()
}

async fn save_nudge_feedback(agent: &Agent, map: &HashMap<String, NudgeFeedbackPreference>) {
    if let Ok(raw) = serde_json::to_vec(map) {
        let _ = agent.storage.set(AUTONOMY_NUDGE_FEEDBACK_KEY, &raw).await;
    }
}

async fn load_nudge_timestamps(agent: &Agent, key: &str) -> HashMap<String, String> {
    agent
        .storage
        .get(key)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<HashMap<String, String>>(&raw).ok())
        .unwrap_or_default()
}

async fn save_nudge_timestamps(agent: &Agent, key: &str, map: &HashMap<String, String>) {
    if let Ok(raw) = serde_json::to_vec(map) {
        let _ = agent.storage.set(key, &raw).await;
    }
}

fn is_feedback_suppressed(
    pref: Option<&NudgeFeedbackPreference>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let Some(pref) = pref else {
        return false;
    };
    if pref.dismissed {
        return true;
    }
    if let Some(until) = pref.suppressed_until.as_deref().and_then(parse_utc_rfc3339) {
        return now < until;
    }
    false
}

fn nudge_emit_cooldown_secs(priority: u8) -> i64 {
    match priority {
        5 => 20 * 60,
        4 => 30 * 60,
        3 => 60 * 60,
        _ => 3 * 60 * 60,
    }
}

async fn maybe_push_nudge(
    agent: &Agent,
    out: &mut Vec<PredictiveNudge>,
    hidden: &mut usize,
    feedback: &HashMap<String, NudgeFeedbackPreference>,
    now: chrono::DateTime<chrono::Utc>,
    mut nudge: PredictiveNudge,
) {
    if is_feedback_suppressed(feedback.get(&nudge.id), now) {
        *hidden += 1;
        return;
    }
    let query = format!("{} {}", nudge.title, nudge.detail);
    nudge.memory_clues = collect_memory_clues(agent, &query).await;
    out.push(nudge);
}

async fn build_predictive_nudges(
    agent: &Agent,
    settings: &AutonomySettings,
) -> (Vec<PredictiveNudge>, usize) {
    let now = chrono::Utc::now();
    let feedback = load_nudge_feedback(agent).await;
    let mut hidden = 0usize;
    let mut nudges: Vec<PredictiveNudge> = Vec::new();

    let (overdue_tasks, due_soon_tasks, failed_tasks, pending_tasks) = {
        let tasks = agent.tasks.read().await;
        let mut overdue = Vec::new();
        let mut due_soon = Vec::new();
        let mut failed = 0usize;
        let mut pending = 0usize;
        for task in tasks.all().iter() {
            if matches!(task.status, TaskStatus::Failed { .. }) {
                failed += 1;
            }
            if matches!(
                task.status,
                TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::Paused
            ) {
                pending += 1;
            }
            if let Some(scheduled_for) = task.scheduled_for {
                let minutes = (scheduled_for - now).num_minutes();
                if minutes < 0
                    && matches!(
                        task.status,
                        TaskStatus::Pending | TaskStatus::AwaitingApproval
                    )
                {
                    overdue.push((
                        task.id.to_string(),
                        task.description.clone(),
                        task.action.clone(),
                        task.arguments.clone(),
                        -minutes,
                    ));
                } else if (0..=45).contains(&minutes)
                    && matches!(
                        task.status,
                        TaskStatus::Pending
                            | TaskStatus::AwaitingApproval
                            | TaskStatus::Paused
                            | TaskStatus::InProgress
                    )
                {
                    due_soon.push((task.id.to_string(), task.description.clone(), minutes));
                }
            }
        }
        (overdue, due_soon, failed, pending)
    };

    for (task_id, description, action_name, arguments, overdue_minutes) in
        overdue_tasks.into_iter().take(4)
    {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: format!("overdue-task-{}", task_id),
                nudge_type: "deadline_risk".to_string(),
                title: "Missed scheduled execution window".to_string(),
                detail: format!(
                    "Task '{}' is overdue by {} minutes.",
                    description, overdue_minutes
                ),
                confidence: 0.92,
                priority: 5,
                source: Some("task_scheduler".to_string()),
                recommended_action: Some(recommendation(
                    "Reschedule overdue task",
                    "Move overdue task into an explicit immediate window.",
                    "create_task",
                    serde_json::json!({
                        "description": format!("Retry: {}", description),
                        "action": action_name,
                        "arguments": arguments,
                        "approval": "auto"
                    }),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    for (task_id, description, due_minutes) in due_soon_tasks.into_iter().take(3) {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: format!("due-soon-{}", task_id),
                nudge_type: "schedule_attention".to_string(),
                title: "Task is due soon".to_string(),
                detail: format!(
                    "'{}' is scheduled in {} minute(s).",
                    description, due_minutes
                ),
                confidence: 0.84,
                priority: 3,
                source: Some("task_scheduler".to_string()),
                recommended_action: Some(recommendation(
                    "Prep due-soon task",
                    "Draft immediate next steps so the due task executes cleanly.",
                    "chat_prompt",
                    serde_json::json!({
                        "prompt": format!("Prepare a quick execution checklist for this due-soon task: {}", description)
                    }),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    if failed_tasks >= 2 {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: "failure-cluster".to_string(),
                nudge_type: "reliability".to_string(),
                title: "Reliability regression detected".to_string(),
                detail: format!(
                    "{} failed task(s) are in the queue. A triage pass can prevent repeat failures.",
                    failed_tasks
                ),
                confidence: 0.82,
                priority: 4,
                source: Some("task_history".to_string()),
                recommended_action: Some(recommendation(
                    "Triage failed tasks",
                    "Summarize root causes and propose fixes for the latest failed tasks.",
                    "chat_prompt",
                    serde_json::json!({
                        "prompt":"Triage recent failed tasks, group by root cause, and propose fix tasks with priorities."
                    }),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    if pending_tasks == 0 {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: "capacity-window".to_string(),
                nudge_type: "planning".to_string(),
                title: "High strategic capacity available".to_string(),
                detail:
                    "Queue is clear. This is a good time to schedule one high-leverage routine."
                        .to_string(),
                confidence: 0.68,
                priority: 2,
                source: Some("task_queue".to_string()),
                recommended_action: Some(recommendation(
                    "Schedule a strategic routine",
                    "Create a recurring brief so the system keeps improving proactively.",
                    "create_task",
                    serde_json::json!({
                        "description":"Strategic weekly brief",
                        "action":"daily_brief",
                        "arguments":{"mode":"strategy"},
                        "cron":"0 0 9 * * 1",
                        "approval":"auto"
                    }),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    let unread_alerts = agent
        .storage
        .count_unread_notifications()
        .await
        .unwrap_or(0);
    if unread_alerts > 0 {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: "attention-load".to_string(),
                nudge_type: "attention_load".to_string(),
                title: "Unread system alerts are accumulating".to_string(),
                detail: format!(
                    "{} unread notification(s) may hide urgent work.",
                    unread_alerts
                ),
                confidence: 0.74,
                priority: 4,
                source: Some("notifications".to_string()),
                recommended_action: Some(recommendation(
                    "Enable Ops Mode",
                    "Apply the Ops preset: create monitoring watchers and incident-focused routines, and make Ops the active autonomy mode.",
                    "activate_mode",
                    serde_json::json!({"mode_id":"ops"}),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    let watcher_count = agent.watcher_manager.list().await.len();
    if watcher_count == 0 {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: "automation-gap".to_string(),
                nudge_type: "automation_gap".to_string(),
                title: "No active watchers".to_string(),
                detail: "Set at least one watcher for key external signals to stay proactive."
                    .to_string(),
                confidence: 0.66,
                priority: 3,
                source: Some("watchers".to_string()),
                recommended_action: Some(recommendation(
                    "Create first watcher",
                    "Create one watcher for the highest-risk external dependency.",
                    "chat_prompt",
                    serde_json::json!({"prompt":"Create one watcher for my highest-risk external dependency."}),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    let memory_entries = agent.memory.entry_count();
    if memory_entries < 20 {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: "memory-thin".to_string(),
                nudge_type: "memory_coverage".to_string(),
                title: "Memory coverage is still thin".to_string(),
                detail: format!(
                    "Only {} durable memory entries are available. More context will improve personalization.",
                    memory_entries
                ),
                confidence: 0.62,
                priority: 2,
                source: Some("memory".to_string()),
                recommended_action: Some(recommendation(
                    "Capture preferences check-in",
                    "Ask the user for persistent preferences and constraints to improve long-term suggestions.",
                    "chat_prompt",
                    serde_json::json!({"prompt":"Ask me 5 high-impact preference questions to improve future proactive suggestions."}),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    let (trace_total, trace_failures) = {
        let traces = agent.trace_history.read().await;
        let total = traces.len();
        let failures = traces
            .iter()
            .take(40)
            .filter(|t| {
                t.steps.iter().any(|step| {
                    let step_type = step.step_type.to_ascii_lowercase();
                    step_type.contains("fail")
                        || step_type.contains("error")
                        || step_type.contains("warning")
                })
            })
            .count();
        (total, failures)
    };
    if trace_total >= 8 && trace_failures >= 4 {
        maybe_push_nudge(
            agent,
            &mut nudges,
            &mut hidden,
            &feedback,
            now,
            PredictiveNudge {
                id: "stream-instability".to_string(),
                nudge_type: "event_stream_health".to_string(),
                title: "Execution stream instability".to_string(),
                detail: format!(
                    "{} of the latest traces ended in failure states; investigate recurring failure patterns.",
                    trace_failures
                ),
                confidence: 0.79,
                priority: 4,
                source: Some("trace_history".to_string()),
                recommended_action: Some(recommendation(
                    "Investigate event-stream failures",
                    "Summarize recurring failure patterns and propose concrete fixes.",
                    "chat_prompt",
                    serde_json::json!({"prompt":"Analyze the latest failed traces and propose a concrete hardening plan with prioritized fixes."}),
                    &settings.trust_policy,
                )),
                memory_clues: Vec::new(),
            },
        )
        .await;
    }

    nudges.sort_by(|a, b| {
        b.priority.cmp(&a.priority).then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    nudges.truncate(12);
    (nudges, hidden)
}

pub async fn run_autonomy_analysis_tick(
    shared: Arc<RwLock<Agent>>,
    trigger: &str,
) -> serde_json::Value {
    let agent = shared.read().await;
    let now = chrono::Utc::now();

    // Prevent storming on chat-heavy sessions. Manual and scheduled triggers bypass this gate.
    if trigger != "manual" && trigger != "sentinel_periodic" {
        let last_scan = agent
            .storage
            .get(AUTONOMY_NUDGE_LAST_SCAN_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| String::from_utf8(raw).ok())
            .and_then(|s| parse_utc_rfc3339(&s));
        if let Some(last) = last_scan {
            if (now - last).num_seconds() < 30 {
                return serde_json::json!({
                    "status":"ok",
                    "trigger": trigger,
                    "generated_at": now.to_rfc3339(),
                    "skipped": true,
                    "reason": "cooldown",
                });
            }
        }
    }

    let settings = load_autonomy_settings(&agent).await;
    let (nudges, hidden_count) = build_predictive_nudges(&agent, &settings).await;

    let mut notified = load_nudge_timestamps(&agent, AUTONOMY_NUDGE_NOTIFIED_KEY).await;
    let mut emitted = 0usize;

    for nudge in nudges.iter().take(6) {
        if nudge.priority < 3 {
            continue;
        }
        let should_emit = match notified.get(&nudge.id).and_then(|s| parse_utc_rfc3339(s)) {
            Some(last) => (now - last).num_seconds() >= nudge_emit_cooldown_secs(nudge.priority),
            None => true,
        };
        if !should_emit {
            continue;
        }

        let body = format!(
            "{} (priority {}, confidence {:.0}%) - {}",
            nudge.title,
            nudge.priority,
            (nudge.confidence * 100.0).round(),
            nudge.detail
        );
        agent
            .emit_notification("What To Improve Now", &body, "info", "predictive_nudge")
            .await;
        notified.insert(nudge.id.clone(), now.to_rfc3339());
        emitted += 1;
    }

    let (awaiting_approval, missing_inputs) = {
        let tasks = agent.tasks.read().await;
        let awaiting = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::AwaitingApproval))
            .count();
        drop(tasks);
        let unread = agent
            .storage
            .list_notifications(120, 0, true)
            .await
            .unwrap_or_default();
        let missing = unread
            .iter()
            .filter(|n| {
                let source = n.source.to_ascii_lowercase();
                let title = n.title.to_ascii_lowercase();
                let body = n.body.to_ascii_lowercase();
                source == "workflow_inputs"
                    || title.contains("missing input")
                    || body.contains("missing input")
                    || title.contains("required input")
                    || body.contains("required input")
            })
            .count();
        (awaiting, missing)
    };
    let attention_signature = format!("a:{}|m:{}", awaiting_approval, missing_inputs);
    let last_attention_signature = agent
        .storage
        .get(AUTONOMY_ATTENTION_STATE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    if awaiting_approval > 0 || missing_inputs > 0 {
        if last_attention_signature.as_deref() != Some(attention_signature.as_str()) {
            let mode_state = if settings.autonomy_mode.eq_ignore_ascii_case("auto") {
                "ON"
            } else if settings.autonomy_mode.eq_ignore_ascii_case("assist") {
                "ASSIST"
            } else {
                "OFF"
            };
            let body = format!(
                "Auto Mode is {} | Waiting on you: {} approvals, {} missing input{}",
                mode_state,
                awaiting_approval,
                missing_inputs,
                if missing_inputs == 1 { "" } else { "s" }
            );
            agent
                .emit_notification(
                    "Autonomy Needs Attention",
                    &body,
                    "warning",
                    "autonomy_attention",
                )
                .await;
            agent.notify_preferred_channel(&body).await;
            let _ = agent
                .storage
                .set(AUTONOMY_ATTENTION_STATE_KEY, attention_signature.as_bytes())
                .await;
        }
    } else if last_attention_signature.is_some() {
        let _ = agent.storage.delete(AUTONOMY_ATTENTION_STATE_KEY).await;
    }

    save_nudge_timestamps(&agent, AUTONOMY_NUDGE_NOTIFIED_KEY, &notified).await;
    if let Ok(raw) = serde_json::to_vec(&nudges) {
        let _ = agent.storage.set(AUTONOMY_LAST_NUDGES_KEY, &raw).await;
    }
    let _ = agent
        .storage
        .set(AUTONOMY_NUDGE_LAST_SCAN_KEY, now.to_rfc3339().as_bytes())
        .await;

    serde_json::json!({
        "status":"ok",
        "trigger": trigger,
        "generated_at": now.to_rfc3339(),
        "nudges": nudges.len(),
        "hidden": hidden_count,
        "emitted": emitted,
        "needs_attention": {
            "awaiting_approval": awaiting_approval,
            "missing_inputs": missing_inputs,
        }
    })
}

fn spawn_autonomy_analysis_tick(agent: SharedAgent, trigger: &str) {
    let trigger = trigger.to_string();
    tokio::spawn(async move {
        let _ = run_autonomy_analysis_tick(agent, &trigger).await;
    });
}

async fn get_predictive_nudges(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    let (nudges, hidden_count) = build_predictive_nudges(&agent, &settings).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "nudges": nudges,
            "hidden_count": hidden_count,
        })),
    )
        .into_response()
}

async fn set_predictive_nudge_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<NudgeFeedbackRequest>,
) -> Response {
    if id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "nudge id is required".to_string(),
            }),
        )
            .into_response();
    }

    let action = request.action.trim().to_ascii_lowercase();
    let agent = state.agent.read().await;
    let mut map = load_nudge_feedback(&agent).await;
    let now = chrono::Utc::now();

    match action.as_str() {
        "dismiss" => {
            let entry = map.entry(id.clone()).or_default();
            entry.dismissed = true;
            entry.suppressed_until = None;
            entry.last_feedback = Some("dismiss".to_string());
            entry.note = request.note.clone();
            entry.updated_at = Some(now.to_rfc3339());
        }
        "snooze" => {
            let minutes = request
                .snooze_minutes
                .unwrap_or(24 * 60)
                .clamp(5, 60 * 24 * 30);
            let until = now + chrono::Duration::minutes(minutes as i64);
            let entry = map.entry(id.clone()).or_default();
            entry.dismissed = false;
            entry.suppressed_until = Some(until.to_rfc3339());
            entry.last_feedback = Some("snooze".to_string());
            entry.note = request.note.clone();
            entry.updated_at = Some(now.to_rfc3339());
        }
        "interested" | "reset" => {
            map.remove(&id);
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "action must be one of: dismiss, snooze, interested, reset".to_string(),
                }),
            )
                .into_response();
        }
    }

    save_nudge_feedback(&agent, &map).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status":"ok",
            "id": id,
            "action": action,
            "preference": map.get(&id).cloned(),
        })),
    )
        .into_response()
}

async fn plan_predictive_nudges(
    State(state): State<AppState>,
    Json(request): Json<NudgePlannerRequest>,
) -> Response {
    let max_items = request.max_items.unwrap_or(3).clamp(1, 8);
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    let (nudges, _) = build_predictive_nudges(&agent, &settings).await;
    let now = chrono::Utc::now();
    let mut planned_map = load_nudge_timestamps(&agent, AUTONOMY_NUDGE_PLANNED_KEY).await;

    let mut planned_results = Vec::new();
    let mut skipped = Vec::new();
    for nudge in nudges.into_iter().take(max_items) {
        let Some(action) = nudge.recommended_action.clone() else {
            skipped.push(serde_json::json!({
                "id": nudge.id,
                "reason":"no_recommended_action",
            }));
            continue;
        };

        if !request.dry_run {
            let recently_planned = planned_map
                .get(&nudge.id)
                .and_then(|s| parse_utc_rfc3339(s))
                .is_some_and(|last| (now - last).num_seconds() < 2 * 60 * 60);
            if recently_planned {
                skipped.push(serde_json::json!({
                    "id": nudge.id,
                    "reason":"cooldown_active",
                }));
                continue;
            }
        }

        match run_recommended_action(&agent, &mut settings, &action, request.dry_run).await {
            Ok(result) => {
                if !request.dry_run {
                    planned_map.insert(nudge.id.clone(), now.to_rfc3339());
                }
                planned_results.push(serde_json::json!({
                    "id": nudge.id,
                    "title": nudge.title,
                    "priority": nudge.priority,
                    "confidence": nudge.confidence,
                    "result": result,
                }));
            }
            Err(e) => {
                skipped.push(serde_json::json!({
                    "id": nudge.id,
                    "reason":"execution_error",
                    "error": e,
                }));
            }
        }
    }

    let _ = save_autonomy_settings(&agent, &settings).await;
    save_nudge_timestamps(&agent, AUTONOMY_NUDGE_PLANNED_KEY, &planned_map).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status":"ok",
            "dry_run": request.dry_run,
            "planned": planned_results,
            "skipped": skipped,
        })),
    )
        .into_response()
}

async fn emit_predictive_nudges(State(state): State<AppState>) -> Response {
    let result = run_autonomy_analysis_tick(state.agent.clone(), "manual").await;
    (StatusCode::OK, Json(result)).into_response()
}

async fn evaluate_trust_request(
    State(state): State<AppState>,
    Json(request): Json<TrustEvaluateRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    let envelope: RiskEnvelope = score_action_risk(
        &request.action_kind,
        &request.payload,
        &settings.trust_policy,
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "risk": {
                "level": risk_level_label(&envelope.level),
                "score": envelope.score,
                "requires_approval": envelope.requires_approval,
                "reasons": envelope.reasons,
            }
        })),
    )
        .into_response()
}

async fn get_voice_briefing(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    if !settings.voice_briefing_enabled {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"enabled":false,"message":"Voice briefing is disabled"})),
        )
            .into_response();
    }
    let briefing = build_autonomy_briefing(&agent, &settings).await;
    let short_risks = briefing
        .top_risks
        .iter()
        .take(2)
        .map(|r| r.get("title").and_then(|v| v.as_str()).unwrap_or("risk"))
        .collect::<Vec<_>>()
        .join("; ");
    let short_opps = briefing
        .top_opportunities
        .iter()
        .take(2)
        .map(|r| {
            r.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("opportunity")
        })
        .collect::<Vec<_>>()
        .join("; ");
    let spoken = format!(
        "Good day. Top risks: {}. Top opportunities: {}. I have {} recommended action items.",
        if short_risks.is_empty() {
            "none critical"
        } else {
            &short_risks
        },
        if short_opps.is_empty() {
            "none identified"
        } else {
            &short_opps
        },
        briefing.recommended_actions.len()
    );
    let ssml = format!(
        "<speak><p>{}</p><p>You can say: do it, defer, or summarize.</p></speak>",
        spoken
    );
    if let Ok(bytes) = serde_json::to_vec(&briefing) {
        let _ = agent.storage.set(AUTONOMY_LAST_BRIEF_KEY, &bytes).await;
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "enabled": true,
            "spoken_text": spoken,
            "ssml": ssml,
            "recommended_actions": briefing.recommended_actions,
        })),
    )
        .into_response()
}

async fn handle_voice_command(
    State(state): State<AppState>,
    Json(request): Json<VoiceCommandRequest>,
) -> Response {
    let cmd = request.command.trim().to_ascii_lowercase();
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    let last_brief = agent
        .storage
        .get(AUTONOMY_LAST_BRIEF_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_slice::<AutonomyBriefingResponse>(&v).ok());

    match cmd.as_str() {
        "summarize" => {
            if let Some(brief) = last_brief {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status":"ok",
                        "summary": {
                            "risks": brief.top_risks,
                            "opportunities": brief.top_opportunities,
                            "actions": brief.recommended_actions,
                        }
                    })),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","summary":"No recent briefing available"})),
            )
                .into_response()
        }
        "defer" => {
            agent
                .emit_notification(
                    "Voice command",
                    "Skill deferred by voice command.",
                    "info",
                    "voice",
                )
                .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","result":"Deferred current recommendation"})),
            )
                .into_response()
        }
        "do it" => {
            let Some(brief) = last_brief else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "No recent voice briefing to execute from".to_string(),
                    }),
                )
                    .into_response();
            };
            let action = if let Some(action_id) = request.action_id.as_ref() {
                brief
                    .recommended_actions
                    .into_iter()
                    .find(|a| &a.id == action_id)
            } else {
                brief.recommended_actions.into_iter().next()
            };
            let Some(action) = action else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "No matching recommendation found".to_string(),
                    }),
                )
                    .into_response();
            };
            return match run_recommended_action(&agent, &mut settings, &action, false).await {
                Ok(result) => (
                    StatusCode::OK,
                    Json(serde_json::json!({"status":"ok","result":result})),
                )
                    .into_response(),
                Err(e) => {
                    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response()
                }
            };
        }
        _ => {
            let prompt = format!(
                "Voice command: {}. Respond with a short actionable interpretation.",
                request.command
            );
            match agent.process_message_with_meta(&prompt, "voice", None, None).await {
                Ok(r) => (StatusCode::OK, Json(serde_json::json!({"status":"ok","response": crate::security::redact_pii(&r.response)}))).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: e.to_string() })).into_response(),
            }
        }
    }
}

// ==================== Notification Endpoints ====================

async fn notification_stream_endpoint(State(state): State<AppState>) -> Response {
    let mut notification_events = {
        let agent = state.agent.read().await;
        agent.subscribe_notification_events()
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(32);
    tokio::spawn(async move {
        let connected = serde_json::json!({
            "kind": "notifications.connected",
            "connected_at": chrono::Utc::now().to_rfc3339(),
        });
        if tx
            .send(Ok(Event::default()
                .event("connected")
                .data(connected.to_string())))
            .await
            .is_err()
        {
            return;
        }

        loop {
            match notification_events.recv().await {
                Ok(payload) => {
                    let message = match serde_json::to_string(&payload) {
                        Ok(message) => message,
                        Err(error) => {
                            tracing::warn!(
                                "Failed to serialize notification stream event: {}",
                                error
                            );
                            continue;
                        }
                    };
                    if tx
                        .send(Ok(Event::default().event("notification").data(message)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    let resync = serde_json::json!({
                        "kind": "notifications.resync",
                        "reason": "lagged",
                        "skipped": skipped,
                    });
                    if tx
                        .send(Ok(Event::default()
                            .event("resync")
                            .data(resync.to_string())))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    let closed = serde_json::json!({
                        "kind": "notifications.closed",
                    });
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("closed")
                            .data(closed.to_string())))
                        .await;
                    break;
                }
            }
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn list_notifications_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let unread_only = params.get("unread").map(|v| v == "true").unwrap_or(false);
    let agent = state.agent.read().await;
    let total = agent
        .storage
        .count_notifications(unread_only)
        .await
        .unwrap_or(0);
    match agent
        .storage
        .list_notifications(limit, offset, unread_only)
        .await
    {
        Ok(notifs) => {
            let list: Vec<serde_json::Value> = notifs
                .iter()
                .map(|n| {
                    serde_json::json!({
                        "id": n.id, "title": n.title, "body": n.body,
                        "level": n.level, "source": n.source, "read": n.read,
                        "created_at": n.created_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"notifications": list, "total": total, "limit": limit, "offset": offset}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn mark_read_endpoint(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.mark_notification_read(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn mark_all_read_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.mark_all_notifications_read().await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn notification_count_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.count_unread_notifications().await {
        Ok(count) => (StatusCode::OK, Json(serde_json::json!({"unread": count}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Analytics Endpoints ====================

fn parse_range_param(input: Option<&String>) -> chrono::Duration {
    // Defaults and simple parsing (24h, 7d, 30d, 90d).
    let raw = input
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if raw.is_empty() {
        return chrono::Duration::hours(24);
    }
    let (num, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n = num.parse::<i64>().unwrap_or(24);
    match unit {
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        "w" => chrono::Duration::weeks(n),
        _ => chrono::Duration::hours(24),
    }
}

fn parse_analytics_datetime_param(input: Option<&String>) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = input.map(|s| s.trim()).unwrap_or("");
    if raw.is_empty() {
        return None;
    }
    parse_utc_rfc3339(raw)
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
                .ok()
                .map(|dt| {
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
                })
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| {
                    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
                })
        })
}

fn bucket_start(dt: chrono::DateTime<chrono::Utc>, bucket: &str) -> chrono::DateTime<chrono::Utc> {
    let naive = dt.naive_utc();
    match bucket {
        "day" => chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            naive.date().and_hms_opt(0, 0, 0).unwrap(),
            chrono::Utc,
        ),
        "week" => {
            let date = naive.date();
            let weekday = date.weekday().num_days_from_monday() as i64;
            let start = date.and_hms_opt(0, 0, 0).unwrap() - chrono::Duration::days(weekday);
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(start, chrono::Utc)
        }
        _ => {
            // hour
            let start = naive
                .with_minute(0)
                .and_then(|x| x.with_second(0))
                .and_then(|x| x.with_nanosecond(0))
                .unwrap_or(naive);
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(start, chrono::Utc)
        }
    }
}

#[derive(Debug, Serialize)]
struct LlmAnalyticsTotals {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    request_count: i64,
    estimated_count: i64,
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
struct LlmAnalyticsPoint {
    bucket_start: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    request_count: i64,
    primary_prompt_tokens: i64,
    primary_completion_tokens: i64,
    primary_total_tokens: i64,
    primary_request_count: i64,
    helper_prompt_tokens: i64,
    helper_completion_tokens: i64,
    helper_total_tokens: i64,
    helper_request_count: i64,
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
struct LlmAnalyticsBreakdownRow {
    provider: String,
    model: String,
    channel: Option<String>,
    purpose: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    request_count: i64,
    cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
struct OpenRouterModelPricing {
    prompt_per_token: f64,
    completion_per_token: f64,
}

#[derive(Debug, Clone)]
struct OpenRouterPricingCacheEntry {
    fetched_at: Instant,
    prices: HashMap<String, OpenRouterModelPricing>,
}

static OPENROUTER_PRICING_CACHE: OnceLock<RwLock<Option<OpenRouterPricingCacheEntry>>> =
    OnceLock::new();
const OPENROUTER_PRICING_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

fn openrouter_pricing_cache() -> &'static RwLock<Option<OpenRouterPricingCacheEntry>> {
    OPENROUTER_PRICING_CACHE.get_or_init(|| RwLock::new(None))
}

fn parse_openrouter_price_value(value: &serde_json::Value) -> Option<f64> {
    if let Some(v) = value.as_f64() {
        return Some(v);
    }
    if let Some(v) = value.as_i64() {
        return Some(v as f64);
    }
    value
        .as_str()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

fn add_openrouter_model_aliases(
    prices: &mut HashMap<String, OpenRouterModelPricing>,
    model: &str,
    pricing: OpenRouterModelPricing,
) {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    prices.insert(lower.clone(), pricing.clone());
    if let Some((_, tail)) = lower.rsplit_once('/') {
        prices.entry(tail.to_string()).or_insert(pricing.clone());
    }
    if let Some((_, tail)) = lower.rsplit_once(':') {
        prices.entry(tail.to_string()).or_insert(pricing);
    }
}

async fn fetch_openrouter_pricing(
    api_key: Option<&str>,
) -> std::result::Result<HashMap<String, OpenRouterModelPricing>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Failed to build OpenRouter pricing client: {}", e))?;

    let mut req = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Accept", "application/json")
        .header("HTTP-Referer", "https://github.com/agentark-ai/AgentArk")
        .header("X-Title", "AgentArk");
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key.trim());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("OpenRouter pricing request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "OpenRouter pricing request failed with status {}",
            resp.status()
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter pricing response: {}", e))?;

    let data = body
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "OpenRouter pricing payload missing data array".to_string())?;

    let mut prices: HashMap<String, OpenRouterModelPricing> = HashMap::new();
    for item in data {
        let model_id = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let Some(model_id) = model_id else {
            continue;
        };

        let pricing = item.get("pricing").and_then(|v| v.as_object());
        let Some(pricing) = pricing else {
            continue;
        };

        let prompt_price = pricing
            .get("prompt")
            .or_else(|| pricing.get("input"))
            .and_then(parse_openrouter_price_value);
        let completion_price = pricing
            .get("completion")
            .or_else(|| pricing.get("output"))
            .and_then(parse_openrouter_price_value);

        let (Some(prompt_per_token), Some(completion_per_token)) = (prompt_price, completion_price)
        else {
            continue;
        };

        add_openrouter_model_aliases(
            &mut prices,
            model_id,
            OpenRouterModelPricing {
                prompt_per_token,
                completion_per_token,
            },
        );
    }

    Ok(prices)
}

async fn get_openrouter_pricing_cached(
    api_key: Option<&str>,
) -> HashMap<String, OpenRouterModelPricing> {
    let cache = openrouter_pricing_cache();
    let stale_prices = {
        let guard = cache.read().await;
        if let Some(entry) = guard.as_ref() {
            if entry.fetched_at.elapsed() < OPENROUTER_PRICING_CACHE_TTL {
                return entry.prices.clone();
            }
            Some(entry.prices.clone())
        } else {
            None
        }
    };

    match fetch_openrouter_pricing(api_key).await {
        Ok(prices) if !prices.is_empty() => {
            let mut guard = cache.write().await;
            *guard = Some(OpenRouterPricingCacheEntry {
                fetched_at: Instant::now(),
                prices: prices.clone(),
            });
            prices
        }
        Ok(_) => stale_prices.unwrap_or_default(),
        Err(e) => {
            tracing::warn!("OpenRouter pricing fetch failed: {}", e);
            stale_prices.unwrap_or_default()
        }
    }
}

fn add_model_aliases(models: &mut HashSet<String>, model: &str) {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    models.insert(lower.clone());
    if let Some((_, tail)) = lower.rsplit_once('/') {
        models.insert(tail.to_string());
    }
    if let Some((_, tail)) = lower.rsplit_once(':') {
        models.insert(tail.to_string());
    }
}

fn collect_openrouter_metadata(agent: &Agent) -> (Option<String>, HashSet<String>) {
    let mut openrouter_api_key: Option<String> = None;
    let mut openrouter_models: HashSet<String> = HashSet::new();

    let mut capture_provider = |provider: &LlmProvider| {
        if let LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } = provider
        {
            if base_url
                .as_deref()
                .map(is_openrouter_base_url)
                .unwrap_or(false)
            {
                if openrouter_api_key.is_none() && !api_key.trim().is_empty() {
                    openrouter_api_key = Some(api_key.trim().to_string());
                }
                add_model_aliases(&mut openrouter_models, model);
            }
        }
    };

    capture_provider(&agent.config.llm);
    if let Some(fallback) = agent.config.llm_fallback.as_ref() {
        capture_provider(fallback);
    }
    for slot in &agent.config.model_pool.slots {
        capture_provider(&slot.provider);
    }

    if openrouter_api_key.is_none() {
        if let Ok(env_key) = std::env::var("OPENROUTER_API_KEY") {
            let trimmed = env_key.trim();
            if !trimmed.is_empty() {
                openrouter_api_key = Some(trimmed.to_string());
            }
        }
    }

    (openrouter_api_key, openrouter_models)
}

fn normalize_analytics_provider(
    provider: &str,
    model: &str,
    openrouter_models: &HashSet<String>,
) -> String {
    let provider = provider.trim().to_ascii_lowercase();
    if provider == "openrouter" {
        return "openrouter".to_string();
    }
    if provider != "openai-compatible" {
        return provider;
    }

    let mut aliases: Vec<String> = Vec::new();
    let model_lower = model.trim().to_ascii_lowercase();
    if !model_lower.is_empty() {
        aliases.push(model_lower.clone());
        if let Some((_, tail)) = model_lower.rsplit_once('/') {
            aliases.push(tail.to_string());
        }
        if let Some((_, tail)) = model_lower.rsplit_once(':') {
            aliases.push(tail.to_string());
        }
    }

    if aliases.into_iter().any(|m| openrouter_models.contains(&m)) {
        "openrouter".to_string()
    } else {
        "openai-compatible".to_string()
    }
}

fn find_openrouter_pricing<'a>(
    model: &str,
    prices: &'a HashMap<String, OpenRouterModelPricing>,
) -> Option<&'a OpenRouterModelPricing> {
    let lower = model.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if let Some(p) = prices.get(&lower) {
        return Some(p);
    }
    if let Some((_, tail)) = lower.rsplit_once('/') {
        if let Some(p) = prices.get(tail) {
            return Some(p);
        }
    }
    if let Some((_, tail)) = lower.rsplit_once(':') {
        if let Some(p) = prices.get(tail) {
            return Some(p);
        }
    }
    if !lower.contains('/') {
        if let Some((_, p)) = prices
            .iter()
            .find(|(id, _)| id.ends_with(&format!("/{}", lower)))
        {
            return Some(p);
        }
    }
    None
}

fn estimate_cost_usd(
    provider: &str,
    model: &str,
    prompt: i64,
    completion: i64,
    openrouter_prices: &HashMap<String, OpenRouterModelPricing>,
) -> Option<f64> {
    let p = provider.trim().to_ascii_lowercase();
    if p == "ollama" {
        return Some(0.0);
    }
    if p == "openrouter" || p == "openai-compatible" {
        if let Some(pricing) = find_openrouter_pricing(model, openrouter_prices) {
            let prompt_tokens = prompt.max(0) as f64;
            let completion_tokens = completion.max(0) as f64;
            return Some(
                prompt_tokens * pricing.prompt_per_token
                    + completion_tokens * pricing.completion_per_token,
            );
        }
    }
    // Default pricing map (USD per 1M tokens). This is a heuristic and may be overridden later.
    // Unknown models return None.
    let m = model.trim().to_ascii_lowercase();
    let (in_per_1m, out_per_1m) = if p == "openai" {
        if m.starts_with("gpt-4o-mini") {
            (0.15, 0.60)
        } else if m.starts_with("gpt-4o") {
            (5.0, 15.0)
        } else {
            return None;
        }
    } else if p == "anthropic" {
        if m.contains("sonnet") {
            (3.0, 15.0)
        } else if m.contains("haiku") {
            (0.25, 1.25)
        } else {
            return None;
        }
    } else {
        return None;
    };

    let cost =
        (prompt as f64 / 1_000_000.0) * in_per_1m + (completion as f64 / 1_000_000.0) * out_per_1m;
    Some(cost)
}

fn analytics_purpose_kind(channel: &str, purpose: &str) -> &'static str {
    let channel = channel.trim().to_ascii_lowercase();
    let purpose = purpose.trim().to_ascii_lowercase();
    if purpose.is_empty() {
        return "primary";
    }

    let helper_exact = [
        "title",
        "smalltalk_classifier",
        "request_shape",
        "action_selector",
        "explicit_approval_classifier",
        "skill_import_override_classifier",
        "user_fact_fast_path",
        "user_fact_memory_capture",
        "argument_inference",
        "custom_condition",
    ];

    if helper_exact.contains(&purpose.as_str())
        || purpose.contains("classifier")
        || purpose.ends_with("_selector")
        || purpose.contains("request_shape")
        || purpose.contains("memory_capture")
        || purpose.contains("argument_inference")
        || purpose.contains("custom_condition")
    {
        return "helper";
    }

    if matches!(channel.as_str(), "system" | "watcher" | "automation")
        && !matches!(
            purpose.as_str(),
            "chat" | "chat_tool_followup" | "chat_tool_synthesis" | "chat_tool_repair"
        )
    {
        return "helper";
    }

    "primary"
}

async fn llm_analytics_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let range = parse_range_param(params.get("range"));
    let bucket = params
        .get("bucket")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "hour".to_string());
    let bucket = match bucket.as_str() {
        "hour" | "day" | "week" => bucket,
        _ => "hour".to_string(),
    };

    let now = chrono::Utc::now();
    let mut since = parse_analytics_datetime_param(params.get("from")).unwrap_or(now - range);
    let mut until = parse_analytics_datetime_param(params.get("to")).unwrap_or(now);
    if since > until {
        std::mem::swap(&mut since, &mut until);
    }
    let since_rfc3339 = since.to_rfc3339();

    let agent = state.agent.read().await;
    let rows = match agent.storage.list_llm_usage_since(&since_rfc3339).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };
    let (openrouter_api_key, openrouter_models) = collect_openrouter_metadata(&agent);
    drop(agent);

    let has_openrouter_like_rows = rows.iter().any(|r| {
        let provider = r.provider.trim().to_ascii_lowercase();
        provider == "openrouter" || provider == "openai-compatible"
    });
    let openrouter_prices = if has_openrouter_like_rows {
        get_openrouter_pricing_cached(openrouter_api_key.as_deref()).await
    } else {
        HashMap::new()
    };

    use std::collections::BTreeMap;
    let mut series: BTreeMap<String, LlmAnalyticsPoint> = BTreeMap::new();
    let mut by_model: std::collections::HashMap<(String, String), LlmAnalyticsBreakdownRow> =
        std::collections::HashMap::new();
    let mut by_channel: std::collections::HashMap<String, LlmAnalyticsBreakdownRow> =
        std::collections::HashMap::new();
    let mut by_purpose: std::collections::HashMap<String, LlmAnalyticsBreakdownRow> =
        std::collections::HashMap::new();

    let mut totals = LlmAnalyticsTotals {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        request_count: 0,
        estimated_count: 0,
        cost_usd: Some(0.0),
    };

    for r in rows {
        let dt = parse_utc_rfc3339(&r.created_at).unwrap_or_else(chrono::Utc::now);
        if dt < since || dt > until {
            continue;
        }
        let bstart = bucket_start(dt, &bucket);
        let key = bstart.to_rfc3339();
        let provider = normalize_analytics_provider(&r.provider, &r.model, &openrouter_models);
        let cost = estimate_cost_usd(
            &provider,
            &r.model,
            r.prompt_tokens,
            r.completion_tokens,
            &openrouter_prices,
        );

        totals.prompt_tokens += r.prompt_tokens;
        totals.completion_tokens += r.completion_tokens;
        totals.total_tokens += r.total_tokens;
        totals.request_count += 1;
        if r.estimated {
            totals.estimated_count += 1;
        }
        match (&mut totals.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => totals.cost_usd = None,
            (None, _) => {}
        }

        let entry = series
            .entry(key.clone())
            .or_insert_with(|| LlmAnalyticsPoint {
                bucket_start: key.clone(),
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                primary_prompt_tokens: 0,
                primary_completion_tokens: 0,
                primary_total_tokens: 0,
                primary_request_count: 0,
                helper_prompt_tokens: 0,
                helper_completion_tokens: 0,
                helper_total_tokens: 0,
                helper_request_count: 0,
                cost_usd: Some(0.0),
            });
        entry.prompt_tokens += r.prompt_tokens;
        entry.completion_tokens += r.completion_tokens;
        entry.total_tokens += r.total_tokens;
        entry.request_count += 1;
        match analytics_purpose_kind(&r.channel, &r.purpose) {
            "helper" => {
                entry.helper_prompt_tokens += r.prompt_tokens;
                entry.helper_completion_tokens += r.completion_tokens;
                entry.helper_total_tokens += r.total_tokens;
                entry.helper_request_count += 1;
            }
            _ => {
                entry.primary_prompt_tokens += r.prompt_tokens;
                entry.primary_completion_tokens += r.completion_tokens;
                entry.primary_total_tokens += r.total_tokens;
                entry.primary_request_count += 1;
            }
        }
        match (&mut entry.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => entry.cost_usd = None,
            (None, _) => {}
        }

        let mk = (provider.clone(), r.model.clone());
        let model_row = by_model
            .entry(mk.clone())
            .or_insert_with(|| LlmAnalyticsBreakdownRow {
                provider: mk.0.clone(),
                model: mk.1.clone(),
                channel: None,
                purpose: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                cost_usd: Some(0.0),
            });
        model_row.prompt_tokens += r.prompt_tokens;
        model_row.completion_tokens += r.completion_tokens;
        model_row.total_tokens += r.total_tokens;
        model_row.request_count += 1;
        match (&mut model_row.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => model_row.cost_usd = None,
            (None, _) => {}
        }

        let ch = r.channel.clone();
        let ch_row = by_channel
            .entry(ch.clone())
            .or_insert_with(|| LlmAnalyticsBreakdownRow {
                provider: "".to_string(),
                model: "".to_string(),
                channel: Some(ch.clone()),
                purpose: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                cost_usd: Some(0.0),
            });
        ch_row.prompt_tokens += r.prompt_tokens;
        ch_row.completion_tokens += r.completion_tokens;
        ch_row.total_tokens += r.total_tokens;
        ch_row.request_count += 1;
        match (&mut ch_row.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => ch_row.cost_usd = None,
            (None, _) => {}
        }

        let pur = r.purpose.clone();
        let pur_row = by_purpose
            .entry(pur.clone())
            .or_insert_with(|| LlmAnalyticsBreakdownRow {
                provider: "".to_string(),
                model: "".to_string(),
                channel: None,
                purpose: Some(pur.clone()),
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                cost_usd: Some(0.0),
            });
        pur_row.prompt_tokens += r.prompt_tokens;
        pur_row.completion_tokens += r.completion_tokens;
        pur_row.total_tokens += r.total_tokens;
        pur_row.request_count += 1;
        match (&mut pur_row.cost_usd, cost) {
            (Some(sum), Some(c)) => *sum += c,
            (Some(_), None) => pur_row.cost_usd = None,
            (None, _) => {}
        }
    }

    let mut by_model_list: Vec<LlmAnalyticsBreakdownRow> = by_model.into_values().collect();
    by_model_list.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    let mut by_channel_list: Vec<LlmAnalyticsBreakdownRow> = by_channel.into_values().collect();
    by_channel_list.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    let mut by_purpose_list: Vec<LlmAnalyticsBreakdownRow> = by_purpose.into_values().collect();
    by_purpose_list.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "range": { "since": since.to_rfc3339(), "until": until.to_rfc3339(), "bucket": bucket },
            "totals": totals,
            "series": series.into_values().collect::<Vec<_>>(),
            "by_model": by_model_list,
            "by_channel": by_channel_list,
            "by_purpose": by_purpose_list,
        })),
    )
        .into_response()
}

// ==================== Document Endpoints ====================

async fn list_documents_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params.get("project_id").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    let total = agent.storage.count_documents(project_id).await.unwrap_or(0);
    match agent
        .storage
        .list_documents(limit, offset, project_id)
        .await
    {
        Ok(docs) => {
            let list: Vec<serde_json::Value> = docs
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id, "filename": d.filename, "content_type": d.content_type,
                        "project_id": d.project_id, "chunk_count": d.chunk_count,
                        "file_size": d.file_size, "created_at": d.created_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"documents": list, "total": total, "limit": limit, "offset": offset}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn delete_document_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_document(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

fn sanitize_document_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_').trim_matches('.').to_string();
    if trimmed.is_empty() {
        "document.txt".to_string()
    } else {
        trimmed
    }
}

fn decode_xml_entities(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid DOCX archive: {}", e))?;
    let mut doc_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| "DOCX is missing word/document.xml".to_string())?;
    let mut xml = String::new();
    doc_xml
        .read_to_string(&mut xml)
        .map_err(|e| format!("Failed to read DOCX XML: {}", e))?;

    let normalized = xml
        .replace("<w:tab/>", "\t")
        .replace("<w:br/>", "\n")
        .replace("<w:cr/>", "\n")
        .replace("</w:p>", "\n")
        .replace("</w:tr>", "\n")
        .replace("</w:tc>", "\t");
    let without_tags = regex::Regex::new(r"<[^>]+>")
        .map_err(|e| format!("Regex error while parsing DOCX: {}", e))?
        .replace_all(&normalized, "");
    Ok(decode_xml_entities(&without_tags).trim().to_string())
}

fn extract_document_text(
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let lower_name = filename.to_ascii_lowercase();
    let ext = lower_name.rsplit('.').next().unwrap_or("");
    let lower_ct = content_type.to_ascii_lowercase();

    let looks_pdf = ext == "pdf" || lower_ct == "application/pdf";
    if looks_pdf {
        return pdf_extract::extract_text_from_mem(bytes)
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("Failed to parse PDF: {}", e));
    }

    let looks_docx = ext == "docx"
        || lower_ct
            .contains("application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    if looks_docx {
        return extract_docx_text(bytes);
    }

    if ext == "doc" {
        return Err(
            "Legacy .doc files are not supported yet. Please save as .docx or .txt.".to_string(),
        );
    }

    let text_exts = [
        "txt", "md", "markdown", "json", "csv", "tsv", "xml", "html", "htm", "yaml", "yml", "log",
        "ini", "toml", "sql", "js", "ts", "tsx", "jsx", "py", "rs", "go", "java", "c", "cpp", "h",
        "hpp", "sh", "bat", "ps1",
    ];
    let likely_text = lower_ct.starts_with("text/")
        || lower_ct.contains("json")
        || lower_ct.contains("xml")
        || lower_ct.contains("yaml")
        || text_exts.contains(&ext);
    if likely_text {
        return String::from_utf8(bytes.to_vec())
            .or_else(|_| Ok(String::from_utf8_lossy(bytes).to_string()))
            .map(|s| s.trim().to_string());
    }

    Err(format!(
        "Unsupported file type '{}'. Supported: txt/md/json/csv/xml/yaml, PDF, DOCX.",
        content_type
    ))
}

async fn insert_document_from_text(
    agent: &Agent,
    filename: String,
    content_type: String,
    project_id: Option<String>,
    content: String,
) -> Result<(String, usize), String> {
    // Chunk the content (simple fixed-size chunking)
    let chunk_size = 1000; // chars per chunk
    let chunks: Vec<String> = content
        .chars()
        .collect::<Vec<_>>()
        .chunks(chunk_size)
        .map(|c| c.iter().collect())
        .collect();

    let doc_id = uuid::Uuid::new_v4().to_string();
    let doc = crate::storage::entities::document::Model {
        id: doc_id.clone(),
        filename: filename.clone(),
        content_type,
        project_id,
        chunk_count: chunks.len() as i32,
        file_size: content.len() as i64,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    agent
        .storage
        .insert_document(&doc)
        .await
        .map_err(|e| e.to_string())?;

    // Insert chunks
    for (i, chunk_content) in chunks.iter().enumerate() {
        let chunk = crate::storage::entities::document_chunk::Model {
            id: uuid::Uuid::new_v4().to_string(),
            document_id: doc_id.clone(),
            chunk_index: i as i32,
            content: chunk_content.clone(),
            embedding: None,
        };
        if let Err(e) = agent.storage.insert_document_chunk(&chunk).await {
            tracing::warn!("Failed to insert chunk {}: {}", i, e);
        }
    }

    // Emit notification
    agent
        .emit_notification(
            &format!("Document uploaded: {}", filename),
            &format!("{} chunks indexed", chunks.len()),
            "info",
            "documents",
        )
        .await;

    Ok((doc_id, chunks.len()))
}

/// Upload a document (JSON body with already-extracted text content)
async fn upload_document_endpoint(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let filename = match request.get("filename").and_then(|v| v.as_str()) {
        Some(f) => sanitize_document_filename(f),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "filename required".to_string(),
                }),
            )
                .into_response()
        }
    };
    let content = match request.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "content required".to_string(),
                }),
            )
                .into_response()
        }
    };
    if content.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "content is empty after parsing".to_string(),
            }),
        )
            .into_response();
    }
    let project_id = request
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content_type = request
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text/plain")
        .to_string();

    let agent = state.agent.read().await;
    match insert_document_from_text(&agent, filename.clone(), content_type, project_id, content)
        .await
    {
        Ok((doc_id, chunks)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": doc_id,
                "filename": filename,
                "chunks": chunks,
                "status": "ok"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

/// Upload a binary/text document using multipart form-data and extract text server-side.
/// Expected fields:
/// - file (required)
/// - project_id (optional)
/// - filename (optional override)
/// - content_type (optional override)
async fn upload_document_file_endpoint(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let mut filename_override: Option<String> = None;
    let mut content_type_override: Option<String> = None;
    let mut project_id: Option<String> = None;
    let mut uploaded_filename: Option<String> = None;
    let mut uploaded_content_type = "application/octet-stream".to_string();
    let mut uploaded_bytes: Option<Vec<u8>> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "project_id" => match field.text().await {
                Ok(v) => {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        project_id = Some(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid project_id field: {}", e),
                        }),
                    )
                        .into_response()
                }
            },
            "filename" => match field.text().await {
                Ok(v) => {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        filename_override = Some(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid filename override field: {}", e),
                        }),
                    )
                        .into_response()
                }
            },
            "content_type" => match field.text().await {
                Ok(v) => {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        content_type_override = Some(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid content_type override field: {}", e),
                        }),
                    )
                        .into_response()
                }
            },
            _ => {
                // Treat first non-metadata field as uploaded file payload.
                if uploaded_bytes.is_some() {
                    continue;
                }
                uploaded_filename = field.file_name().map(|s| s.to_string());
                if let Some(ct) = field.content_type() {
                    uploaded_content_type = ct.to_string();
                }
                match field.bytes().await {
                    Ok(bytes) => {
                        if bytes.len() > 50 * 1024 * 1024 {
                            return (
                                StatusCode::PAYLOAD_TOO_LARGE,
                                Json(ErrorResponse {
                                    error: "File too large (50MB max)".to_string(),
                                }),
                            )
                                .into_response();
                        }
                        uploaded_bytes = Some(bytes.to_vec());
                    }
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: format!("Failed to read uploaded file: {}", e),
                            }),
                        )
                            .into_response()
                    }
                }
            }
        }
    }

    let bytes = match uploaded_bytes {
        Some(b) => b,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No file uploaded. Expected multipart field 'file'.".to_string(),
                }),
            )
                .into_response()
        }
    };

    let raw_filename = filename_override
        .or(uploaded_filename)
        .unwrap_or_else(|| "document.txt".to_string());
    let filename = sanitize_document_filename(&raw_filename);
    let content_type = content_type_override.unwrap_or(uploaded_content_type);
    let extracted = match extract_document_text(&filename, &content_type, &bytes) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Parsed document content is empty".to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response()
        }
    };

    let agent = state.agent.read().await;
    match insert_document_from_text(
        &agent,
        filename.clone(),
        content_type.clone(),
        project_id,
        extracted,
    )
    .await
    {
        Ok((doc_id, chunks)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": doc_id,
                "filename": filename,
                "content_type": content_type,
                "chunks": chunks,
                "status": "ok"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

/// Search within a specific document
async fn search_document_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let query = match params.get("q") {
        Some(q) => q.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "query parameter 'q' required".to_string(),
                }),
            )
                .into_response()
        }
    };
    let agent = state.agent.read().await;
    match agent.storage.get_document_chunks(&id).await {
        Ok(chunks) => {
            let query_lower = query.to_lowercase();
            let mut results: Vec<serde_json::Value> = chunks
                .into_iter()
                .filter(|c| c.content.to_lowercase().contains(&query_lower))
                .map(|c| {
                    serde_json::json!({
                        "chunk_index": c.chunk_index,
                        "content": c.content,
                    })
                })
                .collect();
            results.truncate(10);
            (
                StatusCode::OK,
                Json(serde_json::json!({"results": results})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Memory Endpoints ====================

async fn trigger_consolidation(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let llm = agent.llm.clone();
    match agent.memory.run_llm_consolidation(&llm).await {
        Ok(summary) => {
            let event_id = uuid::Uuid::new_v4().to_string();
            agent
                .hooks
                .fire(
                    hooks::HookTrigger::OnConsolidate,
                    hooks::HookContext {
                        event_id: Some(event_id),
                        trigger: "on_consolidate".to_string(),
                        channel: "system".to_string(),
                        message: Some("memory consolidation completed".to_string()),
                        response: Some(summary.clone()),
                        action: None,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    },
                )
                .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "summary": summary})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn memory_stats(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params.get("project_id").map(|s| s.as_str());
    let agent = state.agent.read().await;
    let episode_count = agent
        .storage
        .count_episodes_by_project(project_id)
        .await
        .unwrap_or(0);
    let fact_count = agent.storage.count_facts(project_id).await.unwrap_or(0);
    let doc_count = agent.storage.count_documents(project_id).await.unwrap_or(0);
    let preference_count = agent
        .storage
        .count_user_preferences(project_id)
        .await
        .unwrap_or(0);
    let user_data_count = agent
        .storage
        .count_user_data_items(project_id, None)
        .await
        .unwrap_or(0);
    let knowledge_count = agent
        .storage
        .count_knowledge_items(project_id)
        .await
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "episodes": episode_count,
            "facts": fact_count,
            "documents": doc_count,
            "preferences": preference_count,
            "user_data": user_data_count,
            "knowledge": knowledge_count,
        })),
    )
        .into_response()
}

/// List memory episodes (paginated)
async fn list_episodes(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id = params.get("project_id").map(|s| s.as_str());
    let agent = state.agent.read().await;
    match agent
        .encrypted_storage
        .get_episodes_by_project_decrypted(limit, offset, project_id)
        .await
    {
        Ok(episodes) => {
            let total = agent
                .storage
                .count_episodes_by_project(project_id)
                .await
                .unwrap_or(0);
            let items: Vec<serde_json::Value> = episodes
                .iter()
                .map(|ep| {
                    serde_json::json!({
                        "id": ep.id,
                        "content": ep.content,
                        "context": ep.context,
                        "timestamp": ep.timestamp,
                        "consolidated": ep.consolidated,
                        "importance": ep.importance,
                        "access_count": ep.access_count,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "episodes": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// List semantic facts
async fn list_facts(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params.get("project_id").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    let total = agent.storage.count_facts(project_id).await.unwrap_or(0);
    match agent
        .encrypted_storage
        .get_facts_by_project_decrypted(limit, offset, project_id)
        .await
    {
        Ok(facts) => {
            let items: Vec<serde_json::Value> = facts
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "id": f.id,
                        "fact": f.fact,
                        "confidence": f.confidence,
                        "sources": f.sources,
                        "created_at": f.created_at,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "facts": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct UpsertUserPreferenceRequest {
    key: String,
    value: String,
    confidence: Option<f32>,
    source: Option<String>,
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateUserDataItemRequest {
    kind: String,
    title: String,
    content: String,
    url: Option<String>,
    source_channel: Option<String>,
    conversation_id: Option<String>,
    project_id: Option<String>,
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateKnowledgeItemRequest {
    title: String,
    content: String,
    source: Option<String>,
    url: Option<String>,
    tags: Option<String>,
    project_id: Option<String>,
}

async fn list_user_preferences(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id = params.get("project_id").map(|s| s.as_str());

    let agent = state.agent.read().await;
    match agent
        .storage
        .list_user_preferences(limit, offset, project_id)
        .await
    {
        Ok(items) => {
            let total = agent
                .storage
                .count_user_preferences(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "preferences": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn upsert_user_preference(
    State(state): State<AppState>,
    Json(payload): Json<UpsertUserPreferenceRequest>,
) -> Response {
    if payload.key.trim().is_empty() || payload.value.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "key and value are required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    match agent
        .storage
        .upsert_user_preference(
            payload.key.trim(),
            payload.value.trim(),
            payload.confidence.unwrap_or(0.85),
            payload.source.as_deref(),
            payload
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
        )
        .await
    {
        Ok(item) => (
            StatusCode::OK,
            Json(serde_json::json!({"preference": item})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn delete_user_preference(
    State(state): State<AppState>,
    Path(key): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let agent = state.agent.read().await;
    match agent.storage.delete_user_preference(&key, project_id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn list_user_data_items(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id = params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let kind = params
        .get("kind")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let agent = state.agent.read().await;
    match agent
        .storage
        .list_user_data_items(limit, offset, project_id, kind)
        .await
    {
        Ok(items) => {
            let total = agent
                .storage
                .count_user_data_items(project_id, kind)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn create_user_data_item(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserDataItemRequest>,
) -> Response {
    if payload.kind.trim().is_empty() || payload.title.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "kind and title are required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    match agent
        .storage
        .create_user_data_item(crate::storage::NewUserDataItem {
            kind: payload.kind.trim(),
            title: payload.title.trim(),
            content: payload.content.trim(),
            url: payload.url.as_deref(),
            source_channel: payload.source_channel.as_deref(),
            conversation_id: payload.conversation_id.as_deref(),
            project_id: payload
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
            pinned: payload.pinned.unwrap_or(false),
        })
        .await
    {
        Ok(item) => (StatusCode::OK, Json(serde_json::json!({"item": item}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn delete_user_data_item(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_user_data_item(&id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn list_knowledge_items(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id = params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let agent = state.agent.read().await;
    match agent
        .storage
        .list_knowledge_items(limit, offset, project_id)
        .await
    {
        Ok(items) => {
            let total = agent
                .storage
                .count_knowledge_items(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn create_knowledge_item(
    State(state): State<AppState>,
    Json(payload): Json<CreateKnowledgeItemRequest>,
) -> Response {
    if payload.title.trim().is_empty() || payload.content.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "title and content are required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    match agent
        .storage
        .create_knowledge_item(
            payload.title.trim(),
            payload.content.trim(),
            payload.source.as_deref(),
            payload.url.as_deref(),
            payload.tags.as_deref(),
            payload
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
        )
        .await
    {
        Ok(item) => (StatusCode::OK, Json(serde_json::json!({"item": item}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn delete_knowledge_item(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_knowledge_item(&id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Execute code in an isolated sandbox
async fn execute_code(
    State(state): State<AppState>,
    Json(request): Json<CodeExecuteRequest>,
) -> Response {
    let arguments = serde_json::json!({
        "language": request.language,
        "code": request.code,
        "env": request.env,
        "files": request.files,
    });

    let result = {
        let agent_guard = state.agent.read().await;
        agent_guard
            .runtime
            .execute_action("code_execute", &arguments)
            .await
    };

    match result {
        Ok(output_json) => {
            // The action returns a JSON string; parse it for a clean response
            match serde_json::from_str::<serde_json::Value>(&output_json) {
                Ok(parsed) => {
                    let files = parsed["files"].as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    });
                    let resp = CodeExecuteResponse {
                        output: parsed["output"].as_str().unwrap_or("").to_string(),
                        exit_code: parsed["exit_code"].as_i64().unwrap_or(-1),
                        error: parsed["error"].as_str().map(|s| s.to_string()),
                        files,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
                Err(_) => {
                    // Fallback: return raw output
                    let resp = CodeExecuteResponse {
                        output: output_json,
                        exit_code: 0,
                        error: None,
                        files: None,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
            }
        }
        Err(e) => {
            let resp = CodeExecuteResponse {
                output: String::new(),
                exit_code: -1,
                error: Some(e.to_string()),
                files: None,
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(resp)).into_response()
        }
    }
}

// ==================== MCP (Model Context Protocol) Endpoints ====================

async fn mcp_handler(
    State(state): State<AppState>,
    Json(request): Json<crate::mcp::McpRequest>,
) -> Response {
    let mcp = crate::mcp::McpServer::new();

    // Handle tool calls that need agent access
    if request.method == "tools/call" {
        let tool_name = request
            .params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let args = request
            .params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let result = match tool_name {
            "chat" => {
                let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let channel = args
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("mcp");
                let conversation_id = args.get("conversation_id").and_then(|v| v.as_str());
                let project_id = args.get("project_id").and_then(|v| v.as_str());
                let agent = state.agent.read().await;
                match agent
                    .process_message_with_meta(message, channel, conversation_id, project_id)
                    .await
                {
                    Ok(processed) => serde_json::json!({
                        "content": [{ "type": "text", "text": processed.response }],
                        "conversation_id": processed.conversation_id,
                        "conversation_title": processed.conversation_title,
                    }),
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "memory_search" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let agent = state.agent.read().await;
                match agent.memory.retrieve_relevant(query, limit, None).await {
                    Ok(memories) => {
                        let results: Vec<serde_json::Value> = memories.iter().map(|m| {
                            serde_json::json!({ "content": m.content, "score": m.final_score, "timestamp": m.timestamp.to_rfc3339() })
                        }).collect();
                        serde_json::json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&results).unwrap_or_default() }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "document_search" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let agent = state.agent.read().await;
                match agent.search_documents(query, limit).await {
                    Ok(results) => {
                        let items: Vec<serde_json::Value> = results.iter().map(|(doc_id, content, score)| {
                            serde_json::json!({ "document_id": doc_id, "content": content, "score": score })
                        }).collect();
                        serde_json::json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&items).unwrap_or_default() }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "list_actions" => {
                let agent = state.agent.read().await;
                match agent.runtime.list_actions().await {
                    Ok(actions) => {
                        let items: Vec<serde_json::Value> = actions.iter().map(|a| {
                            serde_json::json!({ "name": a.name, "description": a.description })
                        }).collect();
                        serde_json::json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&items).unwrap_or_default() }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "execute_action" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let action_args = args
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let agent = state.agent.read().await;
                match agent.runtime.execute_action(action, &action_args).await {
                    Ok(result) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": result }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            _ => {
                serde_json::json!({ "content": [{ "type": "text", "text": format!("Unknown tool: {}", tool_name) }], "isError": true })
            }
        };

        return (
            StatusCode::OK,
            Json(crate::mcp::McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            }),
        )
            .into_response();
    }

    // Handle non-tool-call methods
    let response = mcp.handle_request(&request);
    (StatusCode::OK, Json(response)).into_response()
}

async fn mcp_list_tools() -> Json<serde_json::Value> {
    let mcp = crate::mcp::McpServer::new();
    Json(
        serde_json::json!({ "tools": mcp.handle_request(&crate::mcp::McpRequest {
        _jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        method: "tools/list".to_string(),
        params: serde_json::json!({}),
    }).result }),
    )
}

#[derive(Debug, Deserialize)]
struct McpListQuery {
    #[serde(default)]
    include_details: bool,
}

/// List MCP servers (client-side connections)
async fn list_mcp_servers(
    State(state): State<AppState>,
    Query(query): Query<McpListQuery>,
) -> Response {
    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "servers": registry.list_servers(query.include_details),
        })),
    )
        .into_response()
}

/// Get a specific MCP server
async fn get_mcp_server(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    if let Some(server) = registry.get_server(&id, true) {
        (StatusCode::OK, Json(server)).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "MCP server not found".to_string(),
            }),
        )
            .into_response()
    }
}

/// Create a new MCP server
async fn create_mcp_server(
    State(state): State<AppState>,
    Json(request): Json<McpServerRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let server_id = request
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if agent.config.mcp.servers.iter().any(|s| s.id == server_id) {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "MCP server ID already exists".to_string(),
            }),
        )
            .into_response();
    }

    let existing = None;
    let (config, auth_update) = match build_mcp_config(&request, &server_id, existing) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    agent.config.mcp.servers.push(config);

    if let Err(e) = save_mcp_secrets(&mut agent, &server_id, auth_update) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save MCP secrets: {}", e),
            }),
        )
            .into_response();
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    drop(agent);
    schedule_mcp_registry_sync(state.agent.clone());

    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    let server = registry.get_server(&server_id, true);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "server": server,
            "sync_queued": true,
        })),
    )
        .into_response()
}

/// Update an MCP server
async fn update_mcp_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpServerRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let idx = match agent.config.mcp.servers.iter().position(|s| s.id == id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "MCP server not found".to_string(),
                }),
            )
                .into_response();
        }
    };

    let existing = agent.config.mcp.servers.get(idx).cloned();
    let (config, auth_update) = match build_mcp_config(&request, &id, existing.as_ref()) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    agent.config.mcp.servers[idx] = config;

    if let Err(e) = save_mcp_secrets(&mut agent, &id, auth_update) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save MCP secrets: {}", e),
            }),
        )
            .into_response();
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    drop(agent);
    schedule_mcp_registry_sync(state.agent.clone());

    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    let server = registry.get_server(&id, true);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "server": server, "sync_queued": true })),
    )
        .into_response()
}

/// Delete an MCP server
async fn delete_mcp_server(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mut agent = state.agent.write().await;
    let before = agent.config.mcp.servers.len();
    agent.config.mcp.servers.retain(|s| s.id != id);
    if agent.config.mcp.servers.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "MCP server not found".to_string(),
            }),
        )
            .into_response();
    }

    if let Err(e) = clear_mcp_secrets(&mut agent, &id) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to remove MCP secrets: {}", e),
            }),
        )
            .into_response();
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    drop(agent);
    schedule_mcp_registry_sync(state.agent.clone());

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "sync_queued": true })),
    )
        .into_response()
}

/// Refresh MCP server tools/resources
async fn refresh_mcp_server(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    {
        let agent = state.agent.read().await;
        if agent.config.mcp.servers.iter().all(|s| s.id != id) {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "MCP server not found".to_string(),
                }),
            )
                .into_response();
        }
    }

    schedule_mcp_server_refresh(state.agent.clone(), id.clone());

    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    let server = registry.get_server(&id, true);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "refresh_queued",
            "server": server,
            "server_id": id,
        })),
    )
        .into_response()
}

fn build_mcp_config(
    request: &McpServerRequest,
    server_id: &str,
    existing: Option<&crate::core::config::McpServerConfig>,
) -> Result<(crate::core::config::McpServerConfig, Option<McpAuthUpdate>)> {
    if request.name.trim().is_empty() {
        return Err(anyhow::anyhow!("MCP server name is required"));
    }

    let transport = match &request.transport {
        McpTransportRequest::Http { url } => {
            let parsed = url::Url::parse(url).map_err(|_| anyhow::anyhow!("Invalid MCP URL"))?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(anyhow::anyhow!("MCP URL must be http or https"));
            }
            crate::core::config::McpTransportConfig::Http { url: url.clone() }
        }
        McpTransportRequest::Stdio {
            command,
            args,
            working_dir,
        } => {
            if command.trim().is_empty() {
                return Err(anyhow::anyhow!("MCP stdio command is required"));
            }
            crate::core::config::McpTransportConfig::Stdio {
                command: command.clone(),
                args: args.clone(),
                working_dir: working_dir.clone(),
            }
        }
    };

    let (auth_config, auth_update) = parse_mcp_auth(
        request.auth.as_ref(),
        existing.and_then(|e| e.auth.as_ref()),
    );
    let timeout_secs = request
        .timeout_secs
        .or(existing.map(|e| e.timeout_secs))
        .unwrap_or(15);
    let max_response_bytes = request
        .max_response_bytes
        .or(existing.map(|e| e.max_response_bytes))
        .unwrap_or(1024 * 1024);

    let config = crate::core::config::McpServerConfig {
        id: server_id.to_string(),
        name: request.name.trim().to_string(),
        description: request.description.clone(),
        transport,
        enabled: request.enabled,
        resources_enabled: request.resources_enabled,
        auth: auth_config,
        tool_allowlist: clean_allowlist(&request.tool_allowlist),
        resource_allowlist: clean_allowlist(&request.resource_allowlist),
        timeout_secs,
        max_response_bytes,
    };

    Ok((config, auth_update))
}

fn clean_allowlist(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[derive(Debug)]
struct McpAuthUpdate {
    clear: bool,
    token: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

fn parse_mcp_auth(
    request: Option<&McpAuthRequest>,
    existing: Option<&crate::core::config::McpAuthConfig>,
) -> (
    Option<crate::core::config::McpAuthConfig>,
    Option<McpAuthUpdate>,
) {
    let Some(req) = request else {
        return (existing.cloned(), None);
    };

    match req {
        McpAuthRequest::None { .. } => (
            None,
            Some(McpAuthUpdate {
                clear: true,
                token: None,
                username: None,
                password: None,
            }),
        ),
        McpAuthRequest::Bearer {
            header,
            token,
            clear,
        } => {
            let header = header
                .clone()
                .unwrap_or_else(|| "Authorization".to_string());
            (
                Some(crate::core::config::McpAuthConfig::Bearer { header }),
                Some(McpAuthUpdate {
                    clear: *clear,
                    token: token.clone(),
                    username: None,
                    password: None,
                }),
            )
        }
        McpAuthRequest::Basic {
            username,
            password,
            clear,
        } => (
            Some(crate::core::config::McpAuthConfig::Basic),
            Some(McpAuthUpdate {
                clear: *clear,
                token: None,
                username: username.clone(),
                password: password.clone(),
            }),
        ),
        McpAuthRequest::Header { name, value, clear } => (
            Some(crate::core::config::McpAuthConfig::Header { name: name.clone() }),
            Some(McpAuthUpdate {
                clear: *clear,
                token: value.clone(),
                username: None,
                password: None,
            }),
        ),
        McpAuthRequest::Query { name, value, clear } => (
            Some(crate::core::config::McpAuthConfig::Query { name: name.clone() }),
            Some(McpAuthUpdate {
                clear: *clear,
                token: value.clone(),
                username: None,
                password: None,
            }),
        ),
    }
}

fn should_update_secret(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_some_and(|v| !v.is_empty() && v != "[ENCRYPTED]")
}

fn apply_auth_update(
    secrets: &mut crate::core::config::Secrets,
    server_id: &str,
    update: &McpAuthUpdate,
) {
    if update.clear {
        secrets.mcp_auth.remove(server_id);
        return;
    }

    let mut entry = secrets.mcp_auth.get(server_id).cloned().unwrap_or_default();
    let mut changed = false;

    if should_update_secret(&update.token) {
        entry.token = update.token.clone();
        changed = true;
    }
    if should_update_secret(&update.username) {
        entry.username = update.username.clone();
        changed = true;
    }
    if should_update_secret(&update.password) {
        entry.password = update.password.clone();
        changed = true;
    }

    if changed {
        secrets.mcp_auth.insert(server_id.to_string(), entry);
    }
}

fn save_mcp_secrets(
    agent: &mut Agent,
    server_id: &str,
    update: Option<McpAuthUpdate>,
) -> Result<()> {
    if update.is_none() {
        return Ok(());
    }
    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )?;
    if let Some(update) = update.as_ref() {
        manager.update_secrets(|secrets| {
            apply_auth_update(secrets, server_id, update);
            Ok(())
        })?;
    }
    Ok(())
}

fn load_mcp_secrets(agent: &Agent) -> Result<crate::core::config::Secrets> {
    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )?;
    manager.load_secrets()
}

fn clear_mcp_secrets(agent: &mut Agent, server_id: &str) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )?;
    manager.update_secrets(|secrets| {
        secrets.mcp_auth.remove(server_id);
        Ok(())
    })?;
    Ok(())
}

async fn sync_mcp_registry(agent: &mut Agent, secrets: &crate::core::config::Secrets) {
    let Agent {
        mcp,
        safety,
        runtime,
        config,
        ..
    } = &mut *agent;
    let config_ref = &*config;
    let runtime_ref = &*runtime;
    let mut registry = mcp.write().await;
    let _ = registry
        .sync_from_config(config_ref, secrets, runtime_ref, safety)
        .await;
}

fn schedule_mcp_registry_sync(agent_ref: SharedAgent) {
    tokio::spawn(async move {
        let mut agent = agent_ref.write().await;
        let secrets = match load_mcp_secrets(&agent) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("MCP registry sync skipped: failed to load secrets: {}", e);
                return;
            }
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(20),
            sync_mcp_registry(&mut agent, &secrets),
        )
        .await
        {
            Ok(_) => {}
            Err(_) => tracing::warn!("MCP registry sync timed out after 20s"),
        }
    });
}

fn schedule_mcp_server_refresh(agent_ref: SharedAgent, id: String) {
    tokio::spawn(async move {
        let mut agent = agent_ref.write().await;
        if agent.config.mcp.servers.iter().all(|s| s.id != id) {
            return;
        }
        let Agent {
            mcp,
            runtime,
            safety,
            ..
        } = &mut *agent;
        let refresh_future = async {
            let mut registry = mcp.write().await;
            registry.refresh_server(&id, runtime, safety).await
        };
        match tokio::time::timeout(std::time::Duration::from_secs(20), refresh_future).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("MCP server refresh failed for {}: {}", id, e),
            Err(_) => tracing::warn!("MCP server refresh timed out after 20s for {}", id),
        }
    });
}

// - Hook endpoints -

/// Request to create a new hook
#[derive(Debug, Deserialize)]
pub struct AddHookRequest {
    pub name: String,
    pub trigger: String,
    pub hook_type: String,
    pub url: Option<String>,
    #[serde(default)]
    pub action_name: Option<String>,
}

async fn persist_hooks(agent: &Agent) -> std::result::Result<(), String> {
    let bytes = serde_json::to_vec(&agent.hooks.snapshot()).map_err(|e| e.to_string())?;
    agent
        .storage
        .set(HOOKS_STORAGE_KEY, &bytes)
        .await
        .map_err(|e| e.to_string())
}

/// List all registered hooks
async fn list_hooks(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let hooks_list: Vec<hooks::Hook> = agent.hooks.list_hooks().to_vec();
    (StatusCode::OK, Json(hooks_list)).into_response()
}

/// List recent hook run reports
async fn list_hook_runs(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let agent = state.agent.read().await;
    let runs = agent.hooks.list_runs(limit).await;
    (StatusCode::OK, Json(runs)).into_response()
}

/// Add a new hook
async fn add_hook(State(state): State<AppState>, Json(request): Json<AddHookRequest>) -> Response {
    let trigger: hooks::HookTrigger = match serde_json::from_value(serde_json::Value::String(
        request.trigger.clone(),
    )) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid trigger '{}'. Valid values: pre_message, post_message, pre_action, post_action, on_consolidate, on_error",
                        request.trigger
                    ),
                }),
            )
                .into_response();
        }
    };

    let hook = hooks::Hook {
        id: uuid::Uuid::new_v4().to_string(),
        name: request.name,
        action_name: request
            .action_name
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        trigger,
        hook_type: request.hook_type,
        url: request.url,
        enabled: true,
    };

    let id = hook.id.clone();
    let mut agent = state.agent.write().await;
    agent.hooks.add_hook(hook);
    if let Err(e) = persist_hooks(&agent).await {
        agent.hooks.remove_hook(&id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to persist hook: {}", e),
            }),
        )
            .into_response();
    }

    (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
}

/// Remove a hook by ID
async fn remove_hook(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mut agent = state.agent.write().await;
    let before = agent.hooks.snapshot();
    agent.hooks.remove_hook(&id);
    if let Err(e) = persist_hooks(&agent).await {
        agent.hooks = hooks::HookManager::from_hooks(before);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to persist hook removal: {}", e),
            }),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "removed" })),
    )
        .into_response()
}

// - Locked-Mode Server -

/// State for the locked-mode server (before master password is provided)
#[derive(Clone)]
struct LockedState {
    config_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    /// Channel to send the derived key back to main.rs
    unlock_tx: Arc<
        tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<Arc<crate::crypto::KeyManager>>>>,
    >,
    /// Rate limiter: track failed attempts per IP
    attempts: Arc<RwLock<HashMap<String, (u32, Instant)>>>,
}

/// Start a minimal HTTP server that only serves the unlock page.
/// Blocks until the user provides the correct password, then returns the key.
pub async fn serve_locked(
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Arc<crate::crypto::KeyManager>> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    let locked_state = LockedState {
        config_dir: config_dir.to_path_buf(),
        data_dir: data_dir.to_path_buf(),
        unlock_tx: Arc::new(tokio::sync::Mutex::new(Some(tx))),
        attempts: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(locked_page))
        .route("/health", get(locked_health))
        .route("/unlock", post(handle_unlock))
        .route("/logo.svg", get(serve_logo_svg_locked))
        .with_state(locked_state);

    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    println!();
    println!(" -");
    println!(" - AgentArk is LOCKED -");
    println!(" -");
    println!(" - Open http://{} to unlock -", bind_addr);
    println!(" - Or set AGENTARK_MASTER_PASSWORD env var -");
    println!(" -");
    println!();

    tracing::info!("Locked-mode server listening on {}", bind_addr);

    // Run locked server until unlock succeeds
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    );

    // Race: server vs unlock signal
    tokio::select! {
           result = server => {
               result?;
               Err(anyhow::anyhow!("Locked server exited without unlock"))
           }
           key = rx => {
               let key = key.map_err(|_| anyhow::anyhow!("Unlock channel closed"))?;
    tracing::info!("Master password accepted - proceeding to full startup");
               Ok(key)
           }
       }
}

async fn locked_page() -> Html<&'static str> {
    Html(super::web::UNLOCK_PAGE_HTML)
}

async fn locked_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "locked",
        "message": "Master password required to unlock"
    }))
}

async fn serve_logo_svg_locked() -> Response {
    let svg = include_str!("../../assets/logo.svg");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}

#[derive(Deserialize)]
struct UnlockRequest {
    password: String,
}

async fn handle_unlock(
    State(state): State<LockedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<UnlockRequest>,
) -> Response {
    let ip = addr.ip().to_string();

    // Rate limit: max 5 attempts per minute per IP
    {
        let attempts = state.attempts.read().await;
        if let Some((count, since)) = attempts.get(&ip) {
            if since.elapsed() < std::time::Duration::from_secs(60) && *count >= 5 {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(
                        serde_json::json!({ "error": "Too many attempts. Try again in 1 minute." }),
                    ),
                )
                    .into_response();
            }
        }
    }

    let master_mgr =
        crate::crypto::master::MasterPasswordManager::new(&state.config_dir, &state.data_dir);

    match master_mgr.unlock(&req.password) {
        Ok(key) => {
            // Send key to main.rs via channel
            let mut tx_guard = state.unlock_tx.lock().await;
            if let Some(tx) = tx_guard.take() {
                let _ = tx.send(key);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Unlocked successfully. Starting up..."
                })),
            )
                .into_response()
        }
        Err(_) => {
            // Track failed attempt
            let mut attempts = state.attempts.write().await;
            let entry = attempts.entry(ip).or_insert((0, Instant::now()));
            if entry.1.elapsed() >= std::time::Duration::from_secs(60) {
                *entry = (1, Instant::now());
            } else {
                entry.0 += 1;
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Invalid password"
                })),
            )
                .into_response()
        }
    }
}

// - Security Endpoints -

async fn security_status(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let is_set = master_mgr.is_password_set();
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);

    let warning = if !is_set {
        Some("Encryption keys are stored as plain files. Set a master password for stronger protection.")
    } else if bootstrap_active {
        Some("Using a per-install bootstrap password. Set a custom master password to fully own recovery and rotation.")
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "master_password_set": is_set,
            "custom_master_password_set": is_set && !bootstrap_active,
            "using_default": bootstrap_active,
            "bootstrap_password_active": bootstrap_active,
            "encryption_mode": if is_set { "password" } else { "keyfile" },
            "security_warning": warning
        })),
    )
        .into_response()
}

fn current_runtime_encryption_key(
    config_dir: &FsPath,
) -> anyhow::Result<std::sync::Arc<crate::crypto::KeyManager>> {
    if let Some(key) = crate::core::config::global_key_manager() {
        return Ok(key);
    }
    Ok(std::sync::Arc::new(
        crate::crypto::KeyManager::load_or_create(&config_dir.join(".keyfile"))?,
    ))
}

async fn current_storage_encryption_key(
    state: &AppState,
) -> anyhow::Result<std::sync::Arc<crate::crypto::KeyManager>> {
    let agent = state.agent.read().await;
    Ok(agent.encrypted_storage.current_key_manager())
}

async fn rotate_application_encryption<F>(
    state: &AppState,
    config_dir: &FsPath,
    old_secrets_key: std::sync::Arc<crate::crypto::KeyManager>,
    old_storage_key: std::sync::Arc<crate::crypto::KeyManager>,
    new_key: std::sync::Arc<crate::crypto::KeyManager>,
    commit_metadata: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    let old_mgr = crate::core::config::SecureConfigManager::with_key_manager(
        config_dir,
        old_secrets_key.clone(),
    );
    let new_mgr =
        crate::core::config::SecureConfigManager::with_key_manager(config_dir, new_key.clone());
    let secrets = old_mgr.with_secrets_lock(|manager| manager.load_secrets_unlocked())?;
    let agent = state.agent.write().await;

    new_mgr.save_secrets_unlocked(&secrets)?;
    if let Err(storage_err) = agent
        .encrypted_storage
        .reencrypt_all_sensitive_data(old_storage_key.clone(), new_key.clone())
        .await
    {
        let rollback_err = old_mgr.save_secrets_unlocked(&secrets).err();
        return Err(match rollback_err {
            Some(rollback_err) => anyhow::anyhow!(
                "Encrypted storage rekey failed: {}. secrets.enc rollback also failed: {}",
                storage_err,
                rollback_err
            ),
            None => anyhow::anyhow!("Encrypted storage rekey failed: {}", storage_err),
        });
    }

    if let Err(commit_err) = commit_metadata() {
        let storage_rollback_err = agent
            .encrypted_storage
            .reencrypt_all_sensitive_data(new_key.clone(), old_storage_key)
            .await
            .err();
        let secrets_rollback_err = old_mgr.save_secrets_unlocked(&secrets).err();
        return Err(match (storage_rollback_err, secrets_rollback_err) {
            (Some(storage_rollback_err), Some(secrets_rollback_err)) => anyhow::anyhow!(
                "Metadata update failed: {}. Encrypted storage rollback also failed: {}. secrets.enc rollback also failed: {}",
                commit_err,
                storage_rollback_err,
                secrets_rollback_err
            ),
            (Some(storage_rollback_err), None) => anyhow::anyhow!(
                "Metadata update failed: {}. Encrypted storage rollback also failed: {}",
                commit_err,
                storage_rollback_err
            ),
            (None, Some(secrets_rollback_err)) => anyhow::anyhow!(
                "Metadata update failed: {}. secrets.enc rollback also failed: {}",
                commit_err,
                secrets_rollback_err
            ),
            (None, None) => anyhow::anyhow!("Metadata update failed: {}", commit_err),
        });
    }

    crate::core::config::set_global_key_manager(new_key);
    Ok(())
}

#[derive(Deserialize)]
struct SetPasswordRequest {
    password: String,
}

async fn set_master_password(
    State(state): State<AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> Response {
    if req.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Password must be at least 8 characters"
            })),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);

    if master_mgr.is_password_set() && !bootstrap_active {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Master password already set. Use change-password instead."
            })),
        )
            .into_response();
    }

    let old_secrets_key = match current_runtime_encryption_key(&config_dir) {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };
    let old_storage_key = match current_storage_encryption_key(&state).await {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current storage encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };

    match master_mgr.prepare_password(&req.password) {
        Ok(prepared) => {
            let new_key = prepared.key_manager.clone();
            if let Err(e) = rotate_application_encryption(
                &state,
                &config_dir,
                old_secrets_key,
                old_storage_key,
                new_key.clone(),
                || {
                    master_mgr
                        .commit_prepared_password(prepared)
                        .map_err(anyhow::Error::from)
                },
            )
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to set password safely: {}", e)
                    })),
                )
                    .into_response();
            }

            let _ = tokio::fs::remove_file(data_dir.join("encryption.key")).await;
            let _ = tokio::fs::remove_file(config_dir.join(".keyfile")).await;
            tracing::info!("Master password set and applied in-memory (no restart needed)");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Master password set and applied.",
                    "restart_scheduled": false
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to set password: {}", e)
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

async fn change_master_password(
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> Response {
    if req.new_password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "New password must be at least 8 characters"
            })),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);

    let current_pw = if req.current_password.is_empty() {
        match master_mgr.bootstrap_password_if_active() {
            Ok(Some(pw)) => pw,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "Current password is required"
                    })),
                )
                    .into_response();
            }
        }
    } else {
        req.current_password.clone()
    };

    // Verify current password
    let old_secrets_key = match master_mgr.unlock(&current_pw) {
        Ok(key) => key,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Current password is incorrect"
                })),
            )
                .into_response();
        }
    };
    let old_storage_key = match current_storage_encryption_key(&state).await {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current storage encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };

    match master_mgr.prepare_password(&req.new_password) {
        Ok(prepared) => {
            let new_key = prepared.key_manager.clone();
            if let Err(e) = rotate_application_encryption(
                &state,
                &config_dir,
                old_secrets_key,
                old_storage_key,
                new_key.clone(),
                || {
                    master_mgr
                        .commit_prepared_password(prepared)
                        .map_err(anyhow::Error::from)
                },
            )
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to change password safely: {}", e)
                    })),
                )
                    .into_response();
            }

            tracing::info!("Master password changed and applied in-memory");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Password changed and applied.",
                    "restart_scheduled": false
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to change password: {}", e)
            })),
        )
            .into_response(),
    }
}

async fn remove_master_password(
    State(state): State<AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);

    if !master_mgr.is_password_set() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "No master password is set"
            })),
        )
            .into_response();
    }

    // Verify password first
    let old_secrets_key = match master_mgr.unlock(&req.password) {
        Ok(key) => key,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Password is incorrect"
                })),
            )
                .into_response();
        }
    };
    let old_storage_key = match current_storage_encryption_key(&state).await {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current storage encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };

    match master_mgr.prepare_keyfile_encryption() {
        Ok(new_key) => {
            if let Err(e) = rotate_application_encryption(
                &state,
                &config_dir,
                old_secrets_key,
                old_storage_key,
                new_key.clone(),
                || {
                    master_mgr
                        .commit_password_removal()
                        .map_err(anyhow::Error::from)
                },
            )
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to remove password safely: {}", e)
                    })),
                )
                    .into_response();
            }

            tracing::info!("Master password removed, reverted to keyfile encryption in-memory");
            if tunnel_auth::control_plane_tunnel_is_active(&state).await {
                tunnel::stop_tunnel_internal(&state).await;
                tracing::info!(
                    "Stopped active control-plane tunnel after removing the custom master password"
                );
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Master password removed.",
                    "restart_scheduled": false
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to remove password: {}", e)
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_stream_tool_activity_content_hides_html_payloads() {
        let summary = summarize_stream_tool_activity_content(
            "<!DOCTYPE html><html><head><title>arXiv Research Monitor | RL & Time-Series</title></head><body><div>demo</div></body></html>",
        );

        assert_eq!(
            summary,
            "Read HTML document: arXiv Research Monitor | RL & Time-Series."
        );
        assert!(!summary.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn normalize_stream_heartbeat_status_collapses_model_and_memory_messages() {
        assert_eq!(
            normalize_stream_heartbeat_status("Waiting for z-ai/glm-5 to respond (15s)..."),
            "Waiting on model response. No new output yet."
        );
        assert_eq!(
            normalize_stream_heartbeat_status("Mem0 active | Scope: channel:web | Channel: web"),
            "Memory/context setup in progress. No new output yet."
        );
        assert_eq!(
            normalize_stream_heartbeat_status("Context Packing | Loaded 3 messages"),
            "Preparing conversation context. No new output yet."
        );
        assert_eq!(
            normalize_stream_heartbeat_status(
                "Memory available on demand | Scope: channel:web | Channel: web"
            ),
            "Still processing. No new output yet."
        );
    }

    #[test]
    fn normalize_stream_event_for_sse_suppresses_duplicate_heartbeat_updates() {
        let (first_event, first_state) = normalize_stream_event_for_sse(
            crate::core::StreamEvent::Thinking(
                "Waiting for z-ai/glm-5 to respond (5s)...".to_string(),
            ),
            "",
        );
        let Some((event_name, payload)) = first_event else {
            panic!("expected first heartbeat event");
        };
        assert_eq!(event_name, "thinking");
        assert_eq!(
            payload.get("detail").and_then(|v| v.as_str()),
            Some("Waiting on model response. No new output yet.")
        );

        let (second_event, second_state) = normalize_stream_event_for_sse(
            crate::core::StreamEvent::Thinking(
                "Model z-ai/glm-5 is generating (10s elapsed)...".to_string(),
            ),
            &first_state,
        );
        assert!(second_event.is_none());
        assert_eq!(second_state, first_state);
    }

    #[test]
    fn normalize_stream_event_for_sse_summarizes_tool_results() {
        let (event, next_state) = normalize_stream_event_for_sse(
            crate::core::StreamEvent::ToolResult {
                name: "file_read".to_string(),
                content:
                    "<!DOCTYPE html><html><head><title>Demo</title></head><body></body></html>"
                        .to_string(),
            },
            "Waiting on model response. No new output yet.",
        );
        assert!(next_state.is_empty());
        let Some((event_name, payload)) = event else {
            panic!("expected tool_result event");
        };
        assert_eq!(event_name, "tool_result");
        assert_eq!(
            payload.get("content").and_then(|v| v.as_str()),
            Some("Read HTML document: Demo.")
        );
    }

    #[test]
    fn chat_task_classifier_promotes_plain_english_app_requests() {
        let message = "Spin up an admin console for lead triage and deploy it";
        assert!(chat_message_requests_app_work(
            &message.to_ascii_lowercase()
        ));
        assert!(chat_request_should_create_task(
            Some("auto"),
            message,
            false,
            false
        ));
        assert_eq!(classify_chat_task_work_type(message, false, false), "app");
    }

    #[test]
    fn chat_task_classifier_promotes_direct_import_sources() {
        let message = "https://clawhub.ai/pskoett/self-improving-agent";
        assert!(chat_message_requests_import(&message.to_ascii_lowercase()));
        assert!(chat_request_should_create_task(
            Some("auto"),
            message,
            false,
            false
        ));
        assert_eq!(
            classify_chat_task_work_type(message, false, false),
            "import"
        );
    }

    #[test]
    fn chat_task_classifier_promotes_file_backed_runs_in_auto_mode() {
        assert!(chat_request_should_create_task(
            Some("auto"),
            "Please review these files and tell me what matters.",
            false,
            true
        ));
        assert_eq!(
            classify_chat_task_work_type(
                "Please review these files and tell me what matters.",
                false,
                true
            ),
            "workspace"
        );
    }
}
