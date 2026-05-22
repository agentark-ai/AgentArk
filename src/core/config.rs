//! Agent configuration with encryption for sensitive data.
//!
//! Mutable runtime settings are stored in encrypted Postgres KV records when
//! storage is available. Only bootstrap metadata needed before the database is
//! unlocked stays on disk. This module participates in the user-owned side of
//! the data contract: `/app/config/bootstrap.toml` and encrypted `settings:*`
//! KV records must survive release updates.

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
use crate::security::ModelPrivacyConfig;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Builder;

/// Global key manager set at startup (from master password or keyfile).
/// All SecureConfigManager instances use this when available, ensuring
/// consistent encryption across the entire process after password changes.
static GLOBAL_KEY_MANAGER: std::sync::OnceLock<std::sync::RwLock<Option<Arc<KeyManager>>>> =
    std::sync::OnceLock::new();
static GLOBAL_SETTINGS_STORAGE: std::sync::OnceLock<
    std::sync::RwLock<Option<crate::storage::Storage>>,
> = std::sync::OnceLock::new();
static SECRETS_FILE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
pub const HTTP_API_KEY_TTL_SECS: i64 = 24 * 60 * 60;
pub(crate) const SETTINGS_BOOTSTRAP_FILE: &str = "bootstrap.toml";
pub(crate) const SETTINGS_CONFIG_KEY: &str = "settings:agent_config:v2";
pub(crate) const SETTINGS_SECRETS_KEY: &str = "settings:secrets:v2";
pub(crate) const SETTINGS_SEARCH_KEY: &str = "settings:search:v1";
pub(crate) const SETTINGS_RUNTIME_KEY: &str = "settings:runtime:v1";
pub(crate) const SETTINGS_DISABLED_ACTIONS_KEY: &str = "settings:runtime:disabled_actions:v1";
pub(crate) const SETTINGS_ACTION_REVIEWS_KEY: &str = "settings:runtime:action_reviews:v1";
pub(crate) const SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY: &str =
    "settings:runtime:removed_bundled_actions:v1";
pub(crate) const SETTINGS_APPROVED_PERMISSIONS_KEY: &str =
    "settings:security:approved_permissions:v1";
pub(crate) const SETTINGS_KEY_LINEAGE_KEY: &str = "settings:security:key_lineage:v1";
pub(crate) const SETTINGS_ENCRYPTED_KV_KEYS: &[&str] = &[
    SETTINGS_CONFIG_KEY,
    SETTINGS_SECRETS_KEY,
    SETTINGS_SEARCH_KEY,
    SETTINGS_RUNTIME_KEY,
    SETTINGS_DISABLED_ACTIONS_KEY,
    SETTINGS_ACTION_REVIEWS_KEY,
    SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
    SETTINGS_APPROVED_PERMISSIONS_KEY,
];

fn global_key_manager_cell() -> &'static std::sync::RwLock<Option<Arc<KeyManager>>> {
    GLOBAL_KEY_MANAGER.get_or_init(|| std::sync::RwLock::new(None))
}

fn global_settings_storage_cell() -> &'static std::sync::RwLock<Option<crate::storage::Storage>> {
    GLOBAL_SETTINGS_STORAGE.get_or_init(|| std::sync::RwLock::new(None))
}

fn secrets_file_lock() -> &'static std::sync::Mutex<()> {
    SECRETS_FILE_LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn run_blocking_config_section<T, F>(op: F) -> T
where
    F: FnOnce() -> T,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if matches!(
            handle.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        ) {
            return tokio::task::block_in_place(op);
        }
    }
    op()
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

pub fn set_global_settings_storage(storage: crate::storage::Storage) {
    if let Ok(mut guard) = global_settings_storage_cell().write() {
        *guard = Some(storage);
    }
}

pub fn global_settings_storage() -> Option<crate::storage::Storage> {
    global_settings_storage_cell()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BootstrapMetadata {
    #[serde(default)]
    deployment_mode: DeploymentMode,
}

impl Default for BootstrapMetadata {
    fn default() -> Self {
        Self {
            deployment_mode: DeploymentMode::default(),
        }
    }
}

pub fn bootstrap_metadata_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_BOOTSTRAP_FILE)
}

pub fn bootstrap_metadata_exists(config_dir: &Path) -> bool {
    bootstrap_metadata_path(config_dir).exists()
}

