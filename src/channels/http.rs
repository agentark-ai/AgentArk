//! Local HTTP API for IPC with authentication, CORS, and rate limiting

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        ConnectInfo, DefaultBodyLimit, Extension, FromRequestParts, MatchedPath, Multipart, Path,
        Query, Request, State,
    },
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{any, delete, get, post, put},
    Json, Router,
};
use chrono::{Datelike, Timelike};
use futures::{SinkExt, StreamExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, OnceLock,
};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as TungsteniteMessage};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub(crate) mod actions;
mod analytics_control;
mod api_docs_control;
mod api_types;
mod app_serving;
mod applications;
mod arkorbit_control;
mod arkpulse_control;
mod auth;
mod auth_profiles_control;
mod automation_control;
mod autonomy_control;
mod autonomy_support;
mod browser_profiles_control;
mod chat_control;
mod companion_control;
mod control_commands;
mod conversation_control;
mod custom_apis;
mod custom_messaging_channels;
mod document_control;
mod evolution_control;
mod extension_packs;
mod gateway_control;
mod gateway_ops_control;
mod hooks_control;
mod integration_sync;
mod integrations;
mod locked_control;
mod mcp_control;
mod memory_control;
mod middleware_control;
mod model_failover_control;
mod nodes_control;
mod notification_control;
mod observability;
mod plugins;
mod profile_control;
mod reflect_control;
mod runtime_control;
mod secrets_control;
mod security_control;
mod sender_verification;
mod sentinel_panel;
mod server_utils;
mod settings_control;
mod skill_marketplaces;
mod suggestions;
mod swarm_control;
mod trace;
mod tunnel;
mod tunnel_auth;
mod ui_control;
pub(crate) mod webhooks;

pub(crate) use analytics_control::estimate_cost_from_pricing_cache;
pub(crate) use autonomy_control::run_autonomy_analysis_tick;
pub use locked_control::serve_locked;
pub(crate) use memory_control::run_arkmemory_learned_review_pass;

pub(crate) use self::sentinel_panel::{
    load_background_learning_feed, record_background_learning_job_result,
    BackgroundLearningJobUpdate,
};
use analytics_control::*;
use api_docs_control::*;
use api_types::*;
use app_serving::*;
use arkpulse_control::*;
use automation_control::*;
use autonomy_control::*;
use autonomy_support::*;
use chat_control::*;
use control_commands::*;
use conversation_control::*;
use document_control::*;
use evolution_control::*;
use hooks_control::*;
use locked_control::{
    change_master_password, remove_master_password, security_status, set_master_password,
};
use mcp_control::*;
use memory_control::*;
use middleware_control::*;
use notification_control::*;
use profile_control::*;
use reflect_control::*;
use runtime_control::*;
use secrets_control::*;
use server_utils::*;
use settings_control::*;
use swarm_control::*;
use ui_control::*;

use crate::channels::{
    discord::DiscordChannelConfig, google_chat::GoogleChatChannelConfig,
    imessage::IMessageChannelConfig, line::LineChannelConfig, matrix::MatrixTransportConfig,
    qq::QqChannelConfig, signal::SignalChannelConfig, slack::SlackChannelConfig,
    teams::TeamsTransportConfig, wechat::WeChatChannelConfig,
};
use crate::clients::{
    ExecutorClient, ExecutorClientConfig, WorkspaceClient, WorkspaceClientConfig,
};
use crate::core::config::{
    DeploymentMode, EmbeddingsConfig, EmbeddingsProviderKind, TelegramConfig,
    TunnelCloudflareConfig, TunnelConfig, TunnelNgrokConfig, TunnelProviderKind,
    TunnelTailscaleConfig,
};
use crate::core::data_lifecycle::{
    load_data_lifecycle_settings, save_data_lifecycle_settings, DataLifecycleSettings,
};
use crate::core::llm_provider::{
    canonical_provider_id, display_openai_base_url, force_refresh_codex_cli_api_key,
    is_openrouter_base_url, normalize_openai_base_url, openai_provider_label,
    persist_codex_cli_oauth_tokens, provider_allows_model_discovery, resolve_codex_cli_api_key,
    resolve_openai_request_config, HUGGINGFACE_API_BASE_URL, OPENAI_DEVICE_AUTH_CLIENT_ID,
    OPENAI_DEVICE_REDIRECT_URI, OPENAI_DEVICE_TOKEN_URL, OPENAI_DEVICE_USERCODE_URL,
    OPENAI_DEVICE_VERIFY_URL, OPENAI_OAUTH_TOKEN_URL, OPENROUTER_API_BASE_URL,
};
use crate::core::{
    score_action_risk, Agent, AutonomySettings, AutopilotMode, ConversationScope, ExecutionTrace,
    LlmProvider, ModelRole, ModelSlot, RecommendedAction, RiskEnvelope, RiskLevel, Task,
    TaskApproval, TaskQueue, TaskStatus, TrustPolicy, UserProfile,
};
use crate::hooks;

type SharedAgent = Arc<RwLock<Agent>>;
const FRONTEND_DIST_DIR: &str = "frontend/dist";
const DEFAULT_RATE_LIMIT_MAX_TRACKED_IPS: usize = 4096;
const SERVER_BUSY_TRACE_WINDOW_SECS: i64 = 20 * 60;
const SERVER_BUSY_TASK_WINDOW_SECS: i64 = 20 * 60;
const MAX_CHAT_MESSAGE_BYTES: usize = 100_000;
const MAX_HTTP_BODY_BYTES: usize = 2 * 1024 * 1024;
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CONFIG_MUTATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const UI_SESSION_TTL_SECS: i64 = 24 * 60 * 60;
const UI_SESSION_MAX_TRACKED: usize = 4096;
const RELEASE_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RELEASE_UPDATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const AUTONOMY_ANALYSIS_TICK_TIMEOUT: Duration = Duration::from_secs(45);
const AUTONOMY_REACTIVE_TICK_TIMEOUT: Duration = Duration::from_secs(10);
const SSE_MAX_CONNECTION_LIFETIME: Duration = Duration::from_secs(30 * 60);
const HEADER_X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
const HEADER_X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const HEADER_CONTENT_SECURITY_POLICY: HeaderName =
    HeaderName::from_static("content-security-policy");
const HEADER_STRICT_TRANSPORT_SECURITY: HeaderName =
    HeaderName::from_static("strict-transport-security");
const CACHE_CONTROL_FRONTEND_HTML: &str = "no-store";
const CACHE_CONTROL_FRONTEND_ASSET: &str = "public, max-age=31536000, immutable";
static MISSING_API_KEY_WARNED: AtomicBool = AtomicBool::new(false);
static CHAT_SUGGESTION_SCAN_ACTIVE: AtomicBool = AtomicBool::new(false);
static AUTONOMY_ANALYSIS_TICK_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
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

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

async fn acquire_agent_write_for_config_mutation(
    state: &AppState,
    operation: &'static str,
) -> std::result::Result<tokio::sync::OwnedRwLockWriteGuard<Agent>, Response> {
    match tokio::time::timeout(
        CONFIG_MUTATION_LOCK_TIMEOUT,
        state.agent.clone().write_owned(),
    )
    .await
    {
        Ok(guard) => Ok(guard),
        Err(_) => Err(error_response(
            StatusCode::CONFLICT,
            format!(
                "Agent is busy with another active run. Retry {} after the current run finishes.",
                operation
            ),
        )),
    }
}

async fn save_agent_config_snapshot(
    config: crate::core::AgentConfig,
    config_dir: PathBuf,
    data_dir: PathBuf,
) -> Result<()> {
    tokio::task::spawn_blocking(move || config.save(&config_dir, Some(data_dir.as_path())))
        .await
        .map_err(|error| anyhow::anyhow!("Failed to join config save worker: {}", error))?
}

async fn load_saved_model_slot_api_key(
    config_dir: PathBuf,
    data_dir: PathBuf,
    previous_slot_id: String,
    requested_id: String,
    role: String,
    label: String,
) -> Option<String> {
    match tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let secure = crate::core::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(data_dir.as_path()),
        )?;
        let secrets = secure.load_secrets()?;
        Ok(secrets
            .model_pool_keys
            .get(previous_slot_id.trim())
            .cloned()
            .or_else(|| secrets.model_pool_keys.get(requested_id.trim()).cloned())
            .or_else(|| secrets.model_pool_keys.get(role.trim()).cloned())
            .or_else(|| secrets.model_pool_keys.get(label.trim()).cloned()))
    })
    .await
    {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => {
            tracing::warn!("Failed to recover saved model slot key: {}", error);
            None
        }
        Err(error) => {
            tracing::warn!("Model slot key recovery worker failed: {}", error);
            None
        }
    }
}

