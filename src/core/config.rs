//! Agent configuration with encryption for sensitive data
//!
//! Non-sensitive config is stored in config.toml (readable)
//! Sensitive data (API keys, tokens) is stored encrypted in secrets.enc

use super::llm::LlmProvider;
use super::swarm::SwarmConfig;
use crate::crypto::KeyManager;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Global key manager set at startup (from master password or keyfile).
/// All SecureConfigManager instances use this when available, ensuring
/// consistent encryption across the entire process after password changes.
static GLOBAL_KEY_MANAGER: std::sync::OnceLock<Arc<KeyManager>> = std::sync::OnceLock::new();
static CUSTOM_SECRETS_UPDATE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();
pub const HTTP_API_KEY_TTL_SECS: i64 = 24 * 60 * 60;

/// Set the global key manager (called once at startup from main.rs)
pub fn set_global_key_manager(km: Arc<KeyManager>) {
    let _ = GLOBAL_KEY_MANAGER.set(km);
}

/// Get the global key manager if set
pub fn global_key_manager() -> Option<Arc<KeyManager>> {
    GLOBAL_KEY_MANAGER.get().cloned()
}

fn custom_secrets_update_lock() -> &'static std::sync::Mutex<()> {
    CUSTOM_SECRETS_UPDATE_LOCK.get_or_init(|| std::sync::Mutex::new(()))
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
    /// Optional model slot ID dedicated for app_deploy planning/generation.
    /// When unset, app deploy uses the default primary model.
    #[serde(default)]
    pub app_deploy_model_id: Option<String>,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub whatsapp: Option<crate::channels::whatsapp::WhatsAppChannelConfig>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
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
    /// Mem0 memory layer configuration
    #[serde(default)]
    pub mem0: Mem0Config,
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
            name: "AgentArk".to_string(),
            personality: default_personality(),
            llm: LlmProvider::default(),
            llm_fallback: None,
            model_pool: ModelPoolConfig::default(),
            app_deploy_model_id: None,
            telegram: None,
            whatsapp: None,
            sandbox: SandboxConfig::default(),
            memory: MemoryConfig::default(),
            auto_approve: vec![],
            media_gen: MediaGenConfig::default(),
            swarm: SwarmConfig::default(),
            browser: BrowserConfig::default(),
            mem0: Mem0Config::default(),
            mcp: McpConfig::default(),
            tls_cert_path: None,
            tls_key_path: None,
        }
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

/// Mem0 memory layer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mem0Config {
    /// URL of the Mem0 sidecar bridge
    #[serde(default = "default_mem0_bridge_url")]
    pub bridge_url: String,
    /// Enable Mem0 memory layer (disable to fall back to built-in word-overlap)
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_mem0_bridge_url() -> String {
    "http://127.0.0.1:8991".to_string()
}