pub fn load_bootstrap_deployment_mode(config_dir: &Path) -> DeploymentMode {
    std::fs::read_to_string(bootstrap_metadata_path(config_dir))
        .ok()
        .and_then(|raw| toml::from_str::<BootstrapMetadata>(&raw).ok())
        .map(|metadata| metadata.deployment_mode)
        .unwrap_or_default()
}

pub fn save_bootstrap_deployment_mode(
    config_dir: &Path,
    deployment_mode: DeploymentMode,
) -> Result<()> {
    let content = toml::to_string_pretty(&BootstrapMetadata { deployment_mode })?;
    crate::crypto::atomic_write_file(&bootstrap_metadata_path(config_dir), content.as_bytes())?;
    Ok(())
}

fn block_on_storage_future<T, F>(future: F) -> Result<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| handle.block_on(future))
            }
            _ => {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = (|| -> Result<T> {
                        let runtime = Builder::new_current_thread().enable_all().build()?;
                        runtime.block_on(future)
                    })();
                    let _ = tx.send(result);
                });
                rx.recv()
                    .map_err(|_| anyhow!("settings storage worker dropped before completing"))?
            }
        }
    } else {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(future)
    }
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

fn default_true() -> bool {
    true
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

/// Runtime-configurable security guards.
///
/// Defaults deny private/local/metadata targets; administrators can opt in
/// to specific internal hosts via `tool_args.host_whitelist`. The abuse
/// tracker is no-auto-block by design: tripping the threshold requests
/// admin approval rather than locking anyone out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityConfig {
    #[serde(default)]
    pub tool_args: crate::security::tool_args_guard::ToolArgsGuardConfig,
    #[serde(default)]
    pub abuse_tracker: AbuseTrackerConfig,
}

/// Threshold configuration for the abuse approval loop.
///
/// `trips_threshold` = number of inbound-guard blocks tolerated inside
/// `window_minutes` before the source is moved to pending-approval. Admin
/// must explicitly approve or reject — no automatic timeouts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AbuseTrackerConfig {
    #[serde(default = "default_abuse_trips_threshold")]
    pub trips_threshold: u32,
    #[serde(default = "default_abuse_window_minutes")]
    pub window_minutes: u32,
}

impl Default for AbuseTrackerConfig {
    fn default() -> Self {
        Self {
            trips_threshold: default_abuse_trips_threshold(),
            window_minutes: default_abuse_window_minutes(),
        }
    }
}

fn default_abuse_trips_threshold() -> u32 {
    5
}

