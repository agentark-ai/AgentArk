//! Agent configuration with encryption for sensitive data
//!
//! Non-sensitive config is stored in config.toml (readable)
//! Sensitive data (API keys, tokens) is stored encrypted in secrets.enc

use super::llm::LlmProvider;
use super::runtime_image;
use super::swarm::SwarmConfig;
use crate::channels::{
    discord::DiscordChannelConfig, google_chat::GoogleChatChannelConfig,
    imessage::IMessageChannelConfig, line::LineChannelConfig, matrix::MatrixTransportConfig,
    qq::QqChannelConfig, signal::SignalChannelConfig, slack::SlackChannelConfig,
    teams::TeamsTransportConfig, wechat::WeChatChannelConfig, whatsapp::WhatsAppChannelConfig,
};
use crate::crypto::KeyManager;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Global key manager set at startup (from master password or keyfile).
/// All SecureConfigManager instances use this when available, ensuring
/// consistent encryption across the entire process after password changes.
static GLOBAL_KEY_MANAGER: std::sync::OnceLock<std::sync::RwLock<Option<Arc<KeyManager>>>> =
    std::sync::OnceLock::new();
static SECRETS_FILE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
pub const HTTP_API_KEY_TTL_SECS: i64 = 24 * 60 * 60;

fn global_key_manager_cell() -> &'static std::sync::RwLock<Option<Arc<KeyManager>>> {
    GLOBAL_KEY_MANAGER.get_or_init(|| std::sync::RwLock::new(None))
}

fn secrets_file_lock() -> &'static std::sync::Mutex<()> {
    SECRETS_FILE_LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// Set or replace the global key manager (called at startup and after password rotation)
pub fn set_global_key_manager(km: Arc<KeyManager>) {
    if let Ok(mut guard) = global_key_manager_cell().write() {
        *guard = Some(km);
    }
}

/// Get the global key manager if set
pub fn global_key_manager() -> Option<Arc<KeyManager>> {
    global_key_manager_cell()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

/// Role determines when a model slot is used
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    /// Main model for complex reasoning (default fallback for everything)
    Primary,
    /// Fast/cheap model for simple queries
    Fast,
    /// Specialized for code tasks
    Code,
    /// Deep research via OpenRouter Perplexity
    Research,
    /// Generic fallback (used if primary fails)
    Fallback,
}

/// Relative capability level for a configured model slot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapabilityTier {
    Economy,
    #[default]
    Balanced,
    Premium,
}

/// Relative cost level for a configured model slot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelCostTier {
    Low,
    #[default]
    Medium,
    High,
}

/// Scope used when recording model/runtime health.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelHealthScope {
    #[default]
    Provider,
    Slot,
    Session,
}

/// A single model slot in the pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSlot {
    /// Unique ID for this slot
    pub id: String,
    /// Human-readable label (e.g., "Primary", "Fast", "Code Expert")
    pub label: String,
    /// Role determines when this model is used
    pub role: ModelRole,
    /// The LLM provider config
    pub provider: LlmProvider,
    /// Whether this slot is active
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Relative capability tier used for escalation decisions.
    #[serde(default)]
    pub capability_tier: ModelCapabilityTier,
    /// Relative cost tier used to prefer cheaper models first.
    #[serde(default)]
    pub cost_tier: ModelCostTier,
    /// Whether the supervisor may auto-step-up into this slot.
    #[serde(default = "default_true")]
    pub auto_escalate: bool,
    /// Lower values are tried earlier within the same role/cost band.
    #[serde(default)]
    pub escalation_rank: i32,
    /// Scope used for health/cooldown bookkeeping.
    #[serde(default)]
    pub health_scope: ModelHealthScope,
}

/// Multi-model pool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPoolConfig {
    /// All configured model slots
    #[serde(default)]
    pub slots: Vec<ModelSlot>,
    /// Enable smart routing (if false, always use primary)
    #[serde(default = "default_true")]
    pub smart_routing: bool,
}