async fn resolve_requested_model_slot_api_key(
    state: &AppState,
    requested_id: &str,
    request: &ModelSlotRequest,
) -> std::result::Result<Option<String>, Response> {
    let (
        previous_slot_id_for_lookup,
        config_dir_for_lookup,
        data_dir_for_lookup,
        can_reuse_existing_key,
        existing_key_hint,
    ) = {
        let agent = state.agent.read().await;

        let slot_idx = resolve_model_slot_index(&agent.config.model_pool.slots, requested_id);
        let Some(idx) = slot_idx else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Model slot not found".to_string(),
                }),
            )
                .into_response());
        };

        let current_slot = agent.config.model_pool.slots[idx].clone();
        let can_reuse_existing_key = match can_reuse_model_slot_api_key(&current_slot, request) {
            Ok(value) => value,
            Err(error) => {
                return Err(
                    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response()
                );
            }
        };
        let existing_key_hint = match &current_slot.provider {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            _ => None,
        };

        (
            current_slot.id.trim().to_string(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            can_reuse_existing_key,
            existing_key_hint,
        )
    };

    if !can_reuse_existing_key {
        return Ok(None);
    }

    let existing_key = if matches!(
        existing_key_hint.as_deref(),
        None | Some("") | Some("[ENCRYPTED]")
    ) {
        load_saved_model_slot_api_key(
            config_dir_for_lookup,
            data_dir_for_lookup,
            previous_slot_id_for_lookup,
            requested_id.to_string(),
            request.role.clone(),
            request.label.clone(),
        )
        .await
        .or(existing_key_hint)
    } else {
        existing_key_hint
    };

    Ok(existing_key)
}

async fn remove_saved_model_slot_api_keys(
    config_dir: PathBuf,
    data_dir: PathBuf,
    resolved_slot_id: String,
    requested_id: String,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(data_dir.as_path()),
        )?;
        manager.update_secrets(|secrets| {
            if !resolved_slot_id.is_empty() {
                secrets.model_pool_keys.remove(&resolved_slot_id);
            }
            if !requested_id.is_empty() && requested_id != resolved_slot_id {
                secrets.model_pool_keys.remove(&requested_id);
            }
            Ok(())
        })?;
        Ok(())
    })
    .await
    .map_err(|error| anyhow::anyhow!("Failed to join model-key cleanup worker: {}", error))?
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
        #[serde(default)]
        env: Option<std::collections::HashMap<String, String>>,
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
    auth_profile_id: Option<String>,
    #[serde(default)]
    tool_allowlist: Vec<String>,
    #[serde(default)]
    tool_blocklist: Vec<String>,
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
        } else if path.starts_with("/skills") || path.starts_with("/custom-messaging-channels") {
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
    /// Cancellation signals for actively streamed chat tasks.
    pub chat_task_cancellations: Arc<RwLock<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    /// Cancellation signals for active Skills test runs.
    pub action_test_cancellations: Arc<RwLock<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    /// Foreground chat-stream cancellation signals keyed by conversation id.
    chat_conversation_cancellations: Arc<RwLock<HashMap<String, ActiveChatConversationStream>>>,
    /// User profile - can be read without locking agent
    pub user_profile: Arc<RwLock<UserProfile>>,
    /// Tiered rate limiter for all endpoints
    pub tiered_rate_limiter: TieredRateLimiter,
    /// HTTP API key for authentication (None = blocked unless insecure override is enabled)
    pub api_key: Arc<RwLock<Option<String>>>,
    /// HTTP API key expiry (unix timestamp seconds)
    pub api_key_expires_at: Arc<RwLock<Option<i64>>>,
    /// Legacy flag retained for state compatibility; protected routes always require auth.
    pub allow_insecure_no_auth: bool,
    /// Per-browser UI sessions tracked server-side.
    pub ui_sessions: Arc<RwLock<HashMap<String, UiSessionRecord>>>,
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
    /// Short-lived in-process locks for app publish operations.
    pub app_publish_locks: Arc<parking_lot::Mutex<HashSet<String>>>,
    /// Optional internal executor client used when runtime ownership is split out.
    executor_client: Option<Arc<ExecutorClient>>,
    /// Optional internal workspace client used when file authority is split out.
    workspace_client: Option<Arc<WorkspaceClient>>,
    /// Registry of terminal-first external application launchers.
    application_registry: applications::ApplicationLauncherRegistry,
    /// Deployment posture of the control plane.
    pub deployment_mode: DeploymentMode,
    /// Which surface this listener serves.
    pub server_role: HttpServerRole,
    /// Runtime process start time for lightweight status telemetry.
    pub runtime_started_at: Instant,
    /// Optional dedicated bind address for public apps.
    pub public_app_bind_addr: Option<String>,
    /// Optional externally visible base URL for public apps.
    pub public_app_base_url: Option<String>,
    /// Cached release update metadata for lightweight UI polling.
    release_update_cache: Arc<RwLock<ReleaseUpdateCache>>,
}

#[derive(Clone)]
struct ActiveChatConversationStream {
    request_id: String,
    sender: tokio::sync::watch::Sender<bool>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct UiSessionRecord {
    pub issued_at: i64,
    pub expires_at: i64,
    pub last_seen_at: i64,
    pub source: String,
    pub client_hint: Option<String>,
}

fn stack_role() -> Option<String> {
    std::env::var("AGENTARK_STACK_ROLE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
}

fn build_executor_client() -> Result<Option<Arc<ExecutorClient>>> {
    if !matches!(stack_role().as_deref(), Some("control-plane" | "control")) {
        return Ok(None);
    }
    let client = ExecutorClient::new(ExecutorClientConfig::from_env())?;
    if client.bearer_token().is_none() {
        return Ok(None);
    }
    Ok(Some(Arc::new(client)))
}

fn build_workspace_client() -> Result<Option<Arc<WorkspaceClient>>> {
    if !matches!(stack_role().as_deref(), Some("control-plane" | "control")) {
        return Ok(None);
    }
    let client = WorkspaceClient::new(WorkspaceClientConfig::from_env())?;
    if client.bearer_token().is_none() {
        return Ok(None);
    }
    Ok(Some(Arc::new(client)))
}

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

fn cap_sse_lifetime<S>(
    stream: S,
) -> impl futures::Stream<Item = std::result::Result<Event, std::convert::Infallible>> + Send + 'static
where
    S: futures::Stream<Item = std::result::Result<Event, std::convert::Infallible>>
        + Send
        + 'static,
{
    stream.take_until(tokio::time::sleep(SSE_MAX_CONNECTION_LIFETIME))
}

#[derive(Clone, Debug)]
enum PendingOAuthTarget {
    Integration { service_id: String },
    AuthProfile { profile_id: String },
}

#[derive(Clone, Debug)]
struct PendingOAuthState {
    target: PendingOAuthTarget,
    expires_at: i64,
    pkce_verifier: Option<String>,
    redirect_uri: Option<String>,
}

/// Chat request
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub deep_research: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_profile: Option<serde_json::Value>,
    #[serde(default)]
    pub plan_confirmation_mode: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub attachments_present: bool,
    #[serde(default)]
    pub attachments: Vec<crate::core::ChatAttachmentHint>,
    /// ArkOrbit-only: structural context describing the orbit the user is on
    /// when sending this message (active orbit id + widget summary). Slice 2
    /// uses this to inject a structural augmentation into the inbound
    /// classifier prompt so the model can reason about whether the request
    /// involves the page the user has open. Never freeform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arkorbit_context: Option<serde_json::Value>,
    /// Optional structured browser profile selection supplied by the UI.
    /// This keeps profile ids and browser metadata out of the visible chat
    /// message while still giving the agent the selected profile context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_profile_context: Option<serde_json::Value>,
    #[serde(default)]
    pub accepted_suggestion_id: Option<String>,
    #[serde(default)]
    pub sentinel_proposal_id: Option<String>,
}

fn default_channel() -> String {
    "http".to_string()
}

async fn bind_chat_task_cancellation_sender(
    state: &AppState,
    task_id: &str,
    sender: tokio::sync::watch::Sender<bool>,
) {
    state
        .chat_task_cancellations
        .write()
        .await
        .insert(task_id.to_string(), sender);
}

async fn signal_chat_task_cancellation(state: &AppState, task_id: &str) {
    let sender = {
        state
            .chat_task_cancellations
            .read()
            .await
            .get(task_id)
            .cloned()
    };
    if let Some(sender) = sender {
        let _ = sender.send(true);
    }
}

async fn unregister_chat_task_cancellation(state: &AppState, task_id: &str) {
    state.chat_task_cancellations.write().await.remove(task_id);
}

async fn replace_chat_conversation_cancellation_sender(
    state: &AppState,
    conversation_id: &str,
    request_id: &str,
    sender: tokio::sync::watch::Sender<bool>,
) -> Option<tokio::sync::watch::Sender<bool>> {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return None;
    }
    state
        .chat_conversation_cancellations
        .write()
        .await
        .insert(
            conversation_id.to_string(),
            ActiveChatConversationStream {
                request_id: request_id.to_string(),
                sender,
            },
        )
        .map(|entry| entry.sender)
}