fn default_abuse_window_minutes() -> u32 {
    10
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
    Disabled,
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

fn default_email_provider() -> String {
    "auto".to_string()
}

fn default_email_transport_kind() -> String {
    "http".to_string()
}

fn default_email_auth_kind() -> String {
    "none".to_string()
}

fn default_email_smtp_port() -> u16 {
    587
}

fn default_email_smtp_security() -> String {
    "starttls".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailConfig {
    #[serde(default = "default_email_provider")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default)]
    pub transport: EmailTransportConfig,
    #[serde(default)]
    pub auth: EmailAuthConfig,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            provider: default_email_provider(),
            to_address: None,
            from_address: None,
            domain: None,
            transport: EmailTransportConfig::default(),
            auth: EmailAuthConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailTransportConfig {
    #[serde(default = "default_email_transport_kind")]
    pub kind: String,
    #[serde(default)]
    pub http: EmailHttpTransportConfig,
    #[serde(default)]
    pub smtp: EmailSmtpTransportConfig,
}

impl Default for EmailTransportConfig {
    fn default() -> Self {
        Self {
            kind: default_email_transport_kind(),
            http: EmailHttpTransportConfig::default(),
            smtp: EmailSmtpTransportConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailHttpTransportConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailSmtpTransportConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_email_smtp_port")]
    pub port: u16,
    #[serde(default = "default_email_smtp_security")]
    pub security: String,
}

impl Default for EmailSmtpTransportConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: default_email_smtp_port(),
            security: default_email_smtp_security(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailAuthConfig {
    #[serde(default = "default_email_auth_kind")]
    pub kind: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(default)]
    pub basic_username: String,
    #[serde(default)]
    pub basic_password: String,
    #[serde(default)]
    pub aws_access_key_id: String,
    #[serde(default)]
    pub aws_secret_access_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_service: Option<String>,
}

impl Default for EmailAuthConfig {
    fn default() -> Self {
        Self {
            kind: default_email_auth_kind(),
            api_key: String::new(),
            header_name: None,
            scheme: None,
            basic_username: String::new(),
            basic_password: String::new(),
            aws_access_key_id: String::new(),
            aws_secret_access_key: String::new(),
            aws_session_token: None,
            aws_region: None,
            aws_service: None,
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
    pub email: EmailConfig,
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
    /// Controls what sensitive content may enter model prompts.
    #[serde(default)]
    pub model_privacy: ModelPrivacyConfig,
    /// Runtime-configurable security guards (tool-argument whitelist,
    /// abuse-tracker thresholds, etc.).
    #[serde(default)]
    pub security: SecurityConfig,
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
            email: EmailConfig::default(),
            sandbox: SandboxConfig::default(),
            memory: MemoryConfig::default(),
            embeddings: Some(EmbeddingsConfig::default()),
            auto_approve: vec![],
            media_gen: MediaGenConfig::default(),
            swarm: SwarmConfig::default(),
            browser: BrowserConfig::default(),
            tunnel: TunnelConfig::default(),
            observability: ObservabilityConfig::default(),
            model_privacy: ModelPrivacyConfig::default(),
            security: SecurityConfig::default(),
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
    /// Keys: replicate, stability_ai, fal, together, openai_dalle, openai_sora,
    /// google_gemini, google_veo, runway, luma
    #[serde(default)]
    pub provider_api_keys: std::collections::HashMap<String, String>,
    /// Optional compatible endpoint overrides for known media provider adapters.
    /// These are not arbitrary custom providers; the selected provider's request/response
    /// format is still used.
    #[serde(default)]
    pub provider_base_urls: std::collections::HashMap<String, String>,
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
    "lan_discover",
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

#[derive(Debug, Clone)]
pub struct SecureConfigRuntimeState {
    pub config: AgentConfig,
    pub config_degraded: bool,
    pub config_issue: Option<String>,
    pub secrets_degraded: bool,
    pub secrets_issue: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StorageKeyLineageStatus {
    pub local_fingerprint: String,
    pub stored_fingerprint: Option<String>,
    pub mismatch: bool,
    pub initialized: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsKeyLineageRecord {
    version: u32,
    fingerprint: String,
    recorded_at: String,
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
        std::fs::create_dir_all(config_dir).map_err(|error| {
            anyhow!(
                "Failed to create secure config directory at {:?}: {}",
                config_dir,
                error
            )
        })?;
        if let Some(data_dir) = data_dir {
            std::fs::create_dir_all(data_dir).map_err(|error| {
                anyhow!(
                    "Failed to create secure data directory at {:?}: {}",
                    data_dir,
                    error
                )
            })?;
        }

        // Prefer global key (master-password-derived) when available
        if let Some(km) = global_key_manager() {
            return Ok(Self {
                key_manager: km,
                config_dir: config_dir.to_path_buf(),
            });
        }

        if crate::crypto::master::MasterPasswordManager::docker_stack_requires_install_master_secret(
        ) {
            let secret =
                crate::crypto::master::MasterPasswordManager::read_install_master_secret()?
                    .ok_or_else(|| {
                        anyhow!(
                            "Install-managed encryption secret is missing at {}",
                            crate::crypto::master::INSTALL_MASTER_SECRET_PATH
                        )
                    })?;
            let master_mgr = crate::crypto::master::MasterPasswordManager::new(
                config_dir,
                data_dir.unwrap_or(config_dir),
            );
            let key_manager = if master_mgr.is_password_set() {
                master_mgr.unlock(&secret)?
            } else {
                master_mgr.initialize_startup_password_if_needed(&secret)?
            };
            return Ok(Self {
                key_manager,
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

    fn storage_backend(&self) -> Option<crate::storage::Storage> {
        #[cfg(test)]
        {
            if self.config_dir.starts_with(std::env::temp_dir()) {
                return None;
            }
        }
        global_settings_storage()
    }

    pub(crate) fn uses_storage_backend(&self) -> bool {
        self.storage_backend().is_some()
    }

    fn short_fingerprint(fingerprint: &str) -> String {
        fingerprint.chars().take(12).collect()
    }

    fn storage_key_lineage_record(&self) -> SettingsKeyLineageRecord {
        SettingsKeyLineageRecord {
            version: 1,
            fingerprint: self.key_manager.fingerprint(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn load_storage_key_lineage_record(
        &self,
    ) -> Result<(Option<SettingsKeyLineageRecord>, Option<String>)> {
        let Some(storage) = self.storage_backend() else {
            return Ok((None, None));
        };
        let raw =
            block_on_storage_future(async move { storage.get(SETTINGS_KEY_LINEAGE_KEY).await })?;
        let Some(raw) = raw else {
            return Ok((None, None));
        };
        match serde_json::from_slice::<SettingsKeyLineageRecord>(&raw) {
            Ok(record) => Ok((Some(record), None)),
            Err(error) => Ok((
                None,
                Some(format!(
                    "Stored key-lineage metadata is unreadable: {}",
                    error
                )),
            )),
        }
    }

    fn persist_storage_key_lineage_record(&self, record: &SettingsKeyLineageRecord) -> Result<()> {
        let Some(storage) = self.storage_backend() else {
            anyhow::bail!("settings storage backend is unavailable");
        };
        let payload = serde_json::to_vec(record)?;
        block_on_storage_future(
            async move { storage.set(SETTINGS_KEY_LINEAGE_KEY, &payload).await },
        )
    }

    fn probe_existing_settings_payloads(&self) -> Result<Option<String>> {
        let Some(storage) = self.storage_backend() else {
            return Ok(None);
        };
        let key_manager = self.key_manager.clone();
        block_on_storage_future(async move {
            for key in SETTINGS_ENCRYPTED_KV_KEYS {
                let Some(raw) = storage.get(key).await? else {
                    continue;
                };
                if raw.is_empty() {
                    continue;
                }
                if key_manager.decrypt(&raw).is_ok() {
                    continue;
                }
                if serde_json::from_slice::<serde_json::Value>(&raw).is_ok() {
                    continue;
                }
                return Ok(Some(format!(
                    "Stored settings payload '{}' cannot be decrypted with the active key and does not look like legacy plaintext JSON.",
                    key
                )));
            }
            Ok(None)
        })
    }

    pub fn verify_or_initialize_storage_key_lineage(&self) -> Result<StorageKeyLineageStatus> {
        let local_fingerprint = self.key_manager.fingerprint();
        if !self.uses_storage_backend() {
            return Ok(StorageKeyLineageStatus {
                local_fingerprint,
                stored_fingerprint: None,
                mismatch: false,
                initialized: false,
                detail: None,
            });
        }

        let (record, record_issue) = self.load_storage_key_lineage_record()?;
        if let Some(record) = record {
            if record.fingerprint == local_fingerprint {
                return Ok(StorageKeyLineageStatus {
                    local_fingerprint,
                    stored_fingerprint: Some(record.fingerprint),
                    mismatch: false,
                    initialized: false,
                    detail: None,
                });
            }

            return Ok(StorageKeyLineageStatus {
                local_fingerprint: local_fingerprint.clone(),
                stored_fingerprint: Some(record.fingerprint.clone()),
                mismatch: true,
                initialized: false,
                detail: Some(format!(
                    "Settings storage key lineage mismatch: Postgres expects key fingerprint {} but the active config volume provides {}. This usually means the database volume and /app/config volume came from different installs or restore points.",
                    Self::short_fingerprint(&record.fingerprint),
                    Self::short_fingerprint(&local_fingerprint)
                )),
            });
        }

        if let Some(detail) = self.probe_existing_settings_payloads()? {
            let mut issue = record_issue.unwrap_or_default();
            if !issue.is_empty() {
                issue.push(' ');
            }
            issue.push_str(&detail);
            return Ok(StorageKeyLineageStatus {
                local_fingerprint,
                stored_fingerprint: None,
                mismatch: true,
                initialized: false,
                detail: Some(issue),
            });
        }

        let record = self.storage_key_lineage_record();
        self.persist_storage_key_lineage_record(&record)?;

        Ok(StorageKeyLineageStatus {
            local_fingerprint,
            stored_fingerprint: Some(record.fingerprint),
            mismatch: false,
            initialized: true,
            detail: record_issue,
        })
    }

    fn ensure_storage_key_lineage_writable(&self) -> Result<()> {
        let lineage = self.verify_or_initialize_storage_key_lineage()?;
        if lineage.mismatch {
            anyhow::bail!(
                "{}",
                lineage.detail.unwrap_or_else(|| {
                    "Settings storage key lineage does not match the active config key.".to_string()
                })
            );
        }
        Ok(())
    }

    fn default_runtime_agent_config() -> AgentConfig {
        AgentConfig::default()
    }

    fn recovery_dir(config_dir: &Path) -> PathBuf {
        config_dir.join("recovery")
    }

    fn recovery_stem_for_key(key: &str) -> String {
        let mut stem = String::with_capacity(key.len());
        for ch in key.chars() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                stem.push(ch);
            } else {
                stem.push('_');
            }
        }
        stem
    }

    fn persist_unreadable_storage_payload(
        config_dir: &Path,
        key: &'static str,
        encrypted: &[u8],
        detail: &str,
    ) -> Result<PathBuf> {
        #[derive(Serialize)]
        struct RecoveryMetadata<'a> {
            key: &'a str,
            detail: &'a str,
            captured_at: String,
        }

        let recovery_dir = Self::recovery_dir(config_dir);
        std::fs::create_dir_all(&recovery_dir)?;

        let stem = Self::recovery_stem_for_key(key);
        let payload_path = recovery_dir.join(format!("{}.bin", stem));
        let metadata_path = recovery_dir.join(format!("{}.json", stem));

        crate::crypto::atomic_write_file(&payload_path, encrypted)?;
        let metadata = RecoveryMetadata {
            key,
            detail,
            captured_at: chrono::Utc::now().to_rfc3339(),
        };
        let metadata_raw = serde_json::to_vec_pretty(&metadata)?;
        crate::crypto::atomic_write_file(&metadata_path, &metadata_raw)?;

        Ok(payload_path)
    }

    fn is_recoverable_storage_payload_error(error: &anyhow::Error) -> bool {
        error
            .to_string()
            .contains("Failed to decrypt encrypted settings payload for '")
    }

    fn load_storage_payload(&self, key: &'static str) -> Result<Option<Vec<u8>>> {
        let Some(storage) = self.storage_backend() else {
            return Ok(None);
        };
        let key_manager = self.key_manager.clone();
        let config_dir = self.config_dir.clone();
        block_on_storage_future(async move {
            let Some(encrypted) = storage.get(key).await? else {
                return Ok(None);
            };
            if encrypted.is_empty() {
                return Ok(None);
            }
            let decrypted = key_manager.decrypt(&encrypted).map_err(|error| {
                let mut detail = format!(
                    "Failed to decrypt encrypted settings payload for '{}': {}",
                    key, error
                );
                match Self::persist_unreadable_storage_payload(
                    &config_dir,
                    key,
                    &encrypted,
                    &detail,
                ) {
                    Ok(path) => {
                        detail.push_str(&format!(
                            ". Raw encrypted payload was preserved at {}",
                            path.display()
                        ));
                    }
                    Err(backup_error) => {
                        tracing::warn!(
                            "Failed to preserve unreadable encrypted settings payload for '{}': {}",
                            key,
                            backup_error
                        );
                    }
                }
                anyhow!(detail)
            })?;
            Ok(Some(decrypted))
        })
    }

    fn save_storage_payload(&self, key: &'static str, payload: &[u8]) -> Result<()> {
        let Some(storage) = self.storage_backend() else {
            anyhow::bail!("settings storage backend is unavailable");
        };
        let key_manager = self.key_manager.clone();
        let payload = payload.to_vec();
        block_on_storage_future(async move {
            let encrypted = key_manager.encrypt(&payload)?;
            storage.set(key, &encrypted).await
        })
    }

    pub(crate) fn load_encrypted_json<T>(&self, key: &'static str) -> Result<Option<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let Some(raw) = self.load_storage_payload(key)? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice::<T>(&raw)?))
    }

    pub(crate) fn save_encrypted_json<T>(&self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
    {
        if self.uses_storage_backend() {
            self.ensure_storage_key_lineage_writable()?;
            self.ensure_encrypted_json_writable::<T>(key)?;
        }
        let payload = serde_json::to_vec(value)?;
        self.save_storage_payload(key, &payload)
    }

    fn load_encrypted_json_runtime_state<T>(
        &self,
        key: &'static str,
    ) -> Result<(Option<T>, bool, Option<String>)>
    where
        T: DeserializeOwned + Send + 'static,
    {
        match self.load_storage_payload(key) {
            Ok(None) => Ok((None, false, None)),
            Ok(Some(raw)) => match serde_json::from_slice::<T>(&raw) {
                Ok(value) => Ok((Some(value), false, None)),
                Err(error) => {
                    let detail = format!(
                        "Failed to parse encrypted settings payload for '{}': {}",
                        key, error
                    );
                    Ok((None, true, Some(detail)))
                }
            },
            Err(error) if Self::is_recoverable_storage_payload_error(&error) => {
                Ok((None, true, Some(error.to_string())))
            }
            Err(error) => Err(error),
        }
    }

    fn ensure_encrypted_json_writable<T>(&self, key: &'static str) -> Result<()>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let (_value, degraded, detail) = self.load_encrypted_json_runtime_state::<T>(key)?;
        if degraded {
            anyhow::bail!(
                "Refusing to overwrite encrypted settings payload for '{}': {}. Restore the original key material or repair the stored payload before saving.",
                key,
                detail.unwrap_or_else(|| "the current payload is unreadable".to_string())
            );
        }
        Ok(())
    }

    fn secrets_path(&self) -> PathBuf {
        self.config_dir.join("secrets.enc")
    }

    pub(crate) fn load_secrets_unlocked(&self) -> Result<Secrets> {
        if self.uses_storage_backend() {
            return Ok(self
                .load_encrypted_json::<Secrets>(SETTINGS_SECRETS_KEY)?
                .unwrap_or_default());
        }

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

    fn load_secrets_runtime_state_unlocked_detail(
        &self,
    ) -> Result<(Secrets, bool, Option<String>)> {
        match self.load_secrets_unlocked() {
            Ok(secrets) => Ok((secrets, false, None)),
            Err(e) => {
                let detail = e.to_string();
                tracing::error!(
                    "Failed to load encrypted settings from storage: {}. \
                     Starting with empty runtime secrets; encrypted data was preserved for recovery.",
                    detail
                );
                Ok((Secrets::default(), true, Some(detail)))
            }
        }
    }

    fn load_secrets_runtime_state_unlocked(&self) -> Result<(Secrets, bool)> {
        let (secrets, degraded, _detail) = self.load_secrets_runtime_state_unlocked_detail()?;
        Ok((secrets, degraded))
    }

    fn load_config_runtime_state_unlocked(&self) -> Result<(AgentConfig, bool, Option<String>)> {
        if self.uses_storage_backend() {
            let (config, degraded, detail) =
                self.load_encrypted_json_runtime_state::<AgentConfig>(SETTINGS_CONFIG_KEY)?;
            if let Some(config) = config {
                return Ok((config, false, None));
            }
            if degraded {
                return Ok((Self::default_runtime_agent_config(), true, detail));
            }

            let config = Self::default_runtime_agent_config();
            self.save_config_only(&config)?;
            return Ok((config, false, None));
        }

        let config_path = self.config_dir.join("config.toml");
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            return Ok((toml::from_str(&content)?, false, None));
        }

        let config = Self::default_runtime_agent_config();
        self.save_config_only(&config)?;
        Ok((config, false, None))
    }

    pub fn load_runtime_state(&self) -> Result<SecureConfigRuntimeState> {
        let (mut config, config_degraded, config_issue) =
            self.load_config_runtime_state_unlocked()?;
        if config_degraded {
            tracing::error!(
                "Failed to load encrypted agent config from settings storage: {}. \
                 Starting with default runtime config; encrypted data was preserved for recovery and settings writes will stay blocked until the original key is restored.",
                config_issue
                    .as_deref()
                    .unwrap_or("unknown settings load failure")
            );
        }

        let (
            secrets,
            _changed,
            created,
            rotated,
            persisted_change,
            secrets_degraded,
            secrets_issue,
        ) = if self.uses_storage_backend() {
            self.with_secrets_lock(|manager| {
                let (mut secrets, degraded, detail) =
                    manager.load_secrets_runtime_state_unlocked_detail()?;
                let (changed, created, rotated) =
                    Self::ensure_http_api_key_in_secrets(&mut secrets);
                let persisted_change = changed && !degraded;
                if persisted_change {
                    manager.save_secrets_unlocked(&secrets)?;
                }
                Ok((
                    secrets,
                    changed,
                    created,
                    rotated,
                    persisted_change,
                    degraded,
                    detail,
                ))
            })?
        } else if self.secrets_path().exists() {
            self.with_secrets_lock(|manager| {
                let (mut secrets, degraded, detail) =
                    manager.load_secrets_runtime_state_unlocked_detail()?;
                let (changed, created, rotated) =
                    Self::ensure_http_api_key_in_secrets(&mut secrets);
                let persisted_change = changed && !degraded;
                if persisted_change {
                    manager.save_secrets_unlocked(&secrets)?;
                }
                Ok((
                    secrets,
                    changed,
                    created,
                    rotated,
                    persisted_change,
                    degraded,
                    detail,
                ))
            })?
        } else {
            self.migrate_from_plain_config(&mut config)?;

            let (changed, created, rotated) = self.with_secrets_lock(|manager| {
                let mut secrets = manager.load_secrets_unlocked()?;
                let (changed, created, rotated) =
                    Self::ensure_http_api_key_in_secrets(&mut secrets);
                if changed {
                    manager.save_secrets_unlocked(&secrets)?;
                }
                Ok((changed, created, rotated))
            })?;

            let secrets = self.with_secrets_lock(|manager| manager.load_secrets_unlocked())?;
            (secrets, changed, created, rotated, changed, false, None)
        };

        if persisted_change {
            if rotated {
                tracing::info!("Rotated expired HTTP API key");
            } else if created {
                tracing::info!("Generated new HTTP API key for authentication");
            }
        }
        self.inject_secrets(&mut config, &secrets);

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

        Ok(SecureConfigRuntimeState {
            config,
            config_degraded,
            config_issue,
            secrets_degraded,
            secrets_issue,
        })
    }

    pub(crate) fn save_secrets_unlocked(&self, secrets: &Secrets) -> Result<()> {
        if self.uses_storage_backend() {
            self.save_encrypted_json(SETTINGS_SECRETS_KEY, secrets)?;
            return Ok(());
        }
        let json = serde_json::to_vec(secrets)?;
        let encrypted = self.key_manager.encrypt(&json)?;
        crate::crypto::atomic_write_file(&self.secrets_path(), &encrypted)?;
        Ok(())
    }

    pub(crate) fn save_secrets_unlocked_for_rekey(&self, secrets: &Secrets) -> Result<()> {
        if self.uses_storage_backend() {
            let payload = serde_json::to_vec(secrets)?;
            self.save_storage_payload(SETTINGS_SECRETS_KEY, &payload)?;
            return Ok(());
        }
        self.save_secrets_unlocked(secrets)
    }

    pub(crate) fn with_secrets_lock<T, F>(&self, op: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Result<T>,
    {
        run_blocking_config_section(|| {
            let _guard = secrets_file_lock()
                .lock()
                .map_err(|_| anyhow!("secrets file lock poisoned"))?;
            op(self)
        })
    }

    pub fn update_secrets<T, F>(&self, update: F) -> Result<T>
    where
        F: FnOnce(&mut Secrets) -> Result<T>,
    {
        self.with_secrets_lock(|manager| {
            let (mut secrets, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            if degraded {
                anyhow::bail!(
                    "Refusing to update encrypted secrets because the active settings payload could not be decrypted with the current key. Restore the correct key material before mutating secrets."
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
        Ok(self.load_runtime_state()?.config)
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

        // Save sanitized mutable settings
        self.save_config_only(&sanitized)?;

        self.with_secrets_lock(|manager| {
            let (existing, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            if degraded {
                anyhow::bail!(
                    "Refusing to save encrypted secrets because the active settings payload could not be decrypted with the current key."
                );
            }
            let secrets = manager.extract_secrets_from_base(existing, config);
            if !manager.uses_storage_backend()
                && !Self::has_real_secrets(&secrets)
                && manager.secrets_path().exists()
            {
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

    /// Save sanitized mutable settings.
    fn save_config_only(&self, config: &AgentConfig) -> Result<()> {
        if self.uses_storage_backend() {
            self.ensure_storage_key_lineage_writable()?;
            self.ensure_encrypted_json_writable::<AgentConfig>(SETTINGS_CONFIG_KEY)?;
        }
        save_bootstrap_deployment_mode(&self.config_dir, config.deployment_mode)?;
        if self.uses_storage_backend() {
            self.save_encrypted_json(SETTINGS_CONFIG_KEY, config)?;
        } else {
            let config_path = self.config_dir.join("config.toml");
            let content = toml::to_string_pretty(config)?;
            crate::crypto::atomic_write_file(&config_path, content.as_bytes())?;
        }
        Ok(())
    }

    /// Save encrypted secrets
    pub(crate) fn save_secrets(&self, secrets: &Secrets) -> Result<()> {
        self.with_secrets_lock(|manager| manager.save_secrets_unlocked(secrets))
    }

    pub(crate) fn load_secrets(&self) -> Result<Secrets> {
        self.with_secrets_lock(|manager| {
            let (secrets, degraded) = manager.load_secrets_runtime_state_unlocked()?;
            if degraded {
                anyhow::bail!(
                    "Encrypted settings are temporarily unavailable from storage. Refusing to continue with empty runtime secrets."
                );
            }
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

    fn clear_email_secret_entries(custom: &mut std::collections::HashMap<String, String>) {
        for key in [
            "email.auth.api_key",
            "email.auth.basic_password",
            "email.auth.aws_secret_access_key",
            "email.auth.aws_session_token",
        ] {
            custom.remove(key);
        }
    }

    fn upsert_email_secret_entry(
        custom: &mut std::collections::HashMap<String, String>,
        key: &str,
        value: &str,
    ) {
        if value.is_empty() || value == "[ENCRYPTED]" {
            custom.remove(key);
        } else {
            custom.insert(key.to_string(), value.to_string());
        }
    }

    /// Extract secrets from config
    fn extract_secrets_from_base(&self, mut secrets: Secrets, config: &AgentConfig) -> Secrets {
        // Model-related secrets are derived entirely from the current config/runtime state.
        // Clear them first so deleted slots and removed legacy keys do not persist forever.
        secrets.llm_api_key = None;
        secrets.embeddings_api_key = None;
        secrets.llm_fallback_api_key = None;
        secrets.model_pool_keys.clear();
        Self::clear_email_secret_entries(&mut secrets.custom);

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

        Self::upsert_email_secret_entry(
            &mut secrets.custom,
            "email.auth.api_key",
            &config.email.auth.api_key,
        );
        Self::upsert_email_secret_entry(
            &mut secrets.custom,
            "email.auth.basic_password",
            &config.email.auth.basic_password,
        );
        Self::upsert_email_secret_entry(
            &mut secrets.custom,
            "email.auth.aws_secret_access_key",
            &config.email.auth.aws_secret_access_key,
        );
        if let Some(session_token) = config.email.auth.aws_session_token.as_deref() {
            Self::upsert_email_secret_entry(
                &mut secrets.custom,
                "email.auth.aws_session_token",
                session_token,
            );
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
        if !sanitized.email.auth.api_key.is_empty() {
            sanitized.email.auth.api_key = "[ENCRYPTED]".to_string();
        }
        if !sanitized.email.auth.basic_password.is_empty() {
            sanitized.email.auth.basic_password = "[ENCRYPTED]".to_string();
        }
        if !sanitized.email.auth.aws_secret_access_key.is_empty() {
            sanitized.email.auth.aws_secret_access_key = "[ENCRYPTED]".to_string();
        }
        if sanitized
            .email
            .auth
            .aws_session_token
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            sanitized.email.auth.aws_session_token = Some("[ENCRYPTED]".to_string());
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

        if let Some(value) = secrets.custom.get("email.auth.api_key") {
            if !value.is_empty() && value != "[ENCRYPTED]" {
                config.email.auth.api_key = value.clone();
            }
        }
        if let Some(value) = secrets.custom.get("email.auth.basic_password") {
            if !value.is_empty() && value != "[ENCRYPTED]" {
                config.email.auth.basic_password = value.clone();
            }
        }
        if let Some(value) = secrets.custom.get("email.auth.aws_secret_access_key") {
            if !value.is_empty() && value != "[ENCRYPTED]" {
                config.email.auth.aws_secret_access_key = value.clone();
            }
        }
        if let Some(value) = secrets.custom.get("email.auth.aws_session_token") {
            if !value.is_empty() && value != "[ENCRYPTED]" {
                config.email.auth.aws_session_token = Some(value.clone());
            }
        }

        // Inject Telegram token
        if let Some(token) = &secrets.telegram_bot_token {
            if !token.is_empty() && token != "[ENCRYPTED]" {
                if let Some(tg) = &mut config.telegram {
                    tg.bot_token = token.clone();
                } else {
                    // Secret exists but config section was lost (e.g. volume reset) — recreate it
                    tracing::info!(
                        "Recovered Telegram config from encrypted secrets (config.toml was missing [telegram] section)"
                    );
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
        use rand::RngExt;
        let mut key_bytes = [0u8; 32];
        rand::rng().fill(&mut key_bytes);
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
    #[serde(default)]
    pub embedding_model: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            embedding_model: String::new(),
        }
    }
}