impl Default for ModelPoolConfig {
    fn default() -> Self {
        Self {
            slots: vec![],
            smart_routing: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TunnelProviderKind {
    #[default]
    Cloudflare,
    Ngrok,
    TailscalePrivate,
    TailscaleFunnel,
    Bore,
}

impl TunnelProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::Ngrok => "ngrok",
            Self::TailscalePrivate => "tailscale_private",
            Self::TailscaleFunnel => "tailscale_funnel",
            Self::Bore => "bore",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelCloudflareConfig {
    #[serde(default = "default_cloudflared_binary")]
    pub binary_path: String,
}

impl Default for TunnelCloudflareConfig {
    fn default() -> Self {
        Self {
            binary_path: default_cloudflared_binary(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelNgrokConfig {
    #[serde(default = "default_ngrok_binary")]
    pub binary_path: String,
    #[serde(default)]
    pub authtoken: String,
    #[serde(default)]
    pub domain: Option<String>,
}

impl Default for TunnelNgrokConfig {
    fn default() -> Self {
        Self {
            binary_path: default_ngrok_binary(),
            authtoken: String::new(),
            domain: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelTailscaleConfig {
    #[serde(default = "default_tailscale_binary")]
    pub binary_path: String,
    #[serde(default)]
    pub auth_key: String,
    #[serde(default)]
    pub hostname: Option<String>,
}

impl Default for TunnelTailscaleConfig {
    fn default() -> Self {
        Self {
            binary_path: default_tailscale_binary(),
            auth_key: String::new(),
            hostname: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelBoreConfig {
    #[serde(default = "default_bore_binary")]
    pub binary_path: String,
    #[serde(default = "default_bore_server")]
    pub server: String,
}

impl Default for TunnelBoreConfig {
    fn default() -> Self {
        Self {
            binary_path: default_bore_binary(),
            server: default_bore_server(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TunnelConfig {
    #[serde(default)]
    pub provider: TunnelProviderKind,
    #[serde(default)]
    pub cloudflare: TunnelCloudflareConfig,
    #[serde(default)]
    pub ngrok: TunnelNgrokConfig,
    #[serde(default)]
    pub tailscale_funnel: TunnelTailscaleConfig,
    #[serde(default)]
    pub bore: TunnelBoreConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_observability_provider")]
    pub provider: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default = "default_observability_service_name")]
    pub service_name: String,
    #[serde(default = "default_observability_header_name")]
    pub header_name: String,
    #[serde(default = "default_observability_privacy_mode")]
    pub privacy_mode: String,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_observability_provider(),
            endpoint: String::new(),
            service_name: default_observability_service_name(),
            header_name: default_observability_header_name(),
            privacy_mode: default_observability_privacy_mode(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    #[default]
    TrustedLocal,
    InternetFacing,
}

impl DeploymentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrustedLocal => "trusted_local",
            Self::InternetFacing => "internet_facing",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublicAppsConfig {
    /// Optional dedicated bind address for the public app listener.
    /// When unset, trusted-local mode continues to serve apps from the control plane.
    #[serde(default)]
    pub bind_addr: Option<String>,
    /// Optional externally reachable base URL for public apps.
    /// Example: https://apps.example.com or http://localhost:8992
    #[serde(default)]
    pub base_url: Option<String>,
}

fn default_local_embeddings_model() -> String {
    "BAAI/bge-small-en-v1.5".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingsProviderKind {
    #[default]
    LocalHf,
    Ollama,
    OpenaiCompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    #[serde(default)]
    pub provider: EmbeddingsProviderKind,
    #[serde(default = "default_local_embeddings_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: String,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingsProviderKind::LocalHf,
            model: default_local_embeddings_model(),
            base_url: None,
            api_key: String::new(),
        }
    }
}

/// Main agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    #[serde(default = "default_personality")]
    pub personality: String,
    /// Primary LLM provider (legacy - migrated to model_pool on load)
    pub llm: LlmProvider,
    /// Fallback LLM provider (legacy)
    #[serde(default)]
    pub llm_fallback: Option<LlmProvider>,
    /// Multi-model pool
    #[serde(default)]
    pub model_pool: ModelPoolConfig,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub slack: Option<SlackChannelConfig>,
    #[serde(default)]
    pub discord: Option<DiscordChannelConfig>,
    #[serde(default)]
    pub matrix: Option<MatrixTransportConfig>,
    #[serde(default)]
    pub teams: Option<TeamsTransportConfig>,
    #[serde(default)]
    pub whatsapp: Option<WhatsAppChannelConfig>,
    #[serde(default)]
    pub google_chat: Option<GoogleChatChannelConfig>,
    #[serde(default)]
    pub signal: Option<SignalChannelConfig>,
    #[serde(default)]
    pub imessage: Option<IMessageChannelConfig>,
    #[serde(default)]
    pub line: Option<LineChannelConfig>,
    #[serde(default)]
    pub wechat: Option<WeChatChannelConfig>,
    #[serde(default)]
    pub qq: Option<QqChannelConfig>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    /// Embeddings backend configuration.
    /// Stored separately from chat models so dense retrieval does not inherit chat provider defaults.
    #[serde(default)]
    pub embeddings: Option<EmbeddingsConfig>,
    #[serde(default)]
    pub auto_approve: Vec<String>,
    /// Media generation settings
    #[serde(default)]
    pub media_gen: MediaGenConfig,
    /// Swarm multi-agent configuration
    #[serde(default)]
    pub swarm: SwarmConfig,
    /// Browser automation configuration
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Public remote-access tunnel configuration
    #[serde(default)]
    pub tunnel: TunnelConfig,
    /// Optional external trace export
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Deployment/security posture for the control plane.
    #[serde(default)]
    pub deployment_mode: DeploymentMode,
    /// Dedicated public-app exposure settings.
    #[serde(default)]
    pub public_apps: PublicAppsConfig,
    /// MCP (Model Context Protocol) external servers
    #[serde(default)]
    pub mcp: McpConfig,
    /// TLS certificate path (enables HTTPS when set with tls_key_path)
    #[serde(default)]
    pub tls_cert_path: Option<String>,
    /// TLS private key path
    #[serde(default)]
    pub tls_key_path: Option<String>,
}

fn default_personality() -> String {
    "friendly".to_string()
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: crate::branding::default_agent_name(),
            personality: default_personality(),
            llm: LlmProvider::default(),
            llm_fallback: None,
            model_pool: ModelPoolConfig::default(),
            telegram: None,
            slack: None,
            discord: None,
            matrix: None,
            teams: None,
            whatsapp: None,
            google_chat: None,
            signal: None,
            imessage: None,
            line: None,
            wechat: None,
            qq: None,
            sandbox: SandboxConfig::default(),
            memory: MemoryConfig::default(),
            embeddings: Some(EmbeddingsConfig::default()),
            auto_approve: vec![],
            media_gen: MediaGenConfig::default(),
            swarm: SwarmConfig::default(),
            browser: BrowserConfig::default(),
            tunnel: TunnelConfig::default(),
            observability: ObservabilityConfig::default(),
            deployment_mode: DeploymentMode::default(),
            public_apps: PublicAppsConfig::default(),
            mcp: McpConfig::default(),
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

impl AgentConfig {
    pub fn embeddings_config(&self) -> EmbeddingsConfig {
        self.embeddings.clone().unwrap_or_default()
    }
}

/// Media generation configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaGenConfig {
    /// Default provider for image generation
    #[serde(default)]
    pub default_image_provider: Option<String>,
    /// Image model name override
    #[serde(default)]
    pub image_model: Option<String>,
    /// Fallback provider for image generation
    #[serde(default)]
    pub fallback_image_provider: Option<String>,
    /// Default provider for video generation
    #[serde(default)]
    pub default_video_provider: Option<String>,
    /// Fallback provider for video generation
    #[serde(default)]
    pub fallback_video_provider: Option<String>,
    /// API keys for media providers (stored encrypted in secrets.enc)
    /// Keys: replicate, stability_ai, fal, together, openai, google, runway, luma
    #[serde(default)]
    pub provider_api_keys: std::collections::HashMap<String, String>,
}

/// Browser automation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// URL of the Playwright bridge
    #[serde(default = "default_browser_bridge_url")]
    pub bridge_url: String,
    /// Maximum concurrent browser sessions
    #[serde(default = "default_max_browser_sessions")]
    pub max_sessions: usize,
    /// Session inactivity timeout in seconds
    #[serde(default = "default_browser_session_timeout")]
    pub session_timeout_secs: u64,
    /// Maximum iterations per browser session
    #[serde(default = "default_browser_max_iterations")]
    pub max_iterations: u32,
}

fn default_browser_bridge_url() -> String {
    "http://127.0.0.1:3100".to_string()
}

fn default_cloudflared_binary() -> String {
    if cfg!(windows) {
        "cloudflared.exe".to_string()
    } else {
        "cloudflared".to_string()
    }
}

fn default_ngrok_binary() -> String {
    "ngrok".to_string()
}

fn default_tailscale_binary() -> String {
    "tailscale".to_string()
}

fn default_bore_binary() -> String {
    "bore".to_string()
}

fn default_bore_server() -> String {
    "bore.pub".to_string()
}

fn default_observability_provider() -> String {
    "langtrace".to_string()
}

fn default_observability_service_name() -> String {
    "agentark".to_string()
}

fn default_observability_header_name() -> String {
    "x-api-key".to_string()
}

fn default_observability_privacy_mode() -> String {
    "metadata_only".to_string()
}

fn default_max_browser_sessions() -> usize {
    2
}

fn default_browser_session_timeout() -> u64 {
    900 // 15 minutes
}

fn default_browser_max_iterations() -> u32 {
    30
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            bridge_url: default_browser_bridge_url(),
            max_sessions: default_max_browser_sessions(),
            session_timeout_secs: default_browser_session_timeout(),
            max_iterations: default_browser_max_iterations(),
        }
    }
}

/// MCP (Model Context Protocol) configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    /// Configured MCP servers
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// MCP server configuration (non-sensitive)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Stable ID (UUID)
    pub id: String,
    /// Display name
    pub name: String,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// Transport settings
    pub transport: McpTransportConfig,
    /// Whether this server is enabled (tools registered)
    #[serde(default)]
    pub enabled: bool,
    /// Whether resources are enabled (disabled by default)
    #[serde(default)]
    pub resources_enabled: bool,
    /// Optional auth configuration (secrets stored separately)
    #[serde(default)]
    pub auth: Option<McpAuthConfig>,
    /// Optional generic auth profile binding used instead of inline MCP secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    /// Allowlist of tool names (empty = all tools allowed)
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// Blocklist of tool names (takes precedence over allowlist)
    #[serde(default)]
    pub tool_blocklist: Vec<String>,
    /// Allowlist of resource URIs (empty = all resources allowed)
    #[serde(default)]
    pub resource_allowlist: Vec<String>,
    /// Request timeout in seconds
    #[serde(default = "default_mcp_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum response size in bytes
    #[serde(default = "default_mcp_max_response_bytes")]
    pub max_response_bytes: usize,
}

fn default_mcp_timeout_secs() -> u64 {
    15
}

fn default_mcp_max_response_bytes() -> usize {
    1024 * 1024 // 1 MB
}

/// MCP transport configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportConfig {
    /// HTTP JSON-RPC endpoint
    Http { url: String },
    /// Local stdio server
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
        /// Names of env vars whose values are stored encrypted in secrets.enc.
        #[serde(default)]
        env_keys: Vec<String>,
    },
}

/// MCP authentication configuration (secrets stored encrypted)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpAuthConfig {
    /// Bearer token in header (default "Authorization")
    Bearer {
        #[serde(default = "default_auth_header")]
        header: String,
    },
    /// HTTP Basic auth (username/password stored encrypted)
    Basic,
    /// Custom header (value stored encrypted)
    Header { name: String },
    /// Query parameter (value stored encrypted)
    Query { name: String },
}

fn default_auth_header() -> String {
    "Authorization".to_string()
}

/// Actions that can NEVER be added to auto_approve, regardless of user settings.
/// These actions have destructive potential and must always go through safety review.
pub const AUTO_APPROVE_BLOCKED: &[&str] = &[
    "shell",
    "bash",
    "code_execute",
    "file_write",
    "file_delete",
    "file_move",
    "docker_exec",
    "http_request",
    "gmail_send", // Sending unsolicited emails is always gated
                  // gmail_reply is intentionally NOT blocked — user can enable auto-reply in settings
];

/// Filter the user-facing auto-approve list down to effective action-name overrides.
///
/// This trims whitespace, drops empty entries, removes blocked actions, and deduplicates
/// while preserving the first-seen order.
pub fn sanitize_auto_approve_actions(list: &[String]) -> Vec<String> {
    let mut sanitized = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for action in list {
        let trimmed = action.trim();
        if trimmed.is_empty() || AUTO_APPROVE_BLOCKED.contains(&trimmed) {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            sanitized.push(trimmed.to_string());
        }
    }

    sanitized
}

/// Encrypted secrets storage
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Secrets {
    /// Primary LLM API key
    pub llm_api_key: Option<String>,
    /// External embeddings API key
    #[serde(default)]
    pub embeddings_api_key: Option<String>,
    /// Fallback LLM API key
    pub llm_fallback_api_key: Option<String>,
    /// Telegram bot token
    pub telegram_bot_token: Option<String>,
    /// Slack bot token
    pub slack_bot_token: Option<String>,
    /// Slack signing secret
    pub slack_signing_secret: Option<String>,
    /// Discord bot token
    pub discord_bot_token: Option<String>,
    /// Matrix access token
    pub matrix_access_token: Option<String>,
    /// Teams access token
    pub teams_access_token: Option<String>,
    /// WhatsApp access token
    #[serde(default)]
    pub whatsapp_access_token: Option<String>,
    /// WhatsApp Cloud API app secret
    #[serde(default)]
    pub whatsapp_app_secret: Option<String>,
    /// WhatsApp bridge access token
    #[serde(default)]
    pub whatsapp_bridge_token: Option<String>,
    /// Google Chat access token
    #[serde(default)]
    pub google_chat_access_token: Option<String>,
    /// Google Chat verification token
    #[serde(default)]
    pub google_chat_verify_token: Option<String>,
    /// Signal bridge access token
    #[serde(default)]
    pub signal_bridge_token: Option<String>,
    /// iMessage bridge access token
    #[serde(default)]
    pub imessage_bridge_token: Option<String>,
    /// LINE access token
    #[serde(default)]
    pub line_channel_access_token: Option<String>,
    /// LINE channel secret
    #[serde(default)]
    pub line_channel_secret: Option<String>,
    /// WeChat bridge access token
    #[serde(default)]
    pub wechat_bridge_token: Option<String>,
    /// QQ bridge access token
    #[serde(default)]
    pub qq_bridge_token: Option<String>,
    /// Tunnel provider auth tokens/keys
    #[serde(default)]
    pub tunnel_ngrok_authtoken: Option<String>,
    #[serde(default)]
    pub tunnel_tailscale_auth_key: Option<String>,
    /// HTTP API authentication key (auto-generated on first run)
    #[serde(default)]
    pub api_key: Option<String>,
    /// Unix timestamp when HTTP API key was issued
    #[serde(default)]
    pub api_key_issued_at: Option<i64>,
    /// Unix timestamp when HTTP API key expires
    #[serde(default)]
    pub api_key_expires_at: Option<i64>,
    /// Media generation provider API keys (encrypted)
    #[serde(default)]
    pub media_provider_keys: std::collections::HashMap<String, String>,
    /// Model pool API keys, keyed by slot ID
    #[serde(default)]
    pub model_pool_keys: std::collections::HashMap<String, String>,
    /// MCP server auth secrets (per server ID)
    #[serde(default)]
    pub mcp_auth: std::collections::HashMap<String, McpAuthSecret>,
    /// MCP stdio env vars (per server ID, encrypted at rest)
    #[serde(default)]
    pub mcp_env: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// Custom secrets (for future extensibility)
    #[serde(default)]
    pub custom: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpApiKeyInfo {
    pub key: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// MCP auth secrets (stored encrypted)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpAuthSecret {
    /// Shared token (Bearer, custom header, or query param)
    #[serde(default)]
    pub token: Option<String>,
    /// Username for Basic auth
    #[serde(default)]
    pub username: Option<String>,
    /// Password for Basic auth
    #[serde(default)]
    pub password: Option<String>,
}

/// Secure configuration manager
/// Handles encryption/decryption of sensitive data
pub struct SecureConfigManager {
    key_manager: Arc<KeyManager>,
    config_dir: std::path::PathBuf,
}

impl SecureConfigManager {
    /// Create a new secure config manager
    /// Optionally accepts a data_dir for keyfile separation (security hardening)
    pub fn new(config_dir: &Path) -> Result<Self> {
        Self::new_with_data_dir(config_dir, None)
    }

    /// Create with separate data_dir (keyfile always lives in config_dir for consistency).
    /// Prefers the global key manager (set at startup from master password) so that all
    /// code paths use the same encryption key, even after password changes.
    pub fn new_with_data_dir(config_dir: &Path, data_dir: Option<&Path>) -> Result<Self> {
        // Prefer global key (master-password-derived) when available
        if let Some(km) = global_key_manager() {
            return Ok(Self {
                key_manager: km,
                config_dir: config_dir.to_path_buf(),
            });
        }

        // Fallback: keyfile-based (pre-startup or tests)
        let keyfile = config_dir.join(".keyfile");

        // Reverse-migration: if keyfile was previously moved to data_dir, copy it back
        if let Some(dd) = data_dir {
            let old_data_keyfile = dd.join(".keyfile");
            if old_data_keyfile.exists() && !keyfile.exists() {
                tracing::info!("Moving keyfile back to config_dir for consistency");
                if let Err(e) = std::fs::copy(&old_data_keyfile, &keyfile) {
                    tracing::warn!("Failed to restore keyfile to config_dir: {}", e);
                } else {
                    let _ = std::fs::remove_file(&old_data_keyfile);
                }
            }
        }

        let key_manager = Arc::new(KeyManager::load_or_create(&keyfile)?);

        Ok(Self {
            key_manager,
            config_dir: config_dir.to_path_buf(),
        })
    }

    /// Create with an externally-provided key manager (master password or testing)
    pub fn with_key_manager(config_dir: &Path, key_manager: Arc<KeyManager>) -> Self {
        Self {
            key_manager,
            config_dir: config_dir.to_path_buf(),
        }
    }

    fn secrets_path(&self) -> PathBuf {
        self.config_dir.join("secrets.enc")
    }

    pub(crate) fn load_secrets_unlocked(&self) -> Result<Secrets> {
        let secrets_path = self.secrets_path();
        if !secrets_path.exists() {
            return Ok(Secrets::default());
        }
        let encrypted_data = std::fs::read(&secrets_path)?;
        if encrypted_data.is_empty() {
            tracing::warn!("secrets.enc is empty, returning defaults");
            return Ok(Secrets::default());
        }
        match self.key_manager.decrypt(&encrypted_data) {
            Ok(decrypted) => {
                let secrets: Secrets = serde_json::from_slice(&decrypted)?;
                Ok(secrets)
            }
            Err(e) => {
                let backup_path = self.config_dir.join("secrets.enc.bak");
                if let Err(copy_err) = std::fs::copy(&secrets_path, &backup_path) {
                    tracing::warn!("Failed to back up secrets.enc: {}", copy_err);
                } else {
                    tracing::info!("Backed up secrets.enc to secrets.enc.bak for recovery");
                }
                Err(anyhow!(
                    "Failed to decrypt secrets.enc with the active encryption key: {}",
                    e
                ))
            }
        }
    }

    fn load_secrets_runtime_state_unlocked(&self) -> Result<(Secrets, bool)> {
        match self.load_secrets_unlocked() {
            Ok(secrets) => Ok((secrets, false)),
            Err(e) => {
                tracing::error!(
                    "Failed to decrypt secrets.enc with the active key: {}. \
                     Starting with empty runtime secrets; encrypted data was preserved for recovery.",
                    e
                );
                Ok((Secrets::default(), true))
            }
        }
    }

    pub(crate) fn save_secrets_unlocked(&self, secrets: &Secrets) -> Result<()> {
        let json = serde_json::to_vec(secrets)?;
        let encrypted = self.key_manager.encrypt(&json)?;
        crate::crypto::atomic_write_file(&self.secrets_path(), &encrypted)
    }

    pub(crate) fn with_secrets_lock<T, F>(&self, op: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Result<T>,
    {
        let _guard = secrets_file_lock()
            .lock()
            .map_err(|_| anyhow!("secrets file lock poisoned"))?;
        op(self)
    }

    pub fn update_secrets<T, F>(&self, update: F) -> Result<T>
    where
        F: FnOnce(&mut Secrets) -> Result<T>,
    {
        self.with_secrets_lock(|manager| {
            let (mut secrets, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            if degraded {
                anyhow::bail!(
                    "Refusing to update encrypted secrets because secrets.enc could not be decrypted with the active key. Restore the correct key material before mutating secrets."
                );
            }
            let out = update(&mut secrets)?;
            manager.save_secrets_unlocked(&secrets)?;
            Ok(out)
        })
    }

    fn has_real_secrets(secrets: &Secrets) -> bool {
        secrets.llm_api_key.as_ref().is_some_and(|k| !k.is_empty())
            || secrets
                .embeddings_api_key
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .llm_fallback_api_key
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .telegram_bot_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .whatsapp_access_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .whatsapp_app_secret
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .google_chat_access_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .google_chat_verify_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .signal_bridge_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .imessage_bridge_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .line_channel_access_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .line_channel_secret
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .wechat_bridge_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .qq_bridge_token
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .tunnel_ngrok_authtoken
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets
                .tunnel_tailscale_auth_key
                .as_ref()
                .is_some_and(|k| !k.is_empty())
            || secrets.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || !secrets.media_provider_keys.is_empty()
            || !secrets.model_pool_keys.is_empty()
            || !secrets.mcp_auth.is_empty()
            || !secrets.mcp_env.is_empty()
            || !secrets.custom.is_empty()
    }

    fn infer_legacy_embeddings_config(config: &AgentConfig) -> EmbeddingsConfig {
        let legacy_model = config.memory.embedding_model.trim();
        if legacy_model.is_empty() {
            return EmbeddingsConfig::default();
        }

        let infer_from_provider = |provider: &LlmProvider| match provider {
            LlmProvider::Ollama { base_url, .. } if !base_url.trim().is_empty() => {
                Some(EmbeddingsConfig {
                    provider: EmbeddingsProviderKind::Ollama,
                    model: legacy_model.to_string(),
                    base_url: Some(base_url.trim().trim_end_matches('/').to_string()),
                    api_key: String::new(),
                })
            }
            LlmProvider::OpenAI {
                api_key, base_url, ..
            } => Some(EmbeddingsConfig {
                provider: EmbeddingsProviderKind::OpenaiCompatible,
                model: legacy_model.to_string(),
                base_url: base_url
                    .as_ref()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.trim_end_matches('/').to_string()),
                api_key: api_key.clone(),
            }),
            LlmProvider::Ollama { .. } => None,
            LlmProvider::Anthropic { .. } => None,
        };

        config
            .model_pool
            .slots
            .iter()
            .filter(|slot| slot.enabled)
            .find(|slot| slot.role == ModelRole::Primary)
            .and_then(|slot| infer_from_provider(&slot.provider))
            .or_else(|| {
                config
                    .model_pool
                    .slots
                    .iter()
                    .filter(|slot| slot.enabled)
                    .find_map(|slot| infer_from_provider(&slot.provider))
            })
            .or_else(|| infer_from_provider(&config.llm))
            .unwrap_or_default()
    }

    fn migrate_embeddings_config(config: &mut AgentConfig) {
        if config.embeddings.is_none() {
            config.embeddings = Some(Self::infer_legacy_embeddings_config(config));
        }

        // Clear the legacy field once migrated so future saves only use the dedicated embeddings config.
        if !config.memory.embedding_model.is_empty() {
            config.memory.embedding_model.clear();
        }
    }

    /// Load configuration with decrypted secrets
    pub fn load(&self) -> Result<AgentConfig> {
        let config_path = self.config_dir.join("config.toml");
        let secrets_path = self.secrets_path();

        // Load base config
        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            let mut config = AgentConfig::default();
            // Fresh installs should start with conservative episode retention enabled.
            // Existing installs keep whatever is already persisted in config.toml.
            config.memory.retention_enabled = true;
            self.save_config_only(&config)?;
            config
        };

        // Load and decrypt secrets
        if secrets_path.exists() {
            let (secrets, _changed, created, rotated, persisted_change) =
                self.with_secrets_lock(|manager| {
                    let (mut secrets, degraded) = manager.load_secrets_runtime_state_unlocked()?;
                    let (changed, created, rotated) =
                        Self::ensure_http_api_key_in_secrets(&mut secrets);
                    let persisted_change = changed && !degraded;
                    if persisted_change {
                        manager.save_secrets_unlocked(&secrets)?;
                    }
                    Ok((secrets, changed, created, rotated, persisted_change))
                })?;
            if persisted_change {
                if rotated {
                    tracing::info!("Rotated expired HTTP API key");
                } else if created {
                    tracing::info!("Generated new HTTP API key for authentication");
                }
            }
            // Inject secrets into config
            self.inject_secrets(&mut config, &secrets);
        } else {
            // Migrate from old plain config if secrets exist there
            self.migrate_from_plain_config(&mut config)?;

            // Ensure API key exists even on first run (no secrets.enc yet)
            let (changed, created, rotated) = self.with_secrets_lock(|manager| {
                let mut secrets = manager.load_secrets_unlocked()?;
                let (changed, created, rotated) =
                    Self::ensure_http_api_key_in_secrets(&mut secrets);
                if changed {
                    manager.save_secrets_unlocked(&secrets)?;
                }
                Ok((changed, created, rotated))
            })?;
            if changed {
                if rotated {
                    tracing::info!("Rotated expired HTTP API key");
                } else if created {
                    tracing::info!("Generated HTTP API key for authentication (first run)");
                }
            }
        }

        // Auto-migrate legacy llm/llm_fallback to model_pool if slots is empty.
        // Fresh installs now start with an unconfigured placeholder, so only
        // migrate when the legacy provider contains usable settings.
        if config.model_pool.slots.is_empty() {
            let legacy_llm_configured = match &config.llm {
                LlmProvider::Ollama { base_url, model } => {
                    !base_url.trim().is_empty() && !model.trim().is_empty()
                }
                LlmProvider::Anthropic { api_key, model } => {
                    !api_key.trim().is_empty() && !model.trim().is_empty()
                }
                LlmProvider::OpenAI {
                    api_key,
                    model,
                    base_url,
                } => {
                    !api_key.trim().is_empty()
                        && !model.trim().is_empty()
                        && (base_url.is_none()
                            || base_url.as_ref().is_some_and(|url| !url.trim().is_empty()))
                }
            };
            if legacy_llm_configured || config.llm_fallback.is_some() {
                let primary_slot = ModelSlot {
                    id: "primary".to_string(),
                    label: "Primary".to_string(),
                    role: ModelRole::Primary,
                    provider: config.llm.clone(),
                    enabled: true,
                    capability_tier: ModelCapabilityTier::Balanced,
                    cost_tier: ModelCostTier::Medium,
                    auto_escalate: true,
                    escalation_rank: 0,
                    health_scope: ModelHealthScope::Provider,
                };
                config.model_pool.slots.push(primary_slot);

                if let Some(fallback) = &config.llm_fallback {
                    let fallback_slot = ModelSlot {
                        id: "fallback".to_string(),
                        label: "Fallback".to_string(),
                        role: ModelRole::Fallback,
                        provider: fallback.clone(),
                        enabled: true,
                        capability_tier: ModelCapabilityTier::Premium,
                        cost_tier: ModelCostTier::High,
                        auto_escalate: true,
                        escalation_rank: 100,
                        health_scope: ModelHealthScope::Provider,
                    };
                    config.model_pool.slots.push(fallback_slot);
                }
                tracing::info!(
                    "Migrated legacy llm config to model_pool ({} slots)",
                    config.model_pool.slots.len()
                );
            }
        }

        Self::migrate_embeddings_config(&mut config);

        Ok(config)
    }

    /// Save configuration with encrypted secrets
    pub fn save(&self, config: &AgentConfig) -> Result<()> {
        self.with_secrets_lock(|manager| {
            let (_existing, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            if degraded {
                anyhow::bail!(
                    "Refusing to save configuration while encrypted secrets are unreadable. Restore the correct key material before saving."
                );
            }
            Ok(())
        })?;

        // Create sanitized config (without secrets)
        let sanitized = self.sanitize_config(config);

        // Save non-sensitive config as TOML
        self.save_config_only(&sanitized)?;

        // Guard: don't overwrite existing secrets.enc with empty data.
        // This prevents data loss when decryption fails (key mismatch)
        // but the user saves settings via the web UI.
        self.with_secrets_lock(|manager| {
            let secrets_path = manager.secrets_path();
            let (existing, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            if degraded {
                anyhow::bail!(
                    "Refusing to save encrypted secrets because secrets.enc could not be decrypted with the active key."
                );
            }
            let secrets = manager.extract_secrets_from_base(existing, config);
            if !Self::has_real_secrets(&secrets) && secrets_path.exists() {
                tracing::warn!(
                    "Skipping secrets.enc overwrite: extracted secrets are empty but file exists. \
                 This likely means decryption failed — preserving existing encrypted secrets."
                );
            } else {
                manager.save_secrets_unlocked(&secrets)?;
            }
            Ok(())
        })?;

        Ok(())
    }

    /// Save only the config.toml (without encryption)
    fn save_config_only(&self, config: &AgentConfig) -> Result<()> {
        let config_path = self.config_dir.join("config.toml");
        let content = toml::to_string_pretty(config)?;
        crate::crypto::atomic_write_file(&config_path, content.as_bytes())?;
        Ok(())
    }

    /// Save encrypted secrets
    pub(crate) fn save_secrets(&self, secrets: &Secrets) -> Result<()> {
        self.with_secrets_lock(|manager| manager.save_secrets_unlocked(secrets))
    }

    pub(crate) fn load_secrets(&self) -> Result<Secrets> {
        self.with_secrets_lock(|manager| {
            let (secrets, _degraded) = manager.load_secrets_runtime_state_unlocked()?;
            Ok(secrets)
        })
        /*
        if !secrets_path.exists() {
            return Ok(Secrets::default());
        }
        let encrypted_data = std::fs::read(&secrets_path)?;
        if encrypted_data.is_empty() {
            tracing::warn!("secrets.enc is empty, returning defaults");
            return Ok(Secrets::default());
        }
        match self.key_manager.decrypt(&encrypted_data) {
            Ok(decrypted) => {
                let secrets: Secrets = serde_json::from_slice(&decrypted)?;
                Ok(secrets)
            }
            Err(e) => {
                // NEVER delete secrets.enc — it contains all API keys and OAuth tokens.
                // Back it up so the user can recover if the key is fixed later.
                let backup_path = self.config_dir.join("secrets.enc.bak");
                if let Err(copy_err) = std::fs::copy(&secrets_path, &backup_path) {
                    tracing::warn!("Failed to back up secrets.enc: {}", copy_err);
                } else {
                    tracing::info!("Backed up secrets.enc to secrets.enc.bak for recovery");
                }
                tracing::error!(
                    "Failed to decrypt secrets.enc (key mismatch or corrupt data): {}. \
                     Secrets preserved as secrets.enc.bak — returning empty defaults. \
                     API keys and OAuth tokens will need to be re-entered unless the \
                     correct encryption key is restored.",
                    e
                );
                Ok(Secrets::default())
            }
        }
        */
    }

    /// Atomically update custom secret keys in a single read-modify-write cycle.
    ///
    /// This guards against concurrent lost updates across async handlers that may
    /// mutate the same encrypted `secrets.enc` file.
    pub fn update_custom_secrets<T, F>(&self, update: F) -> Result<T>
    where
        F: FnOnce(&mut std::collections::HashMap<String, String>) -> Result<T>,
    {
        self.update_secrets(|secrets| update(&mut secrets.custom))
    }

    /// Read a custom secret string by key
    pub fn get_custom_secret(&self, key: &str) -> Result<Option<String>> {
        let secrets = self.load_secrets()?;
        Ok(secrets.custom.get(key).cloned())
    }

    /// Write or remove a custom secret string by key
    pub fn set_custom_secret(&self, key: &str, value: Option<String>) -> Result<()> {
        self.update_custom_secrets(|custom| {
            match value {
                Some(v) => {
                    custom.insert(key.to_string(), v);
                }
                None => {
                    custom.remove(key);
                }
            }
            Ok(())
        })
    }

    /// Extract secrets from config
    fn extract_secrets_from_base(&self, mut secrets: Secrets, config: &AgentConfig) -> Secrets {
        // Model-related secrets are derived entirely from the current config/runtime state.
        // Clear them first so deleted slots and removed legacy keys do not persist forever.
        secrets.llm_api_key = None;
        secrets.embeddings_api_key = None;
        secrets.llm_fallback_api_key = None;
        secrets.model_pool_keys.clear();

        // Extract primary LLM API key
        match &config.llm {
            LlmProvider::Anthropic { api_key, .. }
                if !api_key.is_empty() && api_key != "[ENCRYPTED]" =>
            {
                secrets.llm_api_key = Some(api_key.clone());
            }
            LlmProvider::OpenAI { api_key, .. }
                if !api_key.is_empty() && api_key != "[ENCRYPTED]" =>
            {
                secrets.llm_api_key = Some(api_key.clone());
            }
            _ => {}
        }

        if let Some(embeddings) = &config.embeddings {
            if matches!(
                embeddings.provider,
                EmbeddingsProviderKind::OpenaiCompatible
            ) && !embeddings.api_key.is_empty()
                && embeddings.api_key != "[ENCRYPTED]"
            {
                secrets.embeddings_api_key = Some(embeddings.api_key.clone());
            }
        }

        // Extract fallback LLM API key
        if let Some(fallback) = &config.llm_fallback {
            match fallback {
                LlmProvider::Anthropic { api_key, .. }
                    if !api_key.is_empty() && api_key != "[ENCRYPTED]" =>
                {
                    secrets.llm_fallback_api_key = Some(api_key.clone());
                }
                LlmProvider::OpenAI { api_key, .. }
                    if !api_key.is_empty() && api_key != "[ENCRYPTED]" =>
                {
                    secrets.llm_fallback_api_key = Some(api_key.clone());
                }
                _ => {}
            }
        }

        // Extract Telegram token
        if let Some(tg) = &config.telegram {
            if !tg.bot_token.is_empty() && tg.bot_token != "[ENCRYPTED]" {
                secrets.telegram_bot_token = Some(tg.bot_token.clone());
            }
        }

        // Extract Slack secrets
        if let Some(slack) = &config.slack {
            if !slack.bot_token.is_empty() && slack.bot_token != "[ENCRYPTED]" {
                secrets.slack_bot_token = Some(slack.bot_token.clone());
            }
            if !slack.signing_secret.is_empty() && slack.signing_secret != "[ENCRYPTED]" {
                secrets.slack_signing_secret = Some(slack.signing_secret.clone());
            }
        }

        // Extract Discord secrets
        if let Some(discord) = &config.discord {
            if !discord.bot_token.is_empty() && discord.bot_token != "[ENCRYPTED]" {
                secrets.discord_bot_token = Some(discord.bot_token.clone());
            }
        }

        // Extract Matrix access token
        if let Some(matrix) = &config.matrix {
            if !matrix.access_token.is_empty() && matrix.access_token != "[ENCRYPTED]" {
                secrets.matrix_access_token = Some(matrix.access_token.clone());
            }
        }

        // Extract Teams access token
        if let Some(teams) = &config.teams {
            if !teams.access_token.is_empty() && teams.access_token != "[ENCRYPTED]" {
                secrets.teams_access_token = Some(teams.access_token.clone());
            }
        }

        // Extract WhatsApp access token
        if let Some(wa) = &config.whatsapp {
            if !wa.access_token.is_empty() && wa.access_token != "[ENCRYPTED]" {
                secrets.whatsapp_access_token = Some(wa.access_token.clone());
            }
            if !wa.app_secret.is_empty() && wa.app_secret != "[ENCRYPTED]" {
                secrets.whatsapp_app_secret = Some(wa.app_secret.clone());
            }
            if !wa.bridge_token.is_empty() && wa.bridge_token != "[ENCRYPTED]" {
                secrets.whatsapp_bridge_token = Some(wa.bridge_token.clone());
            }
        }
        if let Some(google_chat) = &config.google_chat {
            if !google_chat.access_token.is_empty() && google_chat.access_token != "[ENCRYPTED]" {
                secrets.google_chat_access_token = Some(google_chat.access_token.clone());
            }
            if !google_chat.verify_token.is_empty() && google_chat.verify_token != "[ENCRYPTED]" {
                secrets.google_chat_verify_token = Some(google_chat.verify_token.clone());
            }
        }
        if let Some(signal) = &config.signal {
            if !signal.bridge_token.is_empty() && signal.bridge_token != "[ENCRYPTED]" {
                secrets.signal_bridge_token = Some(signal.bridge_token.clone());
            }
        }
        if let Some(imessage) = &config.imessage {
            if !imessage.bridge_token.is_empty() && imessage.bridge_token != "[ENCRYPTED]" {
                secrets.imessage_bridge_token = Some(imessage.bridge_token.clone());
            }
        }
        if let Some(line) = &config.line {
            if !line.channel_access_token.is_empty() && line.channel_access_token != "[ENCRYPTED]" {
                secrets.line_channel_access_token = Some(line.channel_access_token.clone());
            }
            if !line.channel_secret.is_empty() && line.channel_secret != "[ENCRYPTED]" {
                secrets.line_channel_secret = Some(line.channel_secret.clone());
            }
        }
        if let Some(wechat) = &config.wechat {
            if !wechat.bridge_token.is_empty() && wechat.bridge_token != "[ENCRYPTED]" {
                secrets.wechat_bridge_token = Some(wechat.bridge_token.clone());
            }
        }
        if let Some(qq) = &config.qq {
            if !qq.bridge_token.is_empty() && qq.bridge_token != "[ENCRYPTED]" {
                secrets.qq_bridge_token = Some(qq.bridge_token.clone());
            }
        }

        if !config.tunnel.ngrok.authtoken.is_empty()
            && config.tunnel.ngrok.authtoken != "[ENCRYPTED]"
        {
            secrets.tunnel_ngrok_authtoken = Some(config.tunnel.ngrok.authtoken.clone());
        }
        if !config.tunnel.tailscale_funnel.auth_key.is_empty()
            && config.tunnel.tailscale_funnel.auth_key != "[ENCRYPTED]"
        {
            secrets.tunnel_tailscale_auth_key =
                Some(config.tunnel.tailscale_funnel.auth_key.clone());
        }

        // Extract media provider API keys
        for (provider, key) in &config.media_gen.provider_api_keys {
            if !key.is_empty() && key != "[ENCRYPTED]" {
                secrets
                    .media_provider_keys
                    .insert(provider.clone(), key.clone());
            }
        }

        // Extract model pool API keys
        for slot in &config.model_pool.slots {
            match &slot.provider {
                LlmProvider::Anthropic { api_key, .. }
                    if !api_key.is_empty() && api_key != "[ENCRYPTED]" =>
                {
                    secrets
                        .model_pool_keys
                        .insert(slot.id.clone(), api_key.clone());
                }
                LlmProvider::OpenAI { api_key, .. }
                    if !api_key.is_empty() && api_key != "[ENCRYPTED]" =>
                {
                    secrets
                        .model_pool_keys
                        .insert(slot.id.clone(), api_key.clone());
                }
                _ => {}
            }
        }

        secrets
    }

    /// Create sanitized config with placeholder secrets
    fn sanitize_config(&self, config: &AgentConfig) -> AgentConfig {
        let mut sanitized = config.clone();

        // Replace primary API key with placeholder
        match &mut sanitized.llm {
            LlmProvider::Anthropic { api_key, .. } => {
                if !api_key.is_empty() {
                    *api_key = "[ENCRYPTED]".to_string();
                }
            }
            LlmProvider::OpenAI { api_key, .. } => {
                if !api_key.is_empty() {
                    *api_key = "[ENCRYPTED]".to_string();
                }
            }
            _ => {}
        }

        if let Some(embeddings) = &mut sanitized.embeddings {
            if matches!(
                embeddings.provider,
                EmbeddingsProviderKind::OpenaiCompatible
            ) && !embeddings.api_key.is_empty()
            {
                embeddings.api_key = "[ENCRYPTED]".to_string();
            }
        }

        // Replace fallback API key with placeholder
        if let Some(fallback) = &mut sanitized.llm_fallback {
            match fallback {
                LlmProvider::Anthropic { api_key, .. } => {
                    if !api_key.is_empty() {
                        *api_key = "[ENCRYPTED]".to_string();
                    }
                }
                LlmProvider::OpenAI { api_key, .. } => {
                    if !api_key.is_empty() {
                        *api_key = "[ENCRYPTED]".to_string();
                    }
                }
                _ => {}
            }
        }

        // Replace Telegram token with placeholder
        if let Some(tg) = &mut sanitized.telegram {
            if !tg.bot_token.is_empty() {
                tg.bot_token = "[ENCRYPTED]".to_string();
            }
        }

        // Replace Slack secrets with placeholders
        if let Some(slack) = &mut sanitized.slack {
            if !slack.bot_token.is_empty() {
                slack.bot_token = "[ENCRYPTED]".to_string();
            }
            if !slack.signing_secret.is_empty() {
                slack.signing_secret = "[ENCRYPTED]".to_string();
            }
        }

        // Replace Discord secrets with placeholders
        if let Some(discord) = &mut sanitized.discord {
            if !discord.bot_token.is_empty() {
                discord.bot_token = "[ENCRYPTED]".to_string();
            }
        }

        // Replace Matrix access token with placeholder
        if let Some(matrix) = &mut sanitized.matrix {
            if !matrix.access_token.is_empty() {
                matrix.access_token = "[ENCRYPTED]".to_string();
            }
        }

        // Replace Teams access token with placeholder
        if let Some(teams) = &mut sanitized.teams {
            if !teams.access_token.is_empty() {
                teams.access_token = "[ENCRYPTED]".to_string();
            }
        }

        // Replace WhatsApp access token with placeholder
        if let Some(wa) = &mut sanitized.whatsapp {
            if !wa.access_token.is_empty() {
                wa.access_token = "[ENCRYPTED]".to_string();
            }
            if !wa.app_secret.is_empty() {
                wa.app_secret = "[ENCRYPTED]".to_string();
            }
            if !wa.bridge_token.is_empty() {
                wa.bridge_token = "[ENCRYPTED]".to_string();
            }
        }
        if let Some(google_chat) = &mut sanitized.google_chat {
            if !google_chat.access_token.is_empty() {
                google_chat.access_token = "[ENCRYPTED]".to_string();
            }
            if !google_chat.verify_token.is_empty() {
                google_chat.verify_token = "[ENCRYPTED]".to_string();
            }
        }
        if let Some(signal) = &mut sanitized.signal {
            if !signal.bridge_token.is_empty() {
                signal.bridge_token = "[ENCRYPTED]".to_string();
            }
        }
        if let Some(imessage) = &mut sanitized.imessage {
            if !imessage.bridge_token.is_empty() {
                imessage.bridge_token = "[ENCRYPTED]".to_string();
            }
        }
        if let Some(line) = &mut sanitized.line {
            if !line.channel_access_token.is_empty() {
                line.channel_access_token = "[ENCRYPTED]".to_string();
            }
            if !line.channel_secret.is_empty() {
                line.channel_secret = "[ENCRYPTED]".to_string();
            }
        }
        if let Some(wechat) = &mut sanitized.wechat {
            if !wechat.bridge_token.is_empty() {
                wechat.bridge_token = "[ENCRYPTED]".to_string();
            }
        }
        if let Some(qq) = &mut sanitized.qq {
            if !qq.bridge_token.is_empty() {
                qq.bridge_token = "[ENCRYPTED]".to_string();
            }
        }

        if !sanitized.tunnel.ngrok.authtoken.is_empty() {
            sanitized.tunnel.ngrok.authtoken = "[ENCRYPTED]".to_string();
        }
        if !sanitized.tunnel.tailscale_funnel.auth_key.is_empty() {
            sanitized.tunnel.tailscale_funnel.auth_key = "[ENCRYPTED]".to_string();
        }

        // Replace media provider API keys with placeholder
        for (_, key) in sanitized.media_gen.provider_api_keys.iter_mut() {
            if !key.is_empty() {
                *key = "[ENCRYPTED]".to_string();
            }
        }

        // Replace model pool API keys with placeholder
        for slot in &mut sanitized.model_pool.slots {
            match &mut slot.provider {
                LlmProvider::Anthropic { api_key, .. } => {
                    if !api_key.is_empty() {
                        *api_key = "[ENCRYPTED]".to_string();
                    }
                }
                LlmProvider::OpenAI { api_key, .. } => {
                    if !api_key.is_empty() {
                        *api_key = "[ENCRYPTED]".to_string();
                    }
                }
                _ => {}
            }
        }

        sanitized
    }

    /// Inject decrypted secrets into config
    fn inject_secrets(&self, config: &mut AgentConfig, secrets: &Secrets) {
        // Inject primary LLM API key (skip placeholder values)
        if let Some(api_key) = &secrets.llm_api_key {
            if !api_key.is_empty() && api_key != "[ENCRYPTED]" {
                match &mut config.llm {
                    LlmProvider::Anthropic { api_key: key, .. } => {
                        *key = api_key.clone();
                    }
                    LlmProvider::OpenAI { api_key: key, .. } => {
                        *key = api_key.clone();
                    }
                    _ => {}
                }
            }
        }

        if let Some(api_key) = &secrets.embeddings_api_key {
            if !api_key.is_empty() && api_key != "[ENCRYPTED]" {
                if let Some(embeddings) = &mut config.embeddings {
                    if matches!(
                        embeddings.provider,
                        EmbeddingsProviderKind::OpenaiCompatible
                    ) {
                        embeddings.api_key = api_key.clone();
                    }
                }
            }
        }

        // Inject fallback LLM API key (skip placeholder values)
        if let Some(api_key) = &secrets.llm_fallback_api_key {
            if !api_key.is_empty() && api_key != "[ENCRYPTED]" {
                if let Some(fallback) = &mut config.llm_fallback {
                    match fallback {
                        LlmProvider::Anthropic { api_key: key, .. } => {
                            *key = api_key.clone();
                        }
                        LlmProvider::OpenAI { api_key: key, .. } => {
                            *key = api_key.clone();
                        }
                        _ => {}
                    }
                }
            }
        }

        // Inject Telegram token
        if let Some(token) = &secrets.telegram_bot_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(tg) = &mut config.telegram {
                    tg.bot_token = token.clone();
                } else {
                    // Secret exists but config section was lost (e.g. volume reset) — recreate it
                    tracing::info!("Recovered Telegram config from encrypted secrets (config.toml was missing [telegram] section)");
                    config.telegram = Some(TelegramConfig {
                        bot_token: token.clone(),
                        allowed_users: vec![],
                        dm_policy: "pairing".to_string(),
                    });
                }
            }
        }

        // Inject Slack secrets
        if let Some(token) = &secrets.slack_bot_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(slack) = &mut config.slack {
                    slack.bot_token = token.clone();
                } else {
                    tracing::info!(
                        "Recovered Slack config from encrypted secrets (config.toml was missing [slack] section)"
                    );
                    config.slack = Some(SlackChannelConfig {
                        bot_token: token.clone(),
                        signing_secret: secrets.slack_signing_secret.clone().unwrap_or_default(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(secret) = &secrets.slack_signing_secret {
            if !secret.is_empty() && secret != "[ENCRYPTED]" {
                if let Some(slack) = &mut config.slack {
                    slack.signing_secret = secret.clone();
                } else {
                    tracing::info!(
                        "Recovered Slack config from encrypted secrets (config.toml was missing [slack] section)"
                    );
                    config.slack = Some(SlackChannelConfig {
                        bot_token: secrets.slack_bot_token.clone().unwrap_or_default(),
                        signing_secret: secret.clone(),
                        ..Default::default()
                    });
                }
            }
        }

        // Inject Discord secrets
        if let Some(token) = &secrets.discord_bot_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(discord) = &mut config.discord {
                    discord.bot_token = token.clone();
                } else {
                    tracing::info!(
                        "Recovered Discord config from encrypted secrets (config.toml was missing [discord] section)"
                    );
                    config.discord = Some(DiscordChannelConfig {
                        bot_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }

        // Inject Matrix access token
        if let Some(token) = &secrets.matrix_access_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(matrix) = &mut config.matrix {
                    matrix.access_token = token.clone();
                } else {
                    tracing::info!(
                        "Recovered Matrix config from encrypted secrets (config.toml was missing [matrix] section)"
                    );
                    config.matrix = Some(MatrixTransportConfig {
                        access_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }

        // Inject Teams access token
        if let Some(token) = &secrets.teams_access_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(teams) = &mut config.teams {
                    teams.access_token = token.clone();
                } else {
                    tracing::info!(
                        "Recovered Teams config from encrypted secrets (config.toml was missing [teams] section)"
                    );
                    config.teams = Some(TeamsTransportConfig {
                        access_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }

        // Inject WhatsApp access token
        if let Some(token) = &secrets.whatsapp_access_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(wa) = &mut config.whatsapp {
                    wa.access_token = token.clone();
                } else {
                    // Secret exists but config section was lost (e.g. volume reset) — recreate it
                    tracing::info!("Recovered WhatsApp config from encrypted secrets");
                    config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
                        mode: Default::default(),
                        access_token: token.clone(),
                        app_secret: secrets.whatsapp_app_secret.clone().unwrap_or_default(),
                        phone_number_id: String::new(),
                        verify_token: "agentark_verify".to_string(),
                        bridge_runtime: Some(
                            crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded,
                        ),
                        bridge_url: crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string(),
                        bridge_token: secrets.whatsapp_bridge_token.clone().unwrap_or_default(),
                        allowed_numbers: vec![],
                        dm_policy: "pairing".to_string(),
                    });
                }
            }
        }
        if let Some(secret) = &secrets.whatsapp_app_secret {
            if !secret.is_empty() && secret != "[ENCRYPTED]" {
                if let Some(wa) = &mut config.whatsapp {
                    wa.app_secret = secret.clone();
                } else {
                    tracing::info!("Recovered WhatsApp config from encrypted secrets");
                    config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
                        mode: Default::default(),
                        access_token: secrets.whatsapp_access_token.clone().unwrap_or_default(),
                        app_secret: secret.clone(),
                        phone_number_id: String::new(),
                        verify_token: "agentark_verify".to_string(),
                        bridge_runtime: Some(
                            crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded,
                        ),
                        bridge_url: crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string(),
                        bridge_token: secrets.whatsapp_bridge_token.clone().unwrap_or_default(),
                        allowed_numbers: vec![],
                        dm_policy: "pairing".to_string(),
                    });
                }
            }
        }
        if let Some(token) = &secrets.whatsapp_bridge_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(wa) = &mut config.whatsapp {
                    wa.bridge_token = token.clone();
                } else {
                    tracing::info!("Recovered WhatsApp bridge token from encrypted secrets");
                    config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
                        mode: Default::default(),
                        access_token: secrets.whatsapp_access_token.clone().unwrap_or_default(),
                        app_secret: secrets.whatsapp_app_secret.clone().unwrap_or_default(),
                        phone_number_id: String::new(),
                        verify_token: "agentark_verify".to_string(),
                        bridge_runtime: Some(
                            crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded,
                        ),
                        bridge_url: crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string(),
                        bridge_token: token.clone(),
                        allowed_numbers: vec![],
                        dm_policy: "pairing".to_string(),
                    });
                }
            }
        }
        if let Some(token) = &secrets.google_chat_access_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(google_chat) = &mut config.google_chat {
                    google_chat.access_token = token.clone();
                } else {
                    config.google_chat = Some(GoogleChatChannelConfig {
                        access_token: token.clone(),
                        verify_token: secrets.google_chat_verify_token.clone().unwrap_or_default(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.google_chat_verify_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(google_chat) = &mut config.google_chat {
                    google_chat.verify_token = token.clone();
                } else {
                    config.google_chat = Some(GoogleChatChannelConfig {
                        access_token: secrets.google_chat_access_token.clone().unwrap_or_default(),
                        verify_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.signal_bridge_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(signal) = &mut config.signal {
                    signal.bridge_token = token.clone();
                } else {
                    config.signal = Some(SignalChannelConfig {
                        bridge_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.imessage_bridge_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(imessage) = &mut config.imessage {
                    imessage.bridge_token = token.clone();
                } else {
                    config.imessage = Some(IMessageChannelConfig {
                        bridge_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.line_channel_access_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(line) = &mut config.line {
                    line.channel_access_token = token.clone();
                } else {
                    config.line = Some(LineChannelConfig {
                        channel_access_token: token.clone(),
                        channel_secret: secrets.line_channel_secret.clone().unwrap_or_default(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.line_channel_secret {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(line) = &mut config.line {
                    line.channel_secret = token.clone();
                } else {
                    config.line = Some(LineChannelConfig {
                        channel_access_token: secrets
                            .line_channel_access_token
                            .clone()
                            .unwrap_or_default(),
                        channel_secret: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.wechat_bridge_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(wechat) = &mut config.wechat {
                    wechat.bridge_token = token.clone();
                } else {
                    config.wechat = Some(WeChatChannelConfig {
                        bridge_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        if let Some(token) = &secrets.qq_bridge_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(qq) = &mut config.qq {
                    qq.bridge_token = token.clone();
                } else {
                    config.qq = Some(QqChannelConfig {
                        bridge_token: token.clone(),
                        ..Default::default()
                    });
                }
            }
        }

        if let Some(token) = &secrets.tunnel_ngrok_authtoken {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                config.tunnel.ngrok.authtoken = token.clone();
            }
        }
        if let Some(token) = &secrets.tunnel_tailscale_auth_key {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                config.tunnel.tailscale_funnel.auth_key = token.clone();
            }
        }

        // Inject media provider API keys (skip placeholder values)
        for (provider, key) in &secrets.media_provider_keys {
            if !key.is_empty() && key != "[ENCRYPTED]" {
                config
                    .media_gen
                    .provider_api_keys
                    .insert(provider.clone(), key.clone());
            }
        }

        // Inject model pool API keys (skip placeholder values)
        for slot in &mut config.model_pool.slots {
            if let Some(api_key) = secrets.model_pool_keys.get(&slot.id) {
                if !api_key.is_empty() && api_key != "[ENCRYPTED]" {
                    match &mut slot.provider {
                        LlmProvider::Anthropic { api_key: key, .. } => {
                            *key = api_key.clone();
                        }
                        LlmProvider::OpenAI { api_key: key, .. } => {
                            *key = api_key.clone();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Migrate secrets from old plain config to encrypted storage
    fn migrate_from_plain_config(&self, config: &mut AgentConfig) -> Result<()> {
        let secrets = self.extract_secrets_from_base(Secrets::default(), config);

        // Only migrate if there are actual secrets
        let has_secrets = secrets
            .llm_api_key
            .as_ref()
            .map(|k| !k.is_empty() && k != "[ENCRYPTED]")
            .unwrap_or(false)
            || secrets
                .embeddings_api_key
                .as_ref()
                .map(|k| !k.is_empty() && k != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .llm_fallback_api_key
                .as_ref()
                .map(|k| !k.is_empty() && k != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .telegram_bot_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .whatsapp_access_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .whatsapp_app_secret
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .whatsapp_bridge_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .google_chat_access_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .google_chat_verify_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .signal_bridge_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .imessage_bridge_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .line_channel_access_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .line_channel_secret
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .wechat_bridge_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .qq_bridge_token
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .tunnel_ngrok_authtoken
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || secrets
                .tunnel_tailscale_auth_key
                .as_ref()
                .map(|t| !t.is_empty() && t != "[ENCRYPTED]")
                .unwrap_or(false)
            || !secrets.media_provider_keys.is_empty();

        if has_secrets {
            tracing::info!("Migrating secrets to encrypted storage...");
            self.save_secrets(&secrets)?;

            // Save sanitized config
            let sanitized = self.sanitize_config(config);
            self.save_config_only(&sanitized)?;

            tracing::info!("Secrets migration complete");
        }

        Ok(())
    }

    /// Get the HTTP API key (for middleware validation)
    pub fn get_api_key(&self) -> Result<Option<String>> {
        Ok(self.get_api_key_info()?.map(|info| info.key))
    }

    /// Get the HTTP API key with TTL metadata. This enforces rotation for expired keys.
    pub fn get_api_key_info(&self) -> Result<Option<HttpApiKeyInfo>> {
        let (info, _) = self.ensure_api_key_info()?;
        Ok(info)
    }

    /// Ensure API key exists and rotate if expired.
    /// Returns the current key info and whether rotation happened in this call.
    pub fn ensure_api_key_info(&self) -> Result<(Option<HttpApiKeyInfo>, bool)> {
        self.with_secrets_lock(|manager| {
            let (mut secrets, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            let (changed, _created, rotated) = Self::ensure_http_api_key_in_secrets(&mut secrets);
            if changed && !degraded {
                manager.save_secrets_unlocked(&secrets)?;
            }
            Ok((Self::api_key_info_from_secrets(&secrets), rotated))
        })
    }

    /// Regenerate the HTTP API key and return key + TTL metadata
    pub fn regenerate_api_key_info(&self) -> Result<HttpApiKeyInfo> {
        let info = self.update_secrets(|secrets| {
            let now = Self::current_unix_ts();
            let api_key = Self::generate_http_api_key();
            secrets.api_key = Some(api_key);
            secrets.api_key_issued_at = Some(now);
            secrets.api_key_expires_at = Some(now + HTTP_API_KEY_TTL_SECS);
            Self::api_key_info_from_secrets(secrets)
                .ok_or_else(|| anyhow!("Failed to build regenerated API key metadata"))
        })?;
        tracing::info!(
            "HTTP API key regenerated (expires in {} seconds)",
            HTTP_API_KEY_TTL_SECS
        );
        Ok(info)
    }

    fn current_unix_ts() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn generate_http_api_key() -> String {
        use rand::RngCore;
        let mut key_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key_bytes);
        base64::engine::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, key_bytes)
    }

    fn api_key_info_from_secrets(secrets: &Secrets) -> Option<HttpApiKeyInfo> {
        let key = secrets.api_key.clone()?;
        if key.trim().is_empty() {
            return None;
        }
        let issued_at = secrets.api_key_issued_at.unwrap_or(0);
        let expires_at = secrets.api_key_expires_at.unwrap_or(0);
        if issued_at <= 0 || expires_at <= 0 {
            return None;
        }
        Some(HttpApiKeyInfo {
            key,
            issued_at,
            expires_at,
        })
    }

    fn ensure_http_api_key_in_secrets(secrets: &mut Secrets) -> (bool, bool, bool) {
        let mut changed = false;
        let created = false;
        let mut rotated = false;
        let now = Self::current_unix_ts();

        let key_missing = secrets
            .api_key
            .as_ref()
            .map(|k| k.trim().is_empty())
            .unwrap_or(true);
        if key_missing {
            secrets.api_key = Some(Self::generate_http_api_key());
            secrets.api_key_issued_at = Some(now);
            secrets.api_key_expires_at = Some(now + HTTP_API_KEY_TTL_SECS);
            return (true, true, false);
        }

        let mut issued_at = secrets.api_key_issued_at.unwrap_or(0);
        let mut expires_at = secrets.api_key_expires_at.unwrap_or(0);

        if issued_at <= 0 {
            issued_at = now;
            changed = true;
        }
        if expires_at <= issued_at {
            expires_at = issued_at + HTTP_API_KEY_TTL_SECS;
            changed = true;
        }

        if expires_at <= now {
            secrets.api_key = Some(Self::generate_http_api_key());
            issued_at = now;
            expires_at = now + HTTP_API_KEY_TTL_SECS;
            rotated = true;
            changed = true;
        }

        if secrets.api_key_issued_at != Some(issued_at) {
            secrets.api_key_issued_at = Some(issued_at);
            changed = true;
        }
        if secrets.api_key_expires_at != Some(expires_at) {
            secrets.api_key_expires_at = Some(expires_at);
            changed = true;
        }

        (changed, created, rotated)
    }
}

impl AgentConfig {
    /// Validate and sanitize auto_approve list, removing blocked actions
    pub fn validate_auto_approve(list: &[String]) -> (Vec<String>, Vec<String>) {
        let mut allowed = Vec::new();
        let mut rejected = Vec::new();
        let mut seen_allowed = std::collections::HashSet::new();
        let mut seen_rejected = std::collections::HashSet::new();
        for action in list {
            let trimmed = action.trim();
            if trimmed.is_empty() {
                continue;
            }
            if AUTO_APPROVE_BLOCKED.contains(&trimmed) {
                tracing::warn!(
                    "Cannot auto-approve '{}': action is in the blocked list",
                    trimmed
                );
                if seen_rejected.insert(trimmed.to_string()) {
                    rejected.push(trimmed.to_string());
                }
            } else if seen_allowed.insert(trimmed.to_string()) {
                allowed.push(trimmed.to_string());
            }
        }
        (allowed, rejected)
    }

    /// Save config with encrypted secrets.
    /// `data_dir` is the persistent data directory where the keyfile lives.
    pub fn save(&self, config_dir: &Path, data_dir: Option<&Path>) -> Result<()> {
        // Always use encryption for saving - don't silently fall back to plain
        let manager = SecureConfigManager::new_with_data_dir(config_dir, data_dir)
            .map_err(|e| anyhow!("Failed to initialize encryption for saving config: {}", e))?;
        manager.save(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
}

fn default_dm_policy() -> String {
    "pairing".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_mode")]
    pub default_mode: String,
    #[serde(default = "default_docker_image")]
    pub docker_image: String,
    #[serde(default = "default_true")]
    pub enable_rollback: bool,
    pub snapshot_dir: Option<String>,
}

fn default_sandbox_mode() -> String {
    "wasm".to_string()
}

fn default_docker_image() -> String {
    runtime_image::default_runtime_image()
}

fn default_true() -> bool {
    true
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            default_mode: default_sandbox_mode(),
            docker_image: default_docker_image(),
            enable_rollback: true,
            snapshot_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_max_episodes")]
    pub max_episodes: usize,
    #[serde(default = "default_consolidation_interval")]
    pub consolidation_interval_hours: u64,
    #[serde(default)]
    pub embedding_model: String,
    /// Optional retention pruning for episodic episodes (not semantic facts).
    /// Fresh installs enable it conservatively; legacy configs without this field deserialize as false.
    #[serde(default = "default_false")]
    pub retention_enabled: bool,
    /// Minimum age (days) before an episode is eligible for pruning.
    #[serde(default = "default_retention_min_age_days")]
    pub retention_min_age_days: u64,
    /// Always keep at least the N newest episodes (regardless of age).
    #[serde(default = "default_retention_keep_last")]
    pub retention_keep_last: usize,
    /// Only prune episodes with importance <= this value (0.0-1.0).
    #[serde(default = "default_retention_max_importance")]
    pub retention_max_importance: f32,
    /// Only prune episodes with access_count <= this value.
    #[serde(default = "default_retention_max_access_count")]
    pub retention_max_access_count: i32,
    /// Require consolidated=true for pruning (strongly recommended).
    #[serde(default = "default_true")]
    pub retention_require_consolidated: bool,
    /// Minimum days between retention runs.
    #[serde(default = "default_retention_run_interval_days")]
    pub retention_run_interval_days: u64,
    /// Only run retention if no user activity in the last N seconds.
    #[serde(default = "default_retention_idle_threshold_secs")]
    pub retention_idle_threshold_secs: u64,
    /// Maximum number of episodes to delete per run (rate limiter).
    #[serde(default = "default_retention_max_delete_per_run")]
    pub retention_max_delete_per_run: u64,
    /// Protect episodes referenced in semantic fact sources.
    #[serde(default = "default_true")]
    pub retention_protect_fact_sources: bool,
}

fn default_max_episodes() -> usize {
    10000
}

fn default_consolidation_interval() -> u64 {
    24
}

fn default_false() -> bool {
    false
}

fn default_retention_min_age_days() -> u64 {
    180
}

fn default_retention_keep_last() -> usize {
    2500
}

fn default_retention_max_importance() -> f32 {
    0.6
}

fn default_retention_max_access_count() -> i32 {
    1
}

fn default_retention_run_interval_days() -> u64 {
    7
}

fn default_retention_idle_threshold_secs() -> u64 {
    600
}

fn default_retention_max_delete_per_run() -> u64 {
    500
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_episodes: default_max_episodes(),
            consolidation_interval_hours: default_consolidation_interval(),
            embedding_model: String::new(),
            retention_enabled: default_false(),
            retention_min_age_days: default_retention_min_age_days(),
            retention_keep_last: default_retention_keep_last(),
            retention_max_importance: default_retention_max_importance(),
            retention_max_access_count: default_retention_max_access_count(),
            retention_require_consolidated: default_true(),
            retention_run_interval_days: default_retention_run_interval_days(),
            retention_idle_threshold_secs: default_retention_idle_threshold_secs(),
            retention_max_delete_per_run: default_retention_max_delete_per_run(),
            retention_protect_fact_sources: default_true(),
        }
    }
}