async fn unregister_chat_conversation_cancellation(
    state: &AppState,
    conversation_id: &str,
    request_id: &str,
) {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return;
    }
    let mut guard = state.chat_conversation_cancellations.write().await;
    let should_remove = guard
        .get(conversation_id)
        .map(|entry| entry.request_id == request_id)
        .unwrap_or(false);
    if should_remove {
        guard.remove(conversation_id);
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
    let client = shared_http_client().clone();
    let resp = client
        .post(OPENAI_DEVICE_USERCODE_URL)
        .json(&serde_json::json!({ "client_id": OPENAI_DEVICE_AUTH_CLIENT_ID }))
        .send()
        .await
        .map_err(|e| format!("Failed to request device code: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("OpenAI device code request failed ({})", status));
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
        return Err("OpenAI returned incomplete device code response".to_string());
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
    crate::spawn_logged!("src/channels/http.rs:764", async move {
        let client = shared_http_client().clone();
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

                                        if let Err(error) = persist_codex_cli_oauth_tokens(
                                            access, refresh, expires_ms,
                                        ) {
                                            let mut state = runtime_bg.write().await;
                                            state.active = false;
                                            state.last_error = Some(format!(
                                                "Failed to save OpenAI OAuth tokens: {}",
                                                error
                                            ));
                                            return;
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
                                    let status = tr.status();
                                    let mut state = runtime_bg.write().await;
                                    state.active = false;
                                    state.last_error =
                                        Some(format!("Token exchange failed ({})", status));
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
                    let mut state = runtime_bg.write().await;
                    state.active = false;
                    state.last_error = Some(format!("Poll failed ({})", status));
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

fn server_can_launch_local_browser() -> bool {
    if cfg!(target_os = "linux") {
        let display_available =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        if !display_available || std::path::Path::new("/.dockerenv").exists() {
            return false;
        }
    }
    true
}

fn normalize_model_credential_base_url(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
}

fn model_slot_credential_scope(slot: &ModelSlot) -> (String, Option<String>) {
    match &slot.provider {
        LlmProvider::Anthropic { .. } => ("anthropic".to_string(), None),
        LlmProvider::Ollama { base_url, .. } => (
            "ollama".to_string(),
            normalize_model_credential_base_url(Some(base_url.as_str())),
        ),
        LlmProvider::OpenAI { base_url, .. } => (
            openai_provider_label(base_url.as_deref()).to_string(),
            normalize_model_credential_base_url(
                display_openai_base_url(base_url.as_ref()).as_deref(),
            ),
        ),
    }
}

fn model_slot_request_credential_scope(
    request: &ModelSlotRequest,
) -> std::result::Result<(String, Option<String>), String> {
    if request.provider.trim().is_empty() {
        return Err("Provider is required".to_string());
    }
    let Some(provider_id) = canonical_provider_id(request.provider.as_str()) else {
        return Err(format!("Unknown provider: {}", request.provider));
    };
    let base_url = request.base_url.as_deref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    match provider_id {
        "anthropic" => Ok(("anthropic".to_string(), None)),
        "ollama" => {
            if base_url.is_none() {
                return Err("Ollama base URL is required".to_string());
            }
            Ok((
                "ollama".to_string(),
                normalize_model_credential_base_url(base_url.as_deref()),
            ))
        }
        "openai" | "openai-compatible" | "openrouter" | "openai-subscription" | "huggingface" => {
            let compat_base_url = normalize_openai_base_url(provider_id, base_url)?;
            Ok((
                openai_provider_label(compat_base_url.as_deref()).to_string(),
                normalize_model_credential_base_url(
                    display_openai_base_url(compat_base_url.as_ref()).as_deref(),
                ),
            ))
        }
        _ => Ok((
            provider_id.to_string(),
            normalize_model_credential_base_url(base_url.as_deref()),
        )),
    }
}

fn can_reuse_model_slot_api_key(
    slot: &ModelSlot,
    request: &ModelSlotRequest,
) -> std::result::Result<bool, String> {
    if request.clear_api_key.unwrap_or(false) {
        return Ok(false);
    }
    Ok(model_slot_credential_scope(slot) == model_slot_request_credential_scope(request)?)
}

fn llm_provider_requires_api_key(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "anthropic" | "openai" | "openrouter" | "openai-subscription" | "huggingface"
    )
}

async fn provider_from_model_slot_request(
    request: &ModelSlotRequest,
    existing_api_key: Option<String>,
) -> std::result::Result<LlmProvider, String> {
    if request.provider.trim().is_empty() {
        return Err("Provider is required".to_string());
    }
    let Some(provider_id) = canonical_provider_id(request.provider.as_str()) else {
        return Err(format!("Unknown provider: {}", request.provider));
    };
    let base_url = request.base_url.clone().and_then(|u| {
        let trimmed = u.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let compat_base_url = normalize_openai_base_url(provider_id, base_url.clone())?;
    let mut api_key = request
        .api_key
        .clone()
        .filter(|k| !k.is_empty() && k != "[ENCRYPTED]")
        .or(existing_api_key.filter(|k| !k.is_empty() && k != "[ENCRYPTED]"))
        .unwrap_or_default();
    if provider_id == "openai-subscription" && api_key.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        api_key = resolve_codex_cli_api_key(&client, false)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
    }
    if provider_id == "ollama" && base_url.is_none() {
        return Err("Ollama base URL is required".to_string());
    }
    if provider_id == "openai-compatible" && compat_base_url.is_none() {
        return Err("Base URL is required for OpenAI-Compatible providers".to_string());
    }
    if llm_provider_requires_api_key(provider_id) && api_key.trim().is_empty() {
        return Err("API key is required for the selected provider".to_string());
    }

    let provider = match provider_id {
        "ollama" => LlmProvider::Ollama {
            base_url: base_url.unwrap_or_default(),
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
        "openai-compatible" | "openrouter" | "huggingface" => LlmProvider::OpenAI {
            api_key: api_key.clone(),
            model: request.model.clone(),
            base_url: compat_base_url,
        },
        "openai-subscription" => {
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
            return Err(format!("Unknown provider: {}", provider_id));
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
    crate::core::secrets::parse_set_secret_command(message)
}

/// Start the HTTP server with authentication, CORS, and rate limiting
pub async fn serve(
    agent: SharedAgent,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    {
        let agent_for_reconcile = agent.clone();
        crate::spawn_logged!("src/channels/http.rs:3767", async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(OPTIONAL_BACKGROUND_JOB_TIMEOUT_SECS),
                async {
                    let agent_guard = agent_for_reconcile.read().await;
                    agent_guard.runtime.reconcile_orphan_containers().await
                },
            )
            .await;
            match result {
                Ok(Ok(_)) => tracing::debug!("Initial sandbox container reconciliation completed"),
                Ok(Err(error)) => {
                    tracing::warn!("Initial sandbox container reconciliation failed: {}", error)
                }
                Err(_) => tracing::warn!(
                    "Initial sandbox container reconciliation timed out after {}s",
                    OPTIONAL_BACKGROUND_JOB_TIMEOUT_SECS
                ),
            }
        });
    }

    let tiered_rate_limiter = TieredRateLimiter::new();
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or("127.0.0.1:8990".to_string());

    // Spawn a background task to periodically clean up expired rate-limit entries
    {
        let trl = tiered_rate_limiter.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/channels/http.rs:3796", async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
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
        let configured_public_app_base_url = public_app_base_url_from_config(&agent_guard.config);
        let isolate_public_apps = internet_facing_apps_should_be_isolated(
            deployment_mode,
            public_app_bind_addr.as_deref(),
        );
        let public_app_base_url = configured_public_app_base_url.clone().or_else(|| {
            if isolate_public_apps {
                public_app_bind_addr
                    .as_deref()
                    .and_then(default_base_url_for_bind_addr)
            } else {
                None
            }
        });
        let insecure_no_auth_requested =
            parse_env_truthy("AGENTARK_INSECURE_NO_AUTH").unwrap_or(false);
        if insecure_no_auth_requested {
            tracing::warn!(
                "Ignoring AGENTARK_INSECURE_NO_AUTH: protected routes always require API auth"
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
        validate_control_plane_listener_posture(
            deployment_mode,
            &bind_addr,
            cookie_secure_default,
        )?;
        validate_public_app_listener_posture(
            deployment_mode,
            public_app_bind_addr.as_deref(),
            configured_public_app_base_url.as_deref(),
            cookie_secure_default,
        )?;
        let local_ui_bootstrap_enabled = deployment_mode == DeploymentMode::TrustedLocal;
        let executor_client = build_executor_client()?;
        let workspace_client = build_workspace_client()?;
        AppState {
            agent: agent.clone(),
            trace_history: agent_guard.trace_history.clone(),
            last_trace: agent_guard.last_trace.clone(),
            tasks: agent_guard.tasks.clone(),
            chat_task_cancellations: Arc::new(RwLock::new(HashMap::new())),
            action_test_cancellations: Arc::new(RwLock::new(HashMap::new())),
            chat_conversation_cancellations: Arc::new(RwLock::new(HashMap::new())),
            user_profile: agent_guard.user_profile.clone(),
            tiered_rate_limiter,
            api_key: Arc::new(RwLock::new(initial_api_key)),
            api_key_expires_at: Arc::new(RwLock::new(initial_api_key_expires_at)),
            allow_insecure_no_auth: false,
            ui_sessions: Arc::new(RwLock::new(HashMap::new())),
            local_ui_bootstrap_enabled,
            local_ui_bootstrap_tokens: Arc::new(RwLock::new(HashMap::new())),
            cookie_secure_default,
            oauth_states: Arc::new(RwLock::new(HashMap::new())),
            remote_login_attempts: Arc::new(RwLock::new(HashMap::new())),
            tunnel: Arc::new(RwLock::new(tunnel::TunnelState::new())),
            whatsapp_bridge: Arc::new(RwLock::new(WhatsAppBridgeState::new())),
            security_events: agent_guard.security_events.clone(),
            app_registry: agent_guard.app_registry.clone(),
            app_publish_locks: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            executor_client,
            workspace_client,
            application_registry: applications::ApplicationLauncherRegistry::default(),
            deployment_mode,
            server_role: HttpServerRole::ControlPlane,
            runtime_started_at: Instant::now(),
            public_app_bind_addr,
            public_app_base_url,
            release_update_cache: Arc::new(RwLock::new(ReleaseUpdateCache::default())),
        }
    };

    spawn_reflect_idle_loop(state.clone());
    spawn_gepa_auto_loop(state.clone());

    // Public routes (no auth required)
    let public_routes = Router::new()
        .route("/", get(web_ui))
        .route("/ui", get(web_ui))
        .route("/ui/v2", get(web_ui_v2))
        .route("/session/bootstrap", post(auth::bootstrap_ui_session))
        .route("/session/logout", post(auth::logout_ui_session))
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
        .route("/favicon.png", get(serve_favicon_png))
        .route("/public/proxy/raw", get(public_proxy_raw))
        .route("/health", get(health))
        .route("/readiness", get(readiness))
        // WhatsApp webhook (public - Meta calls without auth)
        .route("/webhook/whatsapp", get(whatsapp_webhook_verify))
        .route("/webhook/whatsapp", post(whatsapp_webhook_handler))
        .route(
            "/extension-packs/{id}/webhook",
            get(extension_packs::verify_extension_pack_webhook)
                .post(extension_packs::handle_extension_pack_webhook),
        )
        .route("/webhook/google-chat", post(google_chat_webhook_handler))
        .route("/webhook/signal", post(signal_webhook_handler))
        .route("/webhook/imessage", post(imessage_webhook_handler))
        .route("/webhook/line", post(line_webhook_handler))
        .route("/webhook/wechat", post(wechat_webhook_handler))
        .route("/webhook/qq", post(qq_webhook_handler))
        .route(
            "/webhook/inbound/{source_id}",
            post(webhooks::handle_inbound_webhook),
        )
        // Slack webhook (public endpoint, authenticated by Slack request signatures)
        .route("/webhook/slack", post(slack_webhook_handler))
        // Teams webhook (public endpoint, authenticated by Bot Framework JWTs)
        .route("/webhook/teams", post(teams_webhook_handler))
        // OAuth callback (public - browser redirect from Google/Meta with no auth headers)
        .route("/oauth/callback", get(integrations::oauth_callback))
        .route("/companion/web", get(companion_control::companion_web))
        .route("/companion/ws", get(companion_control::companion_ws))
        // Deployed apps (public - these are user-facing apps, no auth required)
        .route("/apps/{app_id}", any(serve_app_root))
        .route("/apps/{app_id}/", any(serve_app_root))
        .route("/apps/{app_id}/{*path}", any(serve_app_path));

    // Protected routes (require Bearer token + rate limited)
    let protected_routes = Router::new()
        .route("/status", get(status))
        .route("/metrics", get(metrics))
        .route("/chat", post(chat))
        .route("/chat/stream", post(chat_stream))
        .route(
            "/chat/tool-approvals/{id}/decision",
            post(decide_chat_tool_approval),
        )
        .route(
            "/chat/credential-prompt",
            get(get_chat_credential_prompt).delete(dismiss_chat_credential_prompt),
        )
        .route(
            "/chat/credential-prompt/submit",
            post(submit_chat_credential_prompt),
        )
        .route(
            "/chat/credential/raw-secret/submit",
            post(submit_chat_raw_secret),
        )
        .route(
            "/chat/credential/raw-secret/reuse-model-credential",
            post(reuse_model_credential_for_chat),
        )
        .route("/chat/clear", post(clear_chat))
        .route("/gateway/channels", get(gateway_control::get_channels))
        .route("/gateway/ops", get(gateway_ops_control::get_overview))
        .route(
            "/gateway/channels/accounts",
            post(gateway_control::create_channel_account),
        )
        .route(
            "/gateway/channels/accounts/{id}",
            post(gateway_control::update_channel_account)
                .delete(gateway_control::delete_channel_account),
        )
        .route("/gateway/routing", get(gateway_control::get_routing))
        .route(
            "/gateway/routing/rules",
            post(gateway_control::create_route_rule),
        )
        .route(
            "/gateway/routing/rules/{id}",
            post(gateway_control::update_route_rule).delete(gateway_control::delete_route_rule),
        )
        .route(
            "/gateway/routing/groups",
            post(gateway_control::create_broadcast_group),
        )
        .route(
            "/gateway/routing/simulate",
            post(gateway_control::simulate_routing),
        )
        .route("/companion/presets", get(companion_control::get_presets))
        .route("/companion/protocol", get(companion_control::get_protocol))
        .route(
            "/companion/connectivity",
            get(companion_control::get_connectivity),
        )
        .route(
            "/companion/mobile-access",
            get(companion_control::get_mobile_access),
        )
        .route(
            "/companion/connectivity/tunnel/start",
            post(companion_control::start_companion_tunnel),
        )
        .route(
            "/companion/connectivity/tunnel/stop",
            post(companion_control::stop_companion_tunnel),
        )
        .route("/companion/devices", get(companion_control::list_devices))
        .route(
            "/companion/pairing-sessions",
            post(companion_control::create_pairing_session),
        )
        .route(
            "/companion/pairing-sessions/{id}/approve",
            post(companion_control::approve_pairing_session),
        )
        .route(
            "/companion/devices/{id}/commands",
            get(companion_control::list_commands).post(companion_control::create_command),
        )
        .route(
            "/companion/commands/{id}/approve",
            post(companion_control::approve_command),
        )
        .route(
            "/companion/devices/{id}/revoke",
            post(companion_control::revoke_device),
        )
        .route(
            "/companion/devices/{id}/tokens/rotate",
            post(companion_control::rotate_token),
        )
        .route("/companion/audit", get(companion_control::get_audit))
        .route("/nodes", get(nodes_control::list_nodes))
        .route("/nodes", post(nodes_control::create_node))
        .route(
            "/nodes/{id}",
            post(nodes_control::update_node).delete(nodes_control::revoke_node),
        )
        .route("/nodes/{id}/heartbeat", post(nodes_control::heartbeat_node))
        .route(
            "/nodes/{id}/commands",
            get(nodes_control::list_node_commands).post(nodes_control::log_node_command),
        )
        .route(
            "/browser/profiles",
            get(browser_profiles_control::list_profiles),
        )
        .route(
            "/browser/profiles",
            post(browser_profiles_control::create_profile),
        )
        .route(
            "/browser/profiles/{id}",
            post(browser_profiles_control::update_profile)
                .delete(browser_profiles_control::delete_profile),
        )
        .route(
            "/browser/profiles/{id}/launch",
            post(browser_profiles_control::launch_profile_browser),
        )
        .route(
            "/browser/profiles/{id}/close",
            post(browser_profiles_control::close_profile_browser),
        )
        .route(
            "/browser/profiles/{id}/lock",
            post(browser_profiles_control::lock_profile),
        )
        .route(
            "/browser/profiles/{id}/unlock",
            post(browser_profiles_control::unlock_profile),
        )
        .route(
            "/browser/profiles/{id}/sessions",
            post(browser_profiles_control::record_session),
        )
        .route(
            "/security/settings",
            get(security_control::get_security_settings)
                .put(security_control::update_security_settings),
        )
        .route(
            "/security/abuse-reviews",
            get(security_control::list_abuse_reviews),
        )
        .route(
            "/security/abuse-reviews/{source_key_hash}/approve",
            post(security_control::approve_abuse_review),
        )
        .route(
            "/security/abuse-reviews/{source_key_hash}/reject",
            post(security_control::reject_abuse_review),
        )
        .route("/auth/profiles", get(auth_profiles_control::list_profiles))
        .route(
            "/auth/profiles",
            post(auth_profiles_control::create_profile),
        )
        .route(
            "/auth/profiles/{id}",
            get(auth_profiles_control::get_profile)
                .post(auth_profiles_control::update_profile)
                .delete(auth_profiles_control::delete_profile),
        )
        .route(
            "/auth/profiles/{id}/revoke",
            post(auth_profiles_control::revoke_profile),
        )
        .route(
            "/auth/profiles/{id}/oauth/start",
            post(auth_profiles_control::start_oauth_profile),
        )
        .route(
            "/auth/profiles/{id}/session/capture",
            post(auth_profiles_control::capture_session_material),
        )
        .route(
            "/models/failover",
            get(model_failover_control::list_failover),
        )
        .route(
            "/models/failover/profiles",
            post(model_failover_control::upsert_profile),
        )
        .route(
            "/models/failover/profiles/{id}/default",
            post(model_failover_control::set_default_profile),
        )
        .route(
            "/models/failover/profiles/{id}/disable",
            post(model_failover_control::disable_profile),
        )
        .route(
            "/models/failover/profiles/{id}/clear-cooldown",
            post(model_failover_control::clear_profile_cooldown),
        )
        .route(
            "/models/failover/profiles/{id}/rotate",
            post(model_failover_control::rotate_profile),
        )
        .route(
            "/models/failover/providers",
            post(model_failover_control::upsert_provider),
        )
        .route(
            "/models/failover/providers/{id}/disable",
            post(model_failover_control::disable_provider),
        )
        .route(
            "/models/failover/providers/{id}/clear-cooldown",
            post(model_failover_control::clear_provider_cooldown),
        )
        .route(
            "/models/failover/chains",
            post(model_failover_control::upsert_chain),
        )
        .route(
            "/models/failover/select",
            post(model_failover_control::select_candidate),
        )
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
        .route(
            "/skills/test-runs/{run_id}/cancel",
            post(actions::cancel_action_test),
        )
        .route("/skills/import", post(actions::import_action))
        .route(
            "/skills/marketplaces",
            get(skill_marketplaces::list_skill_marketplaces)
                .post(skill_marketplaces::create_skill_marketplace),
        )
        .route(
            "/skills/marketplaces/{id}",
            axum::routing::put(skill_marketplaces::update_skill_marketplace)
                .delete(skill_marketplaces::delete_skill_marketplace),
        )
        .route(
            "/skills/marketplaces/{id}/refresh",
            post(skill_marketplaces::refresh_skill_marketplace),
        )
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
        .route(
            "/tasks/{id}/resume-chat/stream",
            post(resume_chat_task_stream),
        )
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route("/tasks/{id}/retry", post(retry_task))
        .route("/tasks/{id}/approve", post(approve_task))
        .route("/tasks/{id}/reject", post(reject_task))
        .route("/background-sessions", get(list_background_sessions))
        .route("/background-sessions", post(create_background_session))
        .route("/background-sessions/{id}", get(get_background_session))
        .route("/background-sessions/{id}", post(update_background_session))
        .route(
            "/background-sessions/{id}",
            axum::routing::delete(delete_background_session),
        )
        .route(
            "/background-sessions/{id}/attach",
            post(attach_background_session_work),
        )
        .route(
            "/background-sessions/{id}/detach",
            post(detach_background_session_work),
        )
        .route(
            "/background-sessions/{id}/pause",
            post(pause_background_session),
        )
        .route(
            "/background-sessions/{id}/resume",
            post(resume_background_session),
        )
        .route(
            "/background-sessions/{id}/cancel",
            post(cancel_background_session),
        )
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
        .route(
            "/autonomy/sentinel/settings",
            get(sentinel_panel::get_sentinel_settings)
                .post(sentinel_panel::update_sentinel_settings),
        )
        .route(
            "/autonomy/sentinel/feed",
            get(sentinel_panel::get_sentinel_feed),
        )
        .route(
            "/autonomy/sentinel/proposals/{id}/approve",
            post(sentinel_panel::approve_sentinel_proposal),
        )
        .route(
            "/autonomy/sentinel/proposals/{id}/dismiss",
            post(sentinel_panel::dismiss_sentinel_proposal),
        )
        .route(
            "/autonomy/sentinel/proposals/{id}/snooze",
            post(sentinel_panel::snooze_sentinel_proposal),
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
        .route("/autonomy/trust/evaluate", post(evaluate_trust_request))
        .route("/autonomy/voice/briefing", get(get_voice_briefing))
        .route("/autonomy/voice/command", post(handle_voice_command))
        .route("/gmail/oauth/start", post(integrations::gmail_oauth_start))
        .route("/gmail/status", get(integrations::gmail_status))
        .route("/gmail/test", get(integrations::gmail_test))
        .route("/settings", get(get_settings))
        .route("/settings", post(update_settings))
        .route(
            "/settings/google-workspace/oauth-client",
            get(get_google_workspace_oauth_client_settings),
        )
        .route(
            "/settings/google-workspace/oauth-client",
            post(update_google_workspace_oauth_client_settings),
        )
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
        .route("/models/test", post(test_model_connection))
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
        .route("/models/discover/{provider}", get(discover_provider_models))
        .route("/profile", get(get_profile))
        .route("/profile/onboarding", post(update_profile_onboarding))
        .route(
            "/profile/onboarding/dismiss",
            post(update_profile_onboarding_dismiss),
        )
        .route("/restart", post(restart_server))
        .route("/update", post(update_server))
        .route("/trace", get(trace::get_trace))
        .route("/trace/{id}", get(trace::get_trace_detail))
        // Integrations routes
        .route("/integrations", get(integrations::list_integrations))
        .route(
            "/extension-packs",
            get(extension_packs::list_extension_packs),
        )
        .route(
            "/extension-packs/install",
            post(extension_packs::install_extension_pack),
        )
        .route(
            "/extension-packs/upload",
            post(extension_packs::upload_extension_pack),
        )
        .route(
            "/extension-packs/scaffold",
            post(extension_packs::scaffold_extension_pack),
        )
        .route(
            "/extension-packs/invoke",
            post(extension_packs::invoke_extension_pack_feature),
        )
        .route(
            "/extension-packs/{id}",
            get(extension_packs::get_extension_pack).delete(extension_packs::delete_extension_pack),
        )
        .route(
            "/extension-packs/{id}/events",
            get(extension_packs::list_extension_pack_events),
        )
        .route(
            "/extension-packs/{id}/connections",
            get(extension_packs::list_extension_pack_connections)
                .post(extension_packs::upsert_extension_pack_connection),
        )
        .route(
            "/extension-packs/{id}/connect-url",
            get(extension_packs::get_extension_pack_connect_url),
        )
        .route(
            "/extension-packs/{id}/runtime/install",
            post(extension_packs::install_extension_pack_runtime),
        )
        .route(
            "/extension-packs/{id}/runtime/verify",
            post(extension_packs::verify_extension_pack_runtime),
        )
        .route(
            "/extension-packs/{id}/runtime/update",
            post(extension_packs::update_extension_pack_runtime),
        )
        .route(
            "/extension-packs/{id}/runtime/uninstall",
            post(extension_packs::uninstall_extension_pack_runtime),
        )
        .route(
            "/extension-packs/{id}/enabled",
            post(extension_packs::set_extension_pack_enabled),
        )
        .route(
            "/extension-packs/{id}/connections/{connection_id}/test",
            post(extension_packs::test_extension_pack_connection),
        )
        .route(
            "/integrations/sync/status",
            get(integration_sync::list_integration_sync_statuses),
        )
        .route(
            "/integrations/sync/feed",
            get(integration_sync::list_integration_sync_feed),
        )
        .route(
            "/integrations/sync/runs",
            get(integration_sync::list_integration_sync_runs),
        )
        .route(
            "/integrations/{id}/auth",
            get(integrations::get_integration_auth_url),
        )
        .route(
            "/integrations/{id}/sync",
            post(integration_sync::update_integration_sync_config),
        )
        .route(
            "/integrations/{id}/sync-now",
            post(integration_sync::run_integration_sync_now),
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
        .route(
            "/plugins",
            get(plugins::list_plugins).post(plugins::create_plugin),
        )
        .route("/plugins/logs", get(plugins::list_plugin_logs))
        .route(
            "/plugins/{id}",
            axum::routing::put(plugins::update_plugin).delete(plugins::delete_plugin),
        )
        .route("/plugins/{id}/refresh", post(plugins::refresh_plugin))
        .route("/plugins/{id}/test", post(plugins::test_plugin))
        .route(
            "/custom-apis",
            get(custom_apis::list_custom_apis).post(custom_apis::create_custom_api),
        )
        .route(
            "/custom-apis/preview",
            post(custom_apis::preview_custom_api),
        )
        .route(
            "/custom-apis/{id}",
            axum::routing::put(custom_apis::update_custom_api)
                .delete(custom_apis::delete_custom_api),
        )
        .route("/custom-apis/{id}/test", post(custom_apis::test_custom_api))
        .route(
            "/custom-messaging-channels",
            get(custom_messaging_channels::list_custom_messaging_channels)
                .post(custom_messaging_channels::create_custom_messaging_channel),
        )
        .route(
            "/custom-messaging-channels/{id}",
            axum::routing::put(custom_messaging_channels::update_custom_messaging_channel)
                .delete(custom_messaging_channels::delete_custom_messaging_channel),
        )
        .route(
            "/custom-messaging-channels/{id}/credentials",
            post(custom_messaging_channels::store_custom_messaging_channel_credentials),
        )
        .route(
            "/custom-messaging-channels/{id}/test",
            post(custom_messaging_channels::test_custom_messaging_channel),
        )
        .route(
            "/sender-verification",
            get(sender_verification::get_sender_verification),
        )
        .route(
            "/sender-verification/settings",
            post(sender_verification::update_sender_verification_settings),
        )
        .route(
            "/sender-verification/approve",
            post(sender_verification::approve_sender),
        )
        .route(
            "/sender-verification/revoke",
            post(sender_verification::revoke_sender),
        )
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
        .route(
            "/swarm/agents/builder/options",
            get(swarm_agent_builder_options),
        )
        .route("/swarm/agents/access-plan", post(swarm_agent_access_plan))
        .route("/swarm/agents/draft", post(swarm_draft_agent))
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
            "/conversations/{id}/latest-run",
            get(get_conversation_latest_run),
        )
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
        // Notification routes
        .route("/notifications", get(list_notifications_endpoint))
        .route("/notifications/stream", get(notification_stream_endpoint))
        .route("/notifications/read-all", post(mark_all_read_endpoint))
        .route("/notifications/{id}/read", post(mark_read_endpoint))
        .route("/notifications/count", get(notification_count_endpoint))
        // Analytics
        .route("/analytics/llm", get(llm_analytics_endpoint))
        .route("/reflect", get(ark_reflect_endpoint))
        .route("/reflect/refresh", post(ark_reflect_refresh_endpoint))
        .route(
            "/reflect/followups/{id}/feedback",
            post(ark_reflect_followup_feedback_endpoint),
        )
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
        // ArkOrbit (per-user limitless canvas) routes
        .route(
            "/api/arkorbit/orbits",
            get(arkorbit_control::list_orbits_endpoint)
                .post(arkorbit_control::create_orbit_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}",
            get(arkorbit_control::get_orbit_endpoint)
                .put(arkorbit_control::update_orbit_endpoint)
                .delete(arkorbit_control::delete_orbit_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/index",
            get(arkorbit_control::orbit_index_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/messages",
            get(arkorbit_control::orbit_messages_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/files",
            get(arkorbit_control::orbit_files_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/files/{*path}",
            get(arkorbit_control::orbit_file_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/widgets/{widget_id}",
            put(arkorbit_control::update_orbit_widget_endpoint)
                .delete(arkorbit_control::delete_orbit_widget_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/fetch",
            get(arkorbit_control::orbit_public_fetch_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/chat/transcripts",
            get(arkorbit_control::orbit_chat_transcripts_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/chat/transcripts/{transcript_id}",
            get(arkorbit_control::orbit_chat_transcript_messages_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/chat/reset",
            post(arkorbit_control::reset_orbit_chat_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/events",
            get(arkorbit_control::orbit_events_endpoint),
        )
        .route(
            "/api/arkorbit/orbits/{id}/chat",
            post(arkorbit_control::orbit_chat_endpoint),
        )
        .route(
            "/api/arkorbit/mod/{orbit_id}/{*path}",
            get(arkorbit_control::resolve_module_endpoint),
        )
        // Memory
        .route("/memory/stats", get(memory_stats))
        .route("/memory/facts", get(list_facts))
        .route(
            "/memory/facts/{id}",
            post(update_memory_fact_value).delete(delete_memory_fact),
        )
        .route("/channels/available", get(list_available_channels))
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
            "/memory/knowledge/sync-agentark-knowledge",
            post(sync_agentark_knowledge),
        )
        .route(
            "/memory/knowledge/{id}",
            axum::routing::delete(delete_knowledge_item),
        )
        // Memory operations
        .route("/arkmemory/summary", get(arkmemory_summary))
        .route(
            "/arkmemory/memories/{id}",
            post(update_memory_fact_value).delete(delete_memory_fact),
        )
        .route("/arkmemory/queue", get(arkmemory_queue))
        .route(
            "/arkmemory/queue/{id}/approve",
            post(arkmemory_approve_queue_item),
        )
        .route(
            "/arkmemory/queue/{id}/reject",
            post(arkmemory_reject_queue_item),
        )
        .route("/arkmemory/ledger", get(arkmemory_ledger))
        .route(
            "/arkmemory/ledger/{id}/rollback",
            post(arkmemory_rollback_ledger_event),
        )
        .route("/arkmemory/health", get(arkmemory_health))
        .route("/arkmemory/health/{id}/apply", post(arkmemory_apply_health))
        .route("/arkmemory/sources/{memory_id}", get(arkmemory_sources))
        .route("/arkmemory/tests", get(arkmemory_tests))
        .route("/arkmemory/tests/run", post(arkmemory_run_tests))
        .route("/arkmemory/cleanup", get(arkmemory_cleanup))
        .route("/arkmemory/cleanup/apply", post(arkmemory_apply_cleanup))
        // Legacy /arkrecall route aliases retained for old browser tabs.
        .route("/arkrecall/summary", get(arkmemory_summary))
        .route(
            "/arkrecall/memories/{id}",
            post(update_memory_fact_value).delete(delete_memory_fact),
        )
        .route("/arkrecall/queue", get(arkmemory_queue))
        .route(
            "/arkrecall/queue/{id}/approve",
            post(arkmemory_approve_queue_item),
        )
        .route(
            "/arkrecall/queue/{id}/reject",
            post(arkmemory_reject_queue_item),
        )
        .route("/arkrecall/ledger", get(arkmemory_ledger))
        .route(
            "/arkrecall/ledger/{id}/rollback",
            post(arkmemory_rollback_ledger_event),
        )
        .route("/arkrecall/health", get(arkmemory_health))
        .route("/arkrecall/health/{id}/apply", post(arkmemory_apply_health))
        .route("/arkrecall/sources/{memory_id}", get(arkmemory_sources))
        .route("/arkrecall/tests", get(arkmemory_tests))
        .route("/arkrecall/tests/run", post(arkmemory_run_tests))
        .route("/arkrecall/cleanup", get(arkmemory_cleanup))
        .route("/arkrecall/cleanup/apply", post(arkmemory_apply_cleanup))
        // Code execution sandbox
        .route("/code/execute", post(execute_code))
        // Hook routes
        .route("/hooks", get(list_hooks))
        .route("/hooks/runs", get(list_hook_runs))
        .route("/hooks", post(add_hook))
        .route("/hooks/{id}", axum::routing::delete(remove_hook))
        .route(
            "/webhooks/sources",
            get(webhooks::list_webhook_sources).post(webhooks::create_webhook_source),
        )
        .route(
            "/webhooks/sources/{id}",
            axum::routing::put(webhooks::update_webhook_source)
                .delete(webhooks::delete_webhook_source),
        )
        .route("/webhooks/events", get(webhooks::list_webhook_events))
        .route(
            "/webhooks/sources/{id}/test",
            post(webhooks::test_webhook_source),
        )
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
        .route(
            "/api/apps/{app_id}/quality_report",
            get(get_app_quality_report),
        )
        .route("/api/apps/{app_id}/stop", post(stop_app))
        .route("/api/apps/{app_id}/restart", post(restart_app))
        .route(
            "/api/apps/{app_id}/access-guard",
            post(update_app_access_guard),
        )
        .route("/api/apps/{app_id}/publish", post(publish_app))
        .route("/api/apps/{app_id}", axum::routing::delete(delete_app))
        .route(
            "/api/applications",
            get(applications::list_application_launchers),
        )
        .route(
            "/api/applications/{app_id}/launch",
            post(applications::launch_application),
        )
        .route(
            "/api/applications/{app_id}/stop",
            post(applications::stop_application),
        )
        // Output file serving (code execution artifacts)
        .route("/api/outputs/{exec_id}/{filename}", get(serve_output_file))
        .route(
            "/api/outputs/{exec_id}/{filename}/download",
            get(download_output_file),
        )
        // File upload for chat attachments
        .route("/api/upload", post(upload_chat_file))
        .route("/api/uploads/{upload_id}", get(serve_upload_file))
        // Approval audit log
        .route("/approvals/log", get(get_approval_log))
        .route("/approvals/{id}/dismiss", post(dismiss_approval))
        // Security event log
        .route("/security/logs", get(get_security_logs))
        // Security / Master Password
        .route("/security/status", get(security_status))
        .route(
            "/security/internal-service-tokens/rotate",
            post(rotate_internal_service_tokens),
        )
        .route("/security/set-password", post(set_master_password))
        .route("/security/change-password", post(change_master_password))
        .route("/security/remove-password", post(remove_master_password))
        // Tunnel management
        .route("/tunnel/status", get(tunnel::get_tunnel_status))
        .route("/tunnel/start", post(tunnel::start_tunnel))
        .route("/tunnel/stop", post(tunnel::stop_tunnel))
        // Watchers
        .route("/watchers", get(get_watchers))
        .route("/watchers/pause-all", post(pause_all_watchers))
        .route("/watchers/resume-all", post(resume_all_watchers))
        .route("/watchers/{id}", axum::routing::delete(delete_watcher))
        .route("/watchers/{id}/cancel", post(cancel_watcher))
        .route("/watchers/{id}/pause", post(pause_watcher))
        .route("/watchers/{id}/resume", post(resume_watcher))
        .route("/watchers/{id}/run-now", post(run_watcher_now))
        .route("/watchers/{id}/extend", post(extend_watcher))
        // Persisted execution runs
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/stream", get(stream_run_events))
        .route("/runs/{id}/cancel", post(cancel_run))
        .route("/runs/{id}/resume", post(resume_run))
        // Greetings (LLM-generated, cached in DB)
        // Pulse log
        .route("/arkpulse", get(get_pulse_log))
        .route("/arkpulse/trigger", post(trigger_pulse))
        .route("/arkpulse/fix", post(run_arkpulse_fix))
        .route("/arkpulse/cleanup-preview", post(arkpulse_cleanup_preview))
        .route("/arkpulse/cleanup", post(run_arkpulse_cleanup))
        .route("/arkpulse/cleanup/{job_id}", get(get_arkpulse_cleanup_job))
        // Browser automation sessions
        .route("/browser/sessions", get(browser_list_sessions))
        .route("/browser/sessions/{id}", get(browser_session_status))
        .route("/browser/sessions/{id}/respond", post(browser_respond))
        .route("/browser/sessions/{id}/claim", post(browser_claim))
        .route("/browser/sessions/{id}/release", post(browser_release))
        .route("/browser/sessions/{id}/complete", post(browser_complete))
        .route("/browser/sessions/{id}/stop", post(browser_stop))
        .route("/browser/sessions/{id}", delete(browser_delete))
        .route("/browser/sessions/{id}/status", get(browser_session_status))
        // WhatsApp bridge proxy (so web UI can reach the sidecar)
        .route("/api/whatsapp-bridge/status", get(whatsapp_bridge_status))
        .route("/api/whatsapp-bridge/logout", post(whatsapp_bridge_logout))
        .route("/api/telegram/status", get(telegram_channel_status))
        // Apply authentication middleware (inner layer, runs after rate limiting)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        // Apply rate limiting first so invalid credentials still count.
        // Verified same-origin UI sessions bypass inside the middleware.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ));

    // CORS layer - allow explicit configured origins + exact active tunnel origin.
    // Same-origin UI access does not need CORS, and trusted-local mode no longer
    // treats arbitrary localhost ports as equivalent.
    let tunnel_for_cors = state.tunnel.clone();
    let explicit_origins: HashSet<String> = std::env::var("AGENTARK_ALLOWED_ORIGINS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .filter_map(normalize_origin)
                .collect::<HashSet<String>>()
        })
        .unwrap_or_default();
    if !explicit_origins.is_empty() {
        tracing::info!(
            "Additional allowed CORS origins configured: {}",
            explicit_origins.len()
        );
    }
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _| {
            if let Ok(origin_str) = origin.to_str() {
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

    // Keep handles for auto-start and Sentinel
    let tunnel_handle = state.tunnel.clone();
    let wa_bridge_handle = state.whatsapp_bridge.clone();

    let app = public_routes
        .merge(protected_routes)
        .with_state(state.clone())
        .layer(DefaultBodyLimit::max(MAX_HTTP_BODY_BYTES))
        .layer(middleware::from_fn(metrics_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            tunnel_exposure_middleware,
        ))
        .layer(cors)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers_middleware,
        ));

    let isolate_public_apps = internet_facing_apps_should_be_isolated(
        state.deployment_mode,
        state.public_app_bind_addr.as_deref(),
    );
    let mut public_app_server: Option<tokio::task::JoinHandle<()>> = None;
    if isolate_public_apps {
        #[cfg(feature = "tls")]
        let public_app_tls_paths = {
            let agent_guard = state.agent.read().await;
            (
                agent_guard.config.tls_cert_path.clone(),
                agent_guard.config.tls_key_path.clone(),
            )
        };
        let mut public_app_state = state.clone();
        public_app_state.server_role = HttpServerRole::PublicApps;
        public_app_state.local_ui_bootstrap_enabled = false;
        public_app_state.allow_insecure_no_auth = false;
        let public_app_routes = Router::new()
            .route("/health", get(health))
            .route("/readiness", get(readiness))
            .route("/public/proxy/raw", get(public_proxy_raw))
            .route("/apps/{app_id}", any(serve_app_root))
            .route("/apps/{app_id}/", any(serve_app_root))
            .route("/apps/{app_id}/{*path}", any(serve_app_path))
            .with_state(public_app_state.clone())
            .layer(DefaultBodyLimit::max(MAX_HTTP_BODY_BYTES))
            .layer(middleware::from_fn(metrics_middleware))
            .layer(middleware::from_fn_with_state(
                public_app_state.clone(),
                security_headers_middleware,
            ));
        let public_app_bind_addr = state
            .public_app_bind_addr
            .clone()
            .unwrap_or("127.0.0.1:8992".to_string());
        let public_app_listener = tokio::net::TcpListener::bind(&public_app_bind_addr).await?;
        #[cfg(feature = "tls")]
        let public_app_bind_is_loopback = bind_addr_is_loopback(&public_app_bind_addr);
        let mut app_shutdown = shutdown_rx.clone();
        public_app_server = Some(tokio::spawn(async move {
            #[cfg(feature = "tls")]
            if !public_app_bind_is_loopback {
                if let (Some(cert_path), Some(key_path)) = public_app_tls_paths.clone() {
                    match public_app_bind_addr.parse::<SocketAddr>() {
                        Ok(addr) => match axum_server::tls_rustls::RustlsConfig::from_pem_file(
                            &cert_path, &key_path,
                        )
                        .await
                        {
                            Ok(rustls_config) => {
                                let handle = axum_server::Handle::new();
                                let shutdown_handle = handle.clone();
                                let mut tls_shutdown = app_shutdown.clone();
                                let shutdown_task = tokio::spawn(async move {
                                    let _ = tls_shutdown.changed().await;
                                    shutdown_handle.graceful_shutdown(Some(
                                        std::time::Duration::from_secs(10),
                                    ));
                                });
                                tracing::info!(
                                    "Public app server listening on https://{}",
                                    public_app_bind_addr
                                );
                                let result = axum_server::bind_rustls(addr, rustls_config)
                                    .handle(handle)
                                    .serve(
                                        public_app_routes
                                            .into_make_service_with_connect_info::<SocketAddr>(),
                                    )
                                    .await;
                                let _ = shutdown_task.await;
                                if let Err(error) = result {
                                    tracing::error!(
                                        "Public HTTPS app server on {} exited with error: {}",
                                        public_app_bind_addr,
                                        error
                                    );
                                }
                                return;
                            }
                            Err(error) => {
                                tracing::error!(
                                    "Failed to load TLS certs for public app listener {}: {}",
                                    public_app_bind_addr,
                                    error
                                );
                                return;
                            }
                        },
                        Err(error) => {
                            tracing::error!(
                                "Invalid public app bind address {} for direct HTTPS listener: {}",
                                public_app_bind_addr,
                                error
                            );
                            return;
                        }
                    }
                }
            }
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
        crate::spawn_logged!("src/channels/http.rs:4874", async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            tracing::info!("AGENTARK_TUNNEL=true - auto-starting remote access tunnel...");
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
                Ok(()) => {
                    let mut tunnel = state_for_tunnel.tunnel.write().await;
                    tunnel.selected_app_id = None;
                    tunnel.exposed_app_ids.clear();
                    tunnel.control_plane_enabled = true;
                    drop(tunnel);
                    tracing::info!("Tunnel auto-started successfully")
                }
                Err(e) => tracing::error!("Failed to auto-start tunnel: {}", e),
            }
        });
    } else {
        let state_for_tunnel = state.clone();
        crate::spawn_logged!("src/channels/http.rs:5444", async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match tunnel::auto_start_selected_app_tunnel(&state_for_tunnel).await {
                Ok(Some(url)) => tracing::info!(
                    "Auto-restored app-only public tunnel for saved app exposure: {}",
                    url
                ),
                Ok(None) => tracing::info!(
                    "No saved public app exposure; remote tunnel infrastructure was not auto-started"
                ),
                Err(error) => {
                    tracing::warn!("Skipped app-only public tunnel auto-restore: {}", error);
                }
            }
        });
    }

    // Auto-start the bundled WhatsApp bridge only when Baileys + embedded mode is enabled.
    if should_manage_embedded_whatsapp_bridge(&state).await {
        let state_for_bridge = state.clone();
        crate::spawn_logged!("src/channels/http.rs:4897", async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            tracing::info!("Auto-starting bundled WhatsApp bridge...");
            match spawn_whatsapp_bridge(state_for_bridge).await {
                Ok(()) => tracing::info!("WhatsApp bridge started"),
                Err(e) => tracing::warn!("WhatsApp bridge unavailable: {}", e),
            }
        });
    }

    schedule_enabled_mcp_server_resumes(state.agent.clone());

    // Sentinel: monitor tunnel + WhatsApp bridge processes, auto-restart if they die
    {
        let t = tunnel_handle.clone();
        let state_for_tunnel = state.clone();
        let wb = wa_bridge_handle.clone();
        let state_for_bridge = state.clone();
        crate::spawn_logged!("src/channels/http.rs:4915", async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
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
                    tracing::info!("Sentinel: restarting tunnel...");
                    match tunnel::spawn_tunnel(&state_for_tunnel, None).await {
                        Ok(()) => tracing::info!("Sentinel: tunnel restarted successfully"),
                        Err(e) => tracing::error!("Sentinel: failed to restart tunnel: {}", e),
                    }
                }

                // Check WhatsApp bridge process health (only if it's supposed to be running)
                let wa_needs_restart = {
                    let mut bridge = wb.write().await;
                    if bridge.active {
                        if let Some(ref mut child) = bridge.process {
                            match child.try_wait() {
                                Ok(Some(_status)) => {
                                    tracing::warn!(
                                        "Sentinel: WhatsApp bridge exited unexpectedly, will restart..."
                                    );
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
                    tracing::info!("Sentinel: restarting WhatsApp bridge...");
                    match spawn_whatsapp_bridge(state_for_bridge.clone()).await {
                        Ok(()) => {
                            tracing::info!("Sentinel: WhatsApp bridge restarted successfully")
                        }
                        Err(e) => {
                            tracing::error!("Sentinel: failed to restart WhatsApp bridge: {}", e)
                        }
                    }
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        });
    }

    // Chat suggestion scanner: sweep chat wishes on a controlled cadence and defer while busy.
    {
        let state_for_suggestions = state.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/channels/http.rs:5085", async move {
            let mut timeout_streak = 0u32;
            loop {
                let storage = { state_for_suggestions.agent.read().await.storage.clone() };
                let settings = load_autonomy_settings_from_storage(&storage).await;
                if autonomy_background_disabled(&settings) {
                    timeout_streak = 0;
                    if !sleep_or_http_shutdown(
                        std::time::Duration::from_secs(OPTIONAL_BACKGROUND_POLL_SECS),
                        &mut shutdown,
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }

                let scan_state = load_chat_suggestion_scan_state(&storage).await;
                let next_due_at = scan_state
                    .next_due_at
                    .as_deref()
                    .and_then(parse_utc_rfc3339);
                let wait_for = if chat_suggestion_scan_is_due(&scan_state, chrono::Utc::now()) {
                    std::time::Duration::from_secs(0)
                } else {
                    next_background_sleep_duration(next_due_at)
                };
                if wait_for > std::time::Duration::from_secs(0) {
                    if !sleep_or_http_shutdown(wait_for, &mut shutdown).await {
                        break;
                    }
                    continue;
                }

                tracing::debug!("Chat suggestion scan started");
                let outcome = tokio::select! {
                    _ = shutdown.changed() => break,
                    result = tokio::time::timeout(
                        std::time::Duration::from_secs(OPTIONAL_BACKGROUND_JOB_TIMEOUT_SECS),
                        run_chat_suggestion_scan(&state_for_suggestions, "scheduler"),
                    ) => result,
                };
                match outcome {
                    Ok(result) => {
                        timeout_streak = 0;
                        tracing::debug!("Chat suggestion scan completed");
                        if result.get("status").and_then(|value| value.as_str())
                            == Some("completed")
                        {
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
                    Err(_) => {
                        timeout_streak = timeout_streak.saturating_add(1);
                        let backoff = optional_background_timeout_backoff(timeout_streak);
                        tracing::warn!(
                            "Chat suggestion scan timed out after {}s (streak={}, next retry in {}s)",
                            OPTIONAL_BACKGROUND_JOB_TIMEOUT_SECS,
                            timeout_streak,
                            backoff.as_secs()
                        );
                        if !sleep_or_http_shutdown(backoff, &mut shutdown).await {
                            break;
                        }
                        continue;
                    }
                }

                if !sleep_or_http_shutdown(
                    std::time::Duration::from_secs(OPTIONAL_BACKGROUND_POLL_SECS),
                    &mut shutdown,
                )
                .await
                {
                    break;
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
        let display_url = display_url_for_bind_addr(&bind_addr, "https")
            .unwrap_or_else(|| format!("https://{}", bind_addr.trim_end_matches('/')));
        tracing::info!("HTTPS server listening on https://{}", bind_addr);
        tracing::info!("Web UI available at {}/", display_url.trim_end_matches('/'));
        if should_warn_for_direct_control_plane_exposure(state.deployment_mode, &bind_addr) {
            tracing::warn!(
                "Internet-facing control plane is listening on '{}'. Ensure API/UI authentication remains enabled.",
                bind_addr
            );
        }
        let handle = axum_server::Handle::new();
        let mut tls_shutdown = shutdown_rx.clone();
        let shutdown_task = {
            let handle = handle.clone();
            crate::spawn_logged!("src/channels/http.rs:5208", async move {
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
    let display_url = display_url_for_bind_addr(&bind_addr, "http")
        .unwrap_or_else(|| format!("http://{}", bind_addr.trim_end_matches('/')));
    tracing::info!("HTTP server listening on http://{}", bind_addr);
    tracing::info!("Web UI available at {}/", display_url.trim_end_matches('/'));
    if should_warn_for_direct_control_plane_exposure(state.deployment_mode, &bind_addr) {
        tracing::warn!(
            "Internet-facing control plane is listening on '{}'. Ensure API/UI authentication remains enabled.",
            bind_addr
        );
        tracing::warn!(
            "Internet-facing control plane is using direct HTTP without TLS. Put it behind HTTPS or use secure remote access before exposing it publicly."
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

#[cfg(test)]
mod tests;