impl Default for Mem0Config {
    fn default() -> Self {
        Self {
            bridge_url: default_mem0_bridge_url(),
            enabled: true,
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
    /// Allowlist of tool names to auto-approve (empty = require approval)
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// Allowlist of resource URIs to auto-approve (empty = require approval)
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
    "gmail_send",
    "gmail_reply",
];

/// Encrypted secrets storage
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Secrets {
    /// Primary LLM API key
    pub llm_api_key: Option<String>,
    /// Fallback LLM API key
    pub llm_fallback_api_key: Option<String>,
    /// Telegram bot token
    pub telegram_bot_token: Option<String>,
    /// WhatsApp access token
    #[serde(default)]
    pub whatsapp_access_token: Option<String>,
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

    /// Load configuration with decrypted secrets
    pub fn load(&self) -> Result<AgentConfig> {
        let config_path = self.config_dir.join("config.toml");
        let secrets_path = self.config_dir.join("secrets.enc");

        // Load base config
        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            let config = AgentConfig::default();
            self.save_config_only(&config)?;
            config
        };

        // Load and decrypt secrets
        if secrets_path.exists() {
            let mut secrets = self.load_secrets()?;
            let (changed, created, rotated) = Self::ensure_http_api_key_in_secrets(&mut secrets);
            if changed {
                self.save_secrets(&secrets)?;
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
            let mut secrets = self.load_secrets().unwrap_or_default();
            let (changed, created, rotated) = Self::ensure_http_api_key_in_secrets(&mut secrets);
            if changed {
                self.save_secrets(&secrets)?;
                if rotated {
                    tracing::info!("Rotated expired HTTP API key");
                } else if created {
                    tracing::info!("Generated HTTP API key for authentication (first run)");
                }
            }
        }

        // Auto-migrate legacy llm/llm_fallback to model_pool if slots is empty
        // Only migrate if the user actually configured a non-default provider (not fresh install defaults)
        if config.model_pool.slots.is_empty() {
            let is_default =
                matches!(&config.llm, LlmProvider::Ollama { model, .. } if model == "llama3.2");
            if !is_default || config.llm_fallback.is_some() {
                let primary_slot = ModelSlot {
                    id: "primary".to_string(),
                    label: "Primary".to_string(),
                    role: ModelRole::Primary,
                    provider: config.llm.clone(),
                    enabled: true,
                };
                config.model_pool.slots.push(primary_slot);

                if let Some(fallback) = &config.llm_fallback {
                    let fallback_slot = ModelSlot {
                        id: "fallback".to_string(),
                        label: "Fallback".to_string(),
                        role: ModelRole::Fallback,
                        provider: fallback.clone(),
                        enabled: true,
                    };
                    config.model_pool.slots.push(fallback_slot);
                }
                tracing::info!(
                    "Migrated legacy llm config to model_pool ({} slots)",
                    config.model_pool.slots.len()
                );
            }
        }

        Ok(config)
    }

    /// Save configuration with encrypted secrets
    pub fn save(&self, config: &AgentConfig) -> Result<()> {
        // Extract secrets from config
        let secrets = self.extract_secrets(config);

        // Create sanitized config (without secrets)
        let sanitized = self.sanitize_config(config);

        // Save non-sensitive config as TOML
        self.save_config_only(&sanitized)?;

        // Guard: don't overwrite existing secrets.enc with empty data.
        // This prevents data loss when decryption fails (key mismatch)
        // but the user saves settings via the web UI.
        let secrets_path = self.config_dir.join("secrets.enc");
        let has_real_secrets = secrets.llm_api_key.as_ref().is_some_and(|k| !k.is_empty())
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
            || secrets.api_key.as_ref().is_some_and(|k| !k.is_empty())
            || !secrets.media_provider_keys.is_empty()
            || !secrets.model_pool_keys.is_empty()
            || !secrets.mcp_auth.is_empty()
            || !secrets.custom.is_empty();

        if !has_real_secrets && secrets_path.exists() {
            tracing::warn!(
                "Skipping secrets.enc overwrite: extracted secrets are empty but file exists. \
                 This likely means decryption failed — preserving existing encrypted secrets."
            );
        } else {
            self.save_secrets(&secrets)?;
        }

        Ok(())
    }

    /// Save only the config.toml (without encryption)
    fn save_config_only(&self, config: &AgentConfig) -> Result<()> {
        let config_path = self.config_dir.join("config.toml");
        let content = toml::to_string_pretty(config)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    /// Save encrypted secrets
    pub(crate) fn save_secrets(&self, secrets: &Secrets) -> Result<()> {
        let secrets_path = self.config_dir.join("secrets.enc");
        let json = serde_json::to_vec(secrets)?;
        let encrypted = self.key_manager.encrypt(&json)?;
        std::fs::write(secrets_path, encrypted)?;
        Ok(())
    }

    pub(crate) fn load_secrets(&self) -> Result<Secrets> {
        let secrets_path = self.config_dir.join("secrets.enc");
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
    }

    /// Atomically update custom secret keys in a single read-modify-write cycle.
    ///
    /// This guards against concurrent lost updates across async handlers that may
    /// mutate the same encrypted `secrets.enc` file.
    pub fn update_custom_secrets<T, F>(&self, update: F) -> Result<T>
    where
        F: FnOnce(&mut std::collections::HashMap<String, String>) -> Result<T>,
    {
        let _guard = custom_secrets_update_lock()
            .lock()
            .map_err(|_| anyhow!("custom secret update lock poisoned"))?;
        let mut secrets = self.load_secrets()?;
        let out = update(&mut secrets.custom)?;
        self.save_secrets(&secrets)?;
        Ok(out)
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
    fn extract_secrets(&self, config: &AgentConfig) -> Secrets {
        // Start from existing secrets so we don't lose fields not tracked in AgentConfig
        // (e.g., api_key for HTTP auth, custom secrets from integrations)
        let mut secrets = self.load_secrets().unwrap_or_default();

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

        // Extract WhatsApp access token
        if let Some(wa) = &config.whatsapp {
            if !wa.access_token.is_empty() && wa.access_token != "[ENCRYPTED]" {
                secrets.whatsapp_access_token = Some(wa.access_token.clone());
            }
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

        // Replace WhatsApp access token with placeholder
        if let Some(wa) = &mut sanitized.whatsapp {
            if !wa.access_token.is_empty() {
                wa.access_token = "[ENCRYPTED]".to_string();
            }
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
                        phone_number_id: String::new(),
                        verify_token: "agentark_verify".to_string(),
                        bridge_url: "http://127.0.0.1:8999".to_string(),
                        allowed_numbers: vec![],
                        dm_policy: "pairing".to_string(),
                    });
                }
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
        let secrets = self.extract_secrets(config);

        // Only migrate if there are actual secrets
        let has_secrets = secrets
            .llm_api_key
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
        let mut secrets = self.load_secrets()?;
        let (changed, _created, rotated) = Self::ensure_http_api_key_in_secrets(&mut secrets);
        if changed {
            self.save_secrets(&secrets)?;
        }
        Ok((Self::api_key_info_from_secrets(&secrets), rotated))
    }

    /// Regenerate the HTTP API key and return the new one
    #[allow(dead_code)]
    pub fn regenerate_api_key(&self) -> Result<String> {
        Ok(self.regenerate_api_key_info()?.key)
    }

    /// Regenerate the HTTP API key and return key + TTL metadata
    pub fn regenerate_api_key_info(&self) -> Result<HttpApiKeyInfo> {
        let mut secrets = self.load_secrets().unwrap_or_default();
        let now = Self::current_unix_ts();
        let api_key = Self::generate_http_api_key();
        secrets.api_key = Some(api_key);
        secrets.api_key_issued_at = Some(now);
        secrets.api_key_expires_at = Some(now + HTTP_API_KEY_TTL_SECS);
        self.save_secrets(&secrets)?;
        tracing::info!(
            "HTTP API key regenerated (expires in {} seconds)",
            HTTP_API_KEY_TTL_SECS
        );
        Self::api_key_info_from_secrets(&secrets)
            .ok_or_else(|| anyhow!("Failed to build regenerated API key metadata"))
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
        for action in list {
            if AUTO_APPROVE_BLOCKED.contains(&action.as_str()) {
                tracing::warn!(
                    "Cannot auto-approve '{}': action is in the blocked list",
                    action
                );
                rejected.push(action.clone());
            } else {
                allowed.push(action.clone());
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
    "agentark-sandbox:latest".to_string()
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
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Optional retention pruning (disabled by default).
    /// Only applies to episodic episodes (not semantic facts).
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

fn default_embedding_model() -> String {
    "BAAI/bge-small-en-v1.5".to_string()
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
            embedding_model: default_embedding_model(),
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
