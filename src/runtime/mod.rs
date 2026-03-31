//! Action Runtime - WASM Sandbox + Docker + Transactional Execution
//!
//! Based on arXiv:2512.12806 "Fault-Tolerant Sandboxing"
//!
//! Features:
//! - WASM sandbox for lightweight, fast action execution
//! - Docker sandbox for heavier/untrusted operations
//! - Transactional filesystem with rollback capability

mod sandbox;
mod transaction;

pub use sandbox::{ActionSandbox, SandboxMode};
pub use transaction::TransactionManager;

use anyhow::{Context, Result};
#[cfg(feature = "docker")]
use futures::TryStreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use crate::actions::{ActionDef, ActionSource};
use crate::core::config::{AgentConfig, SecureConfigManager};

/// Runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub default_sandbox: SandboxMode,
    pub wasm_memory_limit: u64,
    pub docker_image: String,
    pub enable_rollback: bool,
    pub snapshot_dir: PathBuf,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_sandbox: SandboxMode::Wasm,
            wasm_memory_limit: 256 * 1024 * 1024, // 256MB
            docker_image: "agentark-sandbox:latest".to_string(),
            enable_rollback: true,
            snapshot_dir: PathBuf::from("snapshots"),
        }
    }
}

/// The action runtime that manages execution
pub struct ActionRuntime {
    config: RuntimeConfig,
    _sandbox: ActionSandbox,
    /// Transactions wrapped in Mutex for concurrent access
    transactions: tokio::sync::Mutex<TransactionManager>,
    /// Actions wrapped in RwLock for concurrent access
    actions: tokio::sync::RwLock<HashMap<String, LoadedAction>>,
    /// Bundled actions explicitly disabled by user (persisted on disk)
    disabled_actions: tokio::sync::RwLock<HashSet<String>>,
    disabled_actions_file: PathBuf,
    actions_dir: PathBuf,
    cli_skills_dir: PathBuf,
    config_dir: PathBuf,
    /// Shared task queue for list_tasks action
    task_queue: Option<std::sync::Arc<tokio::sync::RwLock<crate::core::TaskQueue>>>,
    /// Action security guard for integrity, static analysis, permissions, injection detection
    action_guard: Option<std::sync::Arc<crate::security::ActionGuard>>,
    /// Shared storage for expense + entity operations
    storage: Option<crate::storage::Storage>,
    /// MCP registry for external tools/resources
    mcp_registry: Option<std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>>,
    /// Plugin registry for third-party HTTP extensions
    plugin_registry:
        Option<std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>>,
}

const LOCAL_APP_HTTP_PORT: u16 = 8990;
const HTTP_GET_TIMEOUT_SECS: u64 = 10;
const HTTP_GET_MAX_BODY_BYTES: usize = 1_000_000;
const MAX_NATIVE_ENV_OVERRIDES: usize = 32;

/// A loaded action ready for execution
struct LoadedAction {
    info: ActionDef,
    wasm_module: Option<Vec<u8>>,
    /// Workflow content from SKILL.md (legacy ACTION.md still supported)
    workflow_content: Option<String>,
    /// Optional fixed local CLI binding backed by a verified host executable
    cli_binding: Option<CliToolBinding>,
    /// Optional MCP binding (external tool/resource)
    mcp_binding: Option<McpBinding>,
    /// Optional plugin binding (third-party HTTP extension)
    plugin_binding: Option<PluginBinding>,
    /// Optional imported custom API binding
    custom_api_binding: Option<CustomApiBinding>,
}

#[derive(Debug, Clone)]
pub struct McpBinding {
    pub server_id: String,
    pub kind: McpBindingKind,
}

#[derive(Debug, Clone)]
pub enum McpBindingKind {
    Tool { name: String },
    Resource { uri: String },
}

#[derive(Debug, Clone)]
pub struct PluginBinding {
    pub plugin_id: String,
    pub action_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolBinding {
    pub executable_path: String,
    #[serde(default)]
    pub verify_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledCliSkillManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub executable_path: String,
    #[serde(default)]
    pub verify_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomApiBinding {
    pub api_id: String,
    pub api_name: String,
    pub operation_id: String,
    pub operation_name: String,
    pub method: String,
    pub base_url: String,
    pub path: String,
    pub read_only: bool,
    pub secret_key: String,
    pub auth_mode: crate::custom_apis::CustomApiAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_username: Option<String>,
    #[serde(default)]
    pub default_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub default_query: BTreeMap<String, String>,
    #[serde(default)]
    pub parameters: Vec<crate::custom_apis::CustomApiParameter>,
    #[serde(default)]
    pub body_required: bool,
}

pub const WORKFLOW_ACTION_MARKER: &str = "__WORKFLOW_ACTION__:";
pub const WORKFLOW_MISSING_INPUTS_MARKER: &str = "__WORKFLOW_MISSING_INPUTS__:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMissingInputsPayload {
    pub action: String,
    pub missing: Vec<String>,
    pub required: Vec<String>,
    pub provided: Vec<String>,
    pub query: String,
}

pub fn parse_workflow_action_marker(output: &str) -> Option<(String, String)> {
    let payload = output.strip_prefix(WORKFLOW_ACTION_MARKER)?;
    let mut parts = payload.splitn(2, ':');
    let action = parts.next()?.trim();
    if action.is_empty() {
        return None;
    }
    let query = parts.next().unwrap_or("").to_string();
    Some((action.to_string(), query))
}

pub fn parse_workflow_missing_inputs_marker(output: &str) -> Option<WorkflowMissingInputsPayload> {
    let payload = output.strip_prefix(WORKFLOW_MISSING_INPUTS_MARKER)?;
    serde_json::from_str::<WorkflowMissingInputsPayload>(payload).ok()
}

/// Isolation level for ephemeral Docker containers
#[cfg(feature = "docker")]
#[derive(Clone, Copy)]
enum ContainerIsolation {
    /// Strict: read-only root, no network, noexec /tmp. For shell commands.
    Strict,
    /// Standard: writable fs, network allowed (for pip/npm install), but still
    /// memory/CPU/PID limited, ephemeral, and auto-removed. For code execution.
    Standard,
}

impl ActionRuntime {
    fn remap_workspace_alias_path(&self, raw: &str) -> Option<PathBuf> {
        let trimmed = raw.trim();
        const PREFIXES: &[&str] = &["/workspace", "/repo", "/project"];
        let matched = PREFIXES.iter().find(|prefix| {
            trimmed == **prefix
                || trimmed
                    .strip_prefix(**prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })?;
        let cwd = std::env::current_dir().ok()?;
        let suffix = trimmed.strip_prefix(matched).unwrap_or("");
        let relative = suffix.trim_start_matches('/');
        if relative.is_empty() {
            Some(cwd)
        } else {
            Some(cwd.join(relative))
        }
    }

    fn allowed_file_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            self.data_dir().to_path_buf(),
            self.actions_dir.clone(),
            self.config_dir.clone(),
        ];
        if let Ok(cwd) = std::env::current_dir() {
            roots.push(cwd);
        }

        let mut deduped = Vec::new();
        for root in roots {
            let candidate = root.canonicalize().unwrap_or(root);
            if !deduped
                .iter()
                .any(|existing: &PathBuf| existing == &candidate)
            {
                deduped.push(candidate);
            }
        }
        deduped
    }

    fn absolutize_tool_path(&self, raw: &str) -> Result<PathBuf> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Path cannot be empty");
        }

        if let Some(remapped) = self.remap_workspace_alias_path(trimmed) {
            return Ok(remapped);
        }

        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(std::env::current_dir()?.join(path))
        }
    }

    fn ensure_tool_path_allowed(&self, candidate: &Path) -> Result<()> {
        let allowed_roots = self.allowed_file_roots();
        if allowed_roots.iter().any(|root| candidate.starts_with(root)) {
            return Ok(());
        }

        let roots = allowed_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "Path '{}' is outside allowed roots: {}",
            candidate.display(),
            roots
        );
    }

    fn resolve_tool_read_path(&self, raw: &str) -> Result<PathBuf> {
        let candidate = self.absolutize_tool_path(raw)?;
        let resolved = candidate.canonicalize()?;
        self.ensure_tool_path_allowed(&resolved)?;
        Ok(resolved)
    }

    fn resolve_tool_write_path(&self, raw: &str) -> Result<PathBuf> {
        let candidate = self.absolutize_tool_path(raw)?;
        if candidate.exists() {
            let resolved = candidate.canonicalize()?;
            self.ensure_tool_path_allowed(&resolved)?;
            if resolved.is_dir() {
                anyhow::bail!("Refusing to overwrite directory '{}'", resolved.display());
            }
            return Ok(resolved);
        }

        let mut missing_components = Vec::new();
        let mut cursor = candidate.as_path();
        while !cursor.exists() {
            let name = cursor
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Path '{}' has no existing parent", raw))?;
            missing_components.push(name.to_os_string());
            cursor = cursor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Path '{}' has no existing parent", raw))?;
        }

        let mut rebuilt = cursor.canonicalize()?;
        self.ensure_tool_path_allowed(&rebuilt)?;
        for component in missing_components.into_iter().rev() {
            let component_text = component.to_string_lossy();
            if component_text.is_empty() || component_text == "." || component_text == ".." {
                anyhow::bail!("Invalid path component '{}'", component_text);
            }
            rebuilt.push(component);
        }
        Ok(rebuilt)
    }

    fn loopback_http_get_allowed(url: &reqwest::Url) -> Result<()> {
        let port = url
            .port_or_known_default()
            .ok_or_else(|| anyhow::anyhow!("URL is missing a usable port"))?;
        if port != LOCAL_APP_HTTP_PORT {
            anyhow::bail!(
                "Loopback http_get is restricted to the local app host on port {}",
                LOCAL_APP_HTTP_PORT
            );
        }

        let path = url.path();
        if path != "/apps" && !path.starts_with("/apps/") {
            anyhow::bail!("Loopback http_get is restricted to deployed app URLs under /apps/");
        }
        Ok(())
    }

    fn host_is_explicitly_local(host: &str) -> bool {
        let normalized = host.trim().to_ascii_lowercase();
        if normalized == "localhost" {
            return true;
        }
        normalized.parse::<IpAddr>().is_ok_and(|ip| match ip {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback(),
        })
    }

    fn ipv4_is_public(ip: Ipv4Addr) -> bool {
        let octets = ip.octets();
        !(ip.is_private()
            || ip.is_loopback()
            || ip.is_link_local()
            || ip.is_multicast()
            || ip.is_unspecified()
            || octets == [255, 255, 255, 255]
            || octets[0] == 0
            || (octets[0] == 100 && (64..=127).contains(&octets[1]))
            || (octets[0] == 169 && octets[1] == 254)
            || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
            || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
            || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
            || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
            || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113))
    }

    fn ipv6_is_public(ip: Ipv6Addr) -> bool {
        !(ip.is_loopback()
            || ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_unique_local()
            || ip.is_unicast_link_local())
    }

    fn ip_is_public(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => Self::ipv4_is_public(v4),
            IpAddr::V6(v6) => Self::ipv6_is_public(v6),
        }
    }

    async fn validate_http_get_url(&self, raw_url: &str) -> Result<reqwest::Url> {
        let parsed = reqwest::Url::parse(raw_url)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("http_get only supports http:// and https:// URLs");
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("URL must include a host");
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!("Embedded credentials are not allowed in http_get URLs");
        }

        let host = parsed
            .host_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if Self::host_is_explicitly_local(&host) {
            Self::loopback_http_get_allowed(&parsed)?;
            return Ok(parsed);
        }
        if host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".home")
            || host.ends_with(".lan")
        {
            anyhow::bail!("Local network hostnames are blocked by http_get");
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            if !Self::ip_is_public(ip) {
                anyhow::bail!("http_get cannot target private or link-local IP addresses");
            }
            return Ok(parsed);
        }

        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut resolved_any = false;
        for addr in tokio::net::lookup_host((host.as_str(), port)).await? {
            resolved_any = true;
            if !Self::ip_is_public(addr.ip()) {
                anyhow::bail!(
                    "http_get cannot target internal address {} resolved from {}",
                    addr.ip(),
                    host
                );
            }
        }
        if !resolved_any {
            anyhow::bail!("Unable to resolve host '{}'", host);
        }

        Ok(parsed)
    }

    fn collect_native_env_overrides(
        arguments: &serde_json::Value,
    ) -> Result<Vec<(String, String)>> {
        let Some(obj) = arguments.get("env").and_then(|v| v.as_object()) else {
            return Ok(Vec::new());
        };

        if obj.len() > MAX_NATIVE_ENV_OVERRIDES {
            anyhow::bail!(
                "Too many environment overrides: {} (max {})",
                obj.len(),
                MAX_NATIVE_ENV_OVERRIDES
            );
        }

        let mut out = Vec::with_capacity(obj.len());
        for (key, value) in obj {
            if key.is_empty()
                || !key
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                || key.chars().next().is_some_and(|ch| ch.is_ascii_digit())
            {
                anyhow::bail!("Invalid environment variable name '{}'", key);
            }

            let upper = key.to_ascii_uppercase();
            let blocked = matches!(
                upper.as_str(),
                "PATH"
                    | "HOME"
                    | "TMPDIR"
                    | "TMP"
                    | "TEMP"
                    | "PWD"
                    | "SHELL"
                    | "ENV"
                    | "BASH_ENV"
                    | "NODE_OPTIONS"
                    | "PYTHONPATH"
                    | "PYTHONHOME"
                    | "RUBYLIB"
                    | "RUBYOPT"
                    | "PERL5OPT"
            ) || upper.starts_with("LD_")
                || upper.starts_with("DYLD_");
            if blocked {
                anyhow::bail!("Environment override '{}' is not allowed", key);
            }

            let string_value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Environment override '{}' must be a string", key))?
                .to_string();
            if string_value.contains('\0') {
                anyhow::bail!("Environment override '{}' contains a NUL byte", key);
            }
            out.push((key.clone(), string_value));
        }

        Ok(out)
    }

    fn load_disabled_actions(path: &Path) -> HashSet<String> {
        let raw = match std::fs::read(path) {
            Ok(v) => v,
            Err(_) => return HashSet::new(),
        };
        serde_json::from_slice::<Vec<String>>(&raw)
            .map(|v| {
                v.into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn save_disabled_actions(&self) -> Result<()> {
        let mut list: Vec<String> = self.disabled_actions.read().await.iter().cloned().collect();
        list.sort();
        let raw = serde_json::to_vec_pretty(&list)?;
        tokio::fs::write(&self.disabled_actions_file, raw).await?;
        Ok(())
    }

    /// Get the data directory (parent of actions_dir)
    fn data_dir(&self) -> &Path {
        self.actions_dir.parent().unwrap_or(&self.actions_dir)
    }

    pub async fn new(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        let config_path = config_dir.join("runtime.toml");
        let config: RuntimeConfig = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            let default = RuntimeConfig::default();
            let content = toml::to_string_pretty(&default)?;
            std::fs::write(&config_path, content)?;
            default
        };

        // User skills go in data dir
        let actions_dir = data_dir.join("skills");
        std::fs::create_dir_all(&actions_dir)?;
        let cli_skills_dir = data_dir.join("cli_skills");
        std::fs::create_dir_all(&cli_skills_dir)?;
        let disabled_actions_file = data_dir.join("disabled_actions.json");
        let disabled_actions = Self::load_disabled_actions(&disabled_actions_file);

        let snapshot_dir = data_dir.join(&config.snapshot_dir);
        std::fs::create_dir_all(&snapshot_dir)?;

        let sandbox = ActionSandbox::new(&config)?;
        let transactions = TransactionManager::new(snapshot_dir);

        let runtime = Self {
            config,
            _sandbox: sandbox,
            transactions: tokio::sync::Mutex::new(transactions),
            actions: tokio::sync::RwLock::new(HashMap::new()),
            disabled_actions: tokio::sync::RwLock::new(disabled_actions),
            disabled_actions_file,
            actions_dir: actions_dir.clone(),
            cli_skills_dir,
            config_dir: config_dir.to_path_buf(),
            task_queue: None,
            action_guard: None,
            storage: None,
            mcp_registry: None,
            plugin_registry: None,
        };

        Ok(runtime)
    }

    /// Set shared task queue reference (called from Agent::init)
    pub fn set_task_queue(
        &mut self,
        tasks: std::sync::Arc<tokio::sync::RwLock<crate::core::TaskQueue>>,
    ) {
        self.task_queue = Some(tasks);
    }

    /// Set action security guard (called from Agent::init before load_all_actions)
    pub fn set_action_guard(&mut self, guard: std::sync::Arc<crate::security::ActionGuard>) {
        self.action_guard = Some(guard);
    }

    /// Set shared storage reference for expense/entity operations (called from Agent::init)
    pub fn set_storage(&mut self, storage: crate::storage::Storage) {
        self.storage = Some(storage);
    }

    /// Set MCP registry (called from Agent::init)
    pub fn set_mcp_registry(
        &mut self,
        registry: std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>,
    ) {
        self.mcp_registry = Some(registry);
    }

    /// Set plugin registry (called from Agent::init)
    pub fn set_plugin_registry(
        &mut self,
        registry: std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>,
    ) {
        self.plugin_registry = Some(registry);
    }

    /// Load all actions (builtin + bundled + user). Call AFTER set_action_guard.
    pub async fn load_all_actions(&self) -> Result<()> {
        // Load built-in actions
        self.load_builtin_actions().await?;

        // Load markdown skills from the app's skills directory (bundled with app)
        let app_skills_dir = std::path::Path::new("/app/skills");
        if app_skills_dir.exists() {
            tracing::info!("Loading bundled skills from {:?}", app_skills_dir);
            self.load_markdown_actions(app_skills_dir, ActionSource::Bundled)
                .await?;
        }

        // Also check relative skills dir (for local development).
        let local_skills_dir = std::env::current_dir()
            .map(|d| d.join("skills"))
            .unwrap_or_else(|_| std::path::PathBuf::from("./skills"));
        if local_skills_dir.exists() && local_skills_dir != app_skills_dir {
            tracing::info!("Loading local skills from {:?}", local_skills_dir);
            self.load_markdown_actions(&local_skills_dir, ActionSource::Bundled)
                .await?;
        }

        // Load user-added skills from data dir
        if self.actions_dir.exists() {
            tracing::info!("Loading user skills from {:?}", self.actions_dir);
            self.load_markdown_actions(&self.actions_dir, ActionSource::Custom)
                .await?;
        }

        if self.cli_skills_dir.exists() {
            tracing::info!(
                "Loading installed CLI skills from {:?}",
                self.cli_skills_dir
            );
            self.load_cli_skill_actions().await?;
        }

        Ok(())
    }

    /// Load built-in actions
    async fn load_builtin_actions(&self) -> Result<()> {
        // File operations
        self.register_builtin_action(ActionDef {
            name: "file_read".to_string(),
            description: "Read contents of a file".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read" }
                },
                "required": ["path"]
            }),
            capabilities: vec!["file_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_write".to_string(),
            description: "Write contents to a file".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }),
            capabilities: vec!["file_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "memory_lookup".to_string(),
            description: "Look up relevant user memory on demand. Use when the answer may depend on prior user facts, preferences, saved links/data, or knowledge base context that is not already in the recent conversation. For source-scoped external learnings such as Moltbook, set `external_sources` only when that source is directly relevant.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What memory or prior context to look up" },
                    "limit": { "type": "integer", "description": "Maximum number of memory hits to return (default: 5)" },
                    "include_semantic": { "type": "boolean", "description": "Include semantic memory matches (default: true)" },
                    "include_structured": { "type": "boolean", "description": "Include structured preferences, user data, and knowledge base context (default: true)" },
                    "include_procedures": { "type": "boolean", "description": "Include learned procedural patterns and workflow guidance (default: true)" },
                    "include_lessons": { "type": "boolean", "description": "Include learned lessons and operating constraints (default: true)" },
                    "external_sources": {
                        "type": "array",
                        "description": "Optional source-scoped external memory surfaces to include only when directly relevant, for example [\"moltbook\"]",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            capabilities: vec!["memory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // HTTP requests
        self.register_builtin_action(ActionDef {
            name: "http_get".to_string(),
            description: "Make an HTTP GET request".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "headers": { "type": "object", "description": "Optional headers" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Wasm),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_restart".to_string(),
            description: "Restart an existing deployed app from its saved metadata. Use after file_write edits to /app/data/apps/<id>/..., when a deployed app needs reload, or when the user asks to restart or re-run an existing app. Prefer app_id from app_inspect when available; otherwise use query to match an app.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to restart. Preferred when already known."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional new app title to persist before restarting. Use when a repurposed app should show a new name in the Apps list."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_stop".to_string(),
            description: "Stop the runtime for an existing deployed app without deleting its files. Use when the user asks to stop, pause, or shut down a deployed app. For repo-based multi-service deployments, `bundle_id` stops all dynamic services in that bundle and skips static ones.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to stop."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    },
                    "bundle_id": {
                        "type": "string",
                        "description": "Optional repo deployment bundle ID to stop all matching dynamic services together."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_delete".to_string(),
            description: "Stop and delete an existing deployed app, including its stored files. Use when the user asks to remove, delete, or tear down a deployed app. For repo-based multi-service deployments, `bundle_id` deletes every app in that repo bundle and cleans up the bundle metadata once the last service is gone.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to delete."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    },
                    "bundle_id": {
                        "type": "string",
                        "description": "Optional repo deployment bundle ID to delete all matching services together."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // Shell commands (requires approval by default)
        self.register_builtin_action(ActionDef {
            name: "shell".to_string(),
            description: "Execute a shell command".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to execute" },
                    "cwd": { "type": "string", "description": "Working directory" }
                },
                "required": ["command"]
            }),
            capabilities: vec!["shell".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // Clipboard
        self.register_builtin_action(ActionDef {
            name: "clipboard_read".to_string(),
            description: "Read from clipboard".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["clipboard_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "clipboard_write".to_string(),
            description: "Write to clipboard".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Content to copy" }
                },
                "required": ["content"]
            }),
            capabilities: vec!["clipboard_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "current_time".to_string(),
            description: "Return the current date and time without using any external integration. Use for date-based reminders, time checks, and internal automation scheduling logic.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "timezone": {
                        "type": "string",
                        "description": "Optional IANA timezone such as 'Asia/Kolkata' or 'America/New_York'. Defaults to UTC."
                    }
                }
            }),
            capabilities: vec!["time".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "notify_user".to_string(),
            description: "Return a notification message for internal reminder/scheduler delivery. Use for reminders and nudges that should be delivered through AgentArk's delivery routing instead of an external data source.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Notification body to deliver"
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional title for the reminder"
                    }
                },
                "required": ["message"]
            }),
            capabilities: vec!["notify".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // Scheduler
        self.register_builtin_action(ActionDef {
            name: "schedule_task".to_string(),
            description: "Schedule a recurring or one-time task. Use 'cron' for recurring (e.g., daily at 9am = '0 9 * * *') or 'at' for one-time (ISO timestamp).".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Task description - what to do" },
                    "cron": { "type": "string", "description": "Cron expression for recurring tasks. Format: 'minute hour day month weekday'. Examples: '0 9 * * *' = daily at 9am, '0 9 * * 1' = every Monday 9am, '*/30 * * * *' = every 30 minutes" },
                    "at": { "type": "string", "description": "ISO 8601 timestamp for one-time task. Example: '2026-02-06T09:00:00+05:30'" },
                    "action": { "type": "string", "description": "Optional explicit action name to run for each task occurrence" },
                    "action_arguments": { "type": "object", "description": "Optional explicit arguments for the selected action" },
                    "report_to": { "type": "string", "description": "Preferred notification channel for results" },
                    "allow_duplicate": { "type": "boolean", "description": "Create a separate task even if a matching one already exists. Default false: matching tasks are updated/reused." },
                    "validation": {
                        "type": "object",
                        "description": "Optional generic validation policy for each run",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "non_empty_result", "structured_success", "contains_text", "regex_match", "json_field_exists", "json_field_equals", "json_array_non_empty"] },
                            "text": { "type": "string" },
                            "field_path": { "type": "string" },
                            "expected": {},
                            "pattern": { "type": "string" }
                        }
                    },
                    "max_attempts": { "type": "integer", "description": "Maximum supervised retry attempts" },
                    "stall_timeout_secs": { "type": "integer", "description": "Maximum seconds a single run may take before timing out" },
                    "retry_backoff_secs": { "type": "integer", "description": "Base backoff before retrying failed runs" },
                    "automation_policy": {
                        "type": "object",
                        "description": "Advanced automation execution policy override",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "stall_timeout_secs": { "type": "integer" },
                            "retry_backoff_secs": { "type": "integer" },
                            "validation": { "type": "object" }
                        }
                    }
                },
                "required": ["task"]
            }),
            capabilities: vec!["scheduler".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Background watcher — poll an action until a condition is met, then act
        // Tunnel control for remote UI access
        self.register_builtin_action(ActionDef {
            name: "tunnel_control".to_string(),
            description: "Manage remote UI access. Use action=start to create an access URL, action=status to check the current URL, and action=stop to disable it. Optionally pass provider=cloudflare|tailscale_private|tailscale_funnel|ngrok|bore when starting.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["start", "stop", "status"], "description": "Tunnel operation" },
                    "provider": { "type": "string", "description": "Optional provider id for start: cloudflare, tailscale_private, tailscale_funnel, ngrok, or bore." },
                    "allow_duplicate": { "type": "boolean", "description": "Repeat an identical tunnel command in the same request. Default false." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;
        self.register_builtin_action(ActionDef {
            name: "watch".to_string(),
            description: "Spawn a background watcher that polls an action at regular intervals until a condition is met, then executes follow-up instructions. Use when asked to 'watch for', 'wait for', 'monitor', 'let me know when', or 'poll until'. The watcher runs autonomously and notifies the user when triggered or timed out. Default duration is 24 hours; users can extend it with timeout_hours, timeout_days, timeout_secs, or until_stopped=true.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this watcher does (shown in UI)" },
                    "poll_action": { "type": "string", "description": "Action to poll (e.g. 'gmail_scan', 'web_search', 'http_get')" },
                    "poll_arguments": { "type": "object", "description": "Arguments for the poll action" },
                    "condition_contains": { "type": "string", "description": "Trigger when result contains this keyword (case-insensitive)" },
                    "condition_matches": { "type": "string", "description": "Trigger when result matches this regex pattern" },
                    "condition_custom": { "type": "string", "description": "Natural language condition description" },
                    "on_trigger": { "type": "string", "description": "What to do when condition is met — natural language instructions for the agent" },
                    "interval_secs": { "type": "integer", "description": "Seconds between polls (default: 60)" },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to watch before giving up (default: 86400 = 24 hours)" },
                    "timeout_hours": { "type": "integer", "description": "Convenience timeout override in hours. Supports very large values." },
                    "timeout_days": { "type": "integer", "description": "Convenience timeout override in days. Supports very large values." },
                    "until_stopped": { "type": "boolean", "description": "Keep watching until the user stops it. Internally stored as a very large timeout." },
                    "notify_channel": { "type": "string", "description": "Channel to notify: 'telegram' or 'http' (default: 'telegram')" },
                    "allow_duplicate": { "type": "boolean", "description": "Create a separate watcher even if a matching one already exists. Default false: matching watchers are updated/reused." },
                    "validation": {
                        "type": "object",
                        "description": "Optional validation policy for successful poll results",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "non_empty_result", "structured_success", "contains_text", "regex_match", "json_field_exists", "json_field_equals", "json_array_non_empty"] },
                            "text": { "type": "string" },
                            "field_path": { "type": "string" },
                            "expected": {},
                            "pattern": { "type": "string" }
                        }
                    },
                    "max_attempts": { "type": "integer", "description": "Maximum supervised retry attempts for the follow-up trigger action" },
                    "stall_timeout_secs": { "type": "integer", "description": "Maximum seconds the trigger follow-up may run before timing out" },
                    "retry_backoff_secs": { "type": "integer", "description": "Base backoff before retrying failed trigger follow-ups" },
                    "automation_policy": {
                        "type": "object",
                        "description": "Advanced automation execution policy override",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "stall_timeout_secs": { "type": "integer" },
                            "retry_backoff_secs": { "type": "integer" },
                            "validation": { "type": "object" }
                        }
                    }
                },
                "required": ["description", "poll_action", "on_trigger"]
            }),
            capabilities: vec!["watcher".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "capability_acquire".to_string(),
            description: "Scaffold a reusable integration/action when the needed capability does not already exist. Generates a reviewable custom SKILL.md backed by connector_request and/or browser_auto, registers it immediately, and returns the new action plus any remaining auth/config requirements.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Action name to create in kebab-case" },
                    "description": { "type": "string", "description": "What the new capability should do" },
                    "kind": { "type": "string", "enum": ["rest_api", "oauth_api", "openapi", "web_automation"], "description": "Scaffold mode" },
                    "base_url": { "type": "string", "description": "Base URL for the provider/API" },
                    "method": { "type": "string", "enum": ["get", "post", "put", "patch", "delete"], "description": "Primary HTTP method" },
                    "path": { "type": "string", "description": "Primary path or endpoint path" },
                    "required_inputs": { "type": "array", "items": { "type": "string" }, "description": "Runtime inputs the generated action should require" },
                    "auth_type": { "type": "string", "enum": ["none", "bearer", "api_key_header", "api_key_query", "oauth2", "basic"], "description": "Primary auth strategy" },
                    "auth_secret_name": { "type": "string", "description": "Secret/config key the generated action should reference" },
                    "auth_header_name": { "type": "string", "description": "Header name for api_key_header auth" },
                    "default_headers": { "type": "object", "description": "Static default headers" },
                    "default_query": { "type": "object", "description": "Static default query params" },
                    "body_template": { "description": "Optional request body template" },
                    "pagination": { "type": "object", "description": "connector_request pagination configuration" },
                    "response_notes": { "type": "string", "description": "How the action should summarize/return results" },
                    "source_notes": { "type": "string", "description": "OpenAPI/docs notes to preserve in the scaffold" },
                    "openapi_url": { "type": "string", "description": "Optional URL to an OpenAPI/Swagger JSON document" },
                    "openapi_text": { "type": "string", "description": "Inline OpenAPI/Swagger JSON content" },
                    "docs_url": { "type": "string", "description": "Optional provider documentation URL" },
                    "docs_text": { "type": "string", "description": "Inline documentation or API notes" },
                    "force": { "type": "boolean", "description": "Force-load even if the security guard warns" },
                    "allow_duplicate": { "type": "boolean", "description": "Create another matching capability scaffold instead of updating/reusing an existing one. Default false." }
                },
                "required": ["name", "description"]
            }),
            capabilities: vec!["integration_builder".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Generic connector scaffold: pagination + rate-limit + auth-refresh + retries.
        self.register_builtin_action(ActionDef {
            name: "connector_request".to_string(),
            description: "Reusable connector scaffold for API/data collectors. Executes HTTP requests with built-in pagination, rate-limit spacing, auth-refresh callbacks, and retry/backoff behavior. Use this to build new integrations dynamically without hardcoding providers.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Target URL" },
                    "method": { "type": "string", "enum": ["get", "post", "put", "patch", "delete"], "description": "HTTP method (default: get)" },
                    "headers": { "type": "object", "description": "HTTP headers" },
                    "query": { "type": "object", "description": "Query params" },
                    "body": { "description": "Optional JSON body" },
                    "timeout_secs": { "type": "integer", "description": "Per-request timeout seconds (default: 30)" },
                    "rate_limit_ms": { "type": "integer", "description": "Min delay between requests/pages in ms" },
                    "retry": {
                        "type": "object",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "initial_backoff_ms": { "type": "integer" },
                            "max_backoff_ms": { "type": "integer" },
                            "jitter_ratio": { "type": "number" },
                            "retry_on_status": { "type": "array", "items": { "type": "integer" } }
                        }
                    },
                    "pagination": {
                        "type": "object",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "page", "cursor"] },
                            "page_param": { "type": "string" },
                            "cursor_param": { "type": "string" },
                            "items_path": { "type": "string" },
                            "next_cursor_path": { "type": "string" },
                            "start_page": { "type": "integer" },
                            "max_pages": { "type": "integer" },
                            "page_size_param": { "type": "string" },
                            "page_size": { "type": "integer" }
                        }
                    },
                    "auth_refresh": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "Action to call on auth expiry (401/403)" },
                            "arguments": { "description": "Arguments for refresh action" },
                            "retry_statuses": { "type": "array", "items": { "type": "integer" } }
                        },
                        "required": ["action"]
                    }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // First-class pipeline DAG spec compiler.
        self.register_builtin_action(ActionDef {
            name: "pipeline_compile".to_string(),
            description: "Validate and compile a pipeline DAG spec (dependency checks + topological order). Optionally persist the spec for scheduled runs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "spec": { "type": "object", "description": "Pipeline spec" },
                    "save": { "type": "boolean", "description": "Persist spec to storage (default: true)" }
                },
                "required": ["spec"]
            }),
            capabilities: vec!["orchestration".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Execute a compiled pipeline with retry/idempotency guards.
        self.register_builtin_action(ActionDef {
            name: "pipeline_run".to_string(),
            description: "Run a pipeline DAG from inline spec or saved pipeline_name. Supports retry/backoff/idempotency per node, dependency-aware execution, and persisted run traces.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pipeline_name": { "type": "string", "description": "Saved pipeline name" },
                    "spec": { "type": "object", "description": "Inline pipeline spec (overrides pipeline_name)" },
                    "dry_run": { "type": "boolean", "description": "Validate/plan without executing" },
                    "context": { "type": "object", "description": "Template context values for node args/idempotency keys" },
                    "allow_privileged": { "type": "boolean", "description": "Allow privileged node actions (default: false)" }
                }
            }),
            capabilities: vec!["orchestration".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Typed signal ranking + consensus primitive.
        self.register_builtin_action(ActionDef {
            name: "signal_consensus".to_string(),
            description: "Rank and reconcile signals using typed scoring weights and optional reviewer perspectives. Returns top prioritized signals for daily decisioning.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "signals": { "type": "array", "items": { "type": "object" }, "description": "Signals with impact/confidence/effort + payload" },
                    "weights": {
                        "type": "object",
                        "properties": {
                            "impact": { "type": "number" },
                            "confidence": { "type": "number" },
                            "effort": { "type": "number" }
                        }
                    },
                    "perspectives": { "type": "array", "items": { "type": "object" }, "description": "Optional reviewer perspectives with custom weights" },
                    "top_k": { "type": "integer", "description": "Max ranked signals to return (default: 20)" }
                },
                "required": ["signals"]
            }),
            capabilities: vec!["analytics".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Gmail scan
        self.register_builtin_action(ActionDef {
            name: "gmail_scan".to_string(),
            description: "Read and scan the user's Gmail inbox. Use when asked to check email, find emails, look for meetings/invites/receipts, or anything email-related. Supports three patterns: `recent` for the literal newest inbox emails in chronological order, `search` for exact Gmail query/filter matches, and `triage` for a smart importance scan across important, primary, recent, and starred messages. Leave mode as `auto` unless you know which behavior is needed.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["auto", "recent", "search", "triage"], "description": "How to interpret the request. `recent` returns the latest inbox emails exactly as they arrived. `search` returns exact matches for query/labels. `triage` runs the smart importance scan. `auto` picks search when query/labels are present, recent when only max_results is set, otherwise triage." },
                    "query": { "type": "string", "description": "Optional Gmail search query, for example 'from:sarah', 'subject:meeting', 'newer_than:2d', or 'label:promotions'. Best used with mode `search` or left with mode `auto` so the tool can infer search mode." },
                    "labels": { "type": "array", "items": { "type": "string" }, "description": "Optional Gmail label IDs such as INBOX, IMPORTANT, UNREAD, STARRED, SENT, DRAFT, SPAM, or TRASH. Supplying labels pushes auto mode into exact search/filter behavior." },
                    "max_results": { "type": "integer", "description": "Number of emails to return. In auto mode, setting only max_results requests the literal newest inbox emails. In search mode, it limits exact matches. In triage mode, it is ignored." }
                }
            }),
            capabilities: vec!["gmail".to_string(), "google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "gmail_reply".to_string(),
            description: "Send an email or reply via the user's Gmail. Use when asked to send, reply to, compose, or draft an email. Can reply to existing threads using thread_id.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject line" },
                    "body": { "type": "string", "description": "Email body text (plain text)" },
                    "thread_id": { "type": "string", "description": "Gmail thread ID to reply to (from gmail_scan results)" }
                },
                "required": ["to", "subject", "body"]
            }),
            capabilities: vec!["gmail".to_string(), "google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Web search
        self.register_builtin_action(ActionDef {
            name: "web_search".to_string(),
            description: "Search the web for current information. Use when asked about news, facts, prices, weather, or anything that needs up-to-date data.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "num_results": { "type": "integer", "description": "Number of results (default 5)" },
                    "backend": { "type": "string", "description": "Search backend: lightpanda, duckduckgo, playwright, brave, brave_api, serper, searxng" }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Research
        self.register_builtin_action(ActionDef {
            name: "research".to_string(),
            description: "Conduct deep research on a topic by searching and analyzing multiple sources. Use for complex questions that need thorough investigation beyond a simple web search.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Research topic or question" },
                    "max_sources": { "type": "integer", "description": "Maximum sources to examine (default 5)" },
                    "backend": { "type": "string", "description": "Optional search backend override: lightpanda, duckduckgo, playwright, brave, brave_api, serper, searxng" },
                    "depth": { "type": "string", "description": "Research depth: quick, standard, deep" },
                    "include_sources": { "type": "boolean", "description": "Include source URLs" }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Code execution sandbox
        self.register_builtin_action(ActionDef {
            name: "code_execute".to_string(),
            description: "Execute code in an isolated Docker sandbox. Supports Python, JavaScript, TypeScript, Bash, Ruby, PHP, Perl, Lua, R, Java, C, C++, Go, Rust, Swift, Kotlin, and Jupyter notebooks (.ipynb). Use when the user asks to run, execute, or test code. Dependencies like pip/npm install work automatically. For ML/data science and EDA, use language='jupyter' to create executable notebooks with visualizations — they get executed and returned as downloadable .ipynb files. When the user has attached files, pass their local paths in the 'files' array — they'll be available at /data/<filename> inside the sandbox.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": { "type": "string", "description": "Programming language: python, javascript, typescript, bash, ruby, php, perl, lua, r, java, c, cpp, go, rust, swift, kotlin, jupyter. Use 'jupyter' for EDA/ML notebooks with visualizations." },
                    "code": { "type": "string", "description": "Code to execute. For jupyter: provide valid .ipynb JSON content (notebook format). For other languages: plain code. Can include dependency installation. When files are provided, access them at /data/<filename>." },
                    "env": { "type": "object", "description": "Optional environment variables (values may include {{secret:...}} / {{env:...}} placeholders).", "additionalProperties": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Local file paths of user-attached files to inject into the sandbox at /data/. Pass the 'local_path' values from uploaded files." }
                },
                "required": ["language", "code"]
            }),
            capabilities: vec!["code_execute".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // List tasks/goals/routines
        self.register_builtin_action(ActionDef {
            name: "list_tasks".to_string(),
            description: "List pending tasks, goals, routines, and scheduled items. Use when the user asks about their pending goals, tasks, agenda, or what's scheduled.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Filter: 'all', 'pending', 'goals', 'routines', 'completed', 'failed'. Default: 'pending'" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "list_watchers".to_string(),
            description: "List background watchers and their live status, poll counts, conditions, and next poll timing. Use when the user asks what the agent is watching, which watchers are active, or whether a watcher has triggered/paused/failed.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "enum": ["active", "paused", "triggered", "failed", "timed_out", "cancelled", "all"],
                        "description": "Watcher status filter (default: active)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum watchers to return (default: 20)"
                    }
                }
            }),
            capabilities: vec!["watcher_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "goal_manage".to_string(),
            description: "Create, list, delete, or report on goals. Use when the user asks about goals, deadlines, progress toward a goal, or wants to save a new goal for later tracking.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "list", "delete", "report"],
                        "description": "Goal operation to perform"
                    },
                    "goal": {
                        "type": "string",
                        "description": "Goal description. Required for create. May also be used to delete a goal by exact text."
                    },
                    "goal_id": {
                        "type": "string",
                        "description": "Specific goal identifier for delete or report."
                    },
                    "due_date": {
                        "type": "string",
                        "description": "Optional due date for create. Accepts YYYY-MM-DD or RFC3339 timestamp."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of goals to list (default 10)."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Create another matching goal-management item instead of updating/reusing an existing one. Default false."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["goal_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Browser automation - fetch and extract content from web pages
        self.register_builtin_action(ActionDef {
            name: "browse".to_string(),
            description: "Fetch a web page and extract content. Use when asked to visit a URL, read a web page, scrape content, or check a website. Returns extracted text, links, or page title depending on the 'extract' parameter.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch (must include http:// or https://)" },
                    "extract": { "type": "string", "description": "What to extract: 'text' (default, main text content), 'links' (all hyperlinks), 'title' (page title), 'all' (text + links + title)" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Image generation
        self.register_builtin_action(ActionDef {
            name: "generate_image".to_string(),
            description: "Generate an image using AI. Use when asked to create, generate, draw, or make an image, picture, illustration, or visual.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Description of the image to generate" },
                    "negative_prompt": { "type": "string", "description": "What NOT to include (optional)" },
                    "width": { "type": "integer", "description": "Image width in pixels (default 1024)" },
                    "height": { "type": "integer", "description": "Image height in pixels (default 1024)" },
                    "style": { "type": "string", "description": "Art style (optional)" }
                },
                "required": ["prompt"]
            }),
            capabilities: vec!["image_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Action management — create/update/delete/list custom actions via chat
        self.register_builtin_action(ActionDef {
            name: "manage_actions".to_string(),
            description: "Create, update, delete, or list bundled and user-added actions/skills/workflows. Use when the user wants to inspect their installed skills, add a new action, or modify the action library.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "delete", "list"],
                        "description": "Operation to perform"
                    },
                    "name": {
                        "type": "string",
                        "description": "Action name in kebab-case (e.g. 'check-weather'). Required for create/update/delete."
                    },
                    "content": {
                        "type": "string",
                        "description": "SKILL.md content with YAML frontmatter. Required for create/update. Format:\n---\nname: action-name\ndescription: What this action does\nversion: \"1.0.0\"\n---\n\n# Action Title\n\n## Steps\n..."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Repeat an identical action-management operation in the same request. Default false."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "list_integrations".to_string(),
            description: "List registered integrations, their enablement/connectivity status, and any integration-backed tools currently available. Use when the user asks what integrations are connected, enabled, configured, or available.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "include_disabled": {
                        "type": "boolean",
                        "description": "Include integrations that are currently disabled for agent dispatch. Default true."
                    },
                    "only_connected": {
                        "type": "boolean",
                        "description": "Only show integrations that are currently connected. Default false."
                    }
                }
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // PDF generation — creates PDF documents from content
        self.register_builtin_action(ActionDef {
            name: "pdf_generate".to_string(),
            description: "Generate a PDF document. Use when asked to create a PDF, report, invoice, or document. Supports styles: report, letter, invoice, plain.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Text content for the PDF" },
                    "title": { "type": "string", "description": "Document title (optional)" },
                    "filename": { "type": "string", "description": "Output filename (default: output.pdf)" },
                    "style": { "type": "string", "enum": ["report", "letter", "invoice", "plain"], "description": "PDF style/template (default: plain)" }
                },
                "required": ["content"]
            }),
            capabilities: vec!["file_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Expense tracking — add, list, summarize, delete expenses
        self.register_builtin_action(ActionDef {
            name: "expense".to_string(),
            description: "Track expenses and spending. Actions: add (record expense), list (view expenses with optional date/category filter), summary (spending summary by category), delete (remove expense by ID). Use when the user mentions spending, costs, expenses, budget, or purchases.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "list", "summary", "delete"], "description": "Operation to perform" },
                    "amount": { "type": "number", "description": "Amount spent (for add)" },
                    "currency": { "type": "string", "description": "Currency code, e.g. USD, INR (default: USD)" },
                    "category": { "type": "string", "description": "Category: food, transport, shopping, bills, entertainment, health, education, other" },
                    "description": { "type": "string", "description": "What was purchased" },
                    "date": { "type": "string", "description": "Date (YYYY-MM-DD). Default: today" },
                    "vendor": { "type": "string", "description": "Store/vendor name (optional)" },
                    "payment_method": { "type": "string", "description": "cash, card, upi, etc. (optional)" },
                    "tags": { "type": "string", "description": "Comma-separated tags (optional)" },
                    "id": { "type": "string", "description": "Expense ID (for delete)" },
                    "from_date": { "type": "string", "description": "Start date filter (YYYY-MM-DD, for list/summary)" },
                    "to_date": { "type": "string", "description": "End date filter (YYYY-MM-DD, for list/summary)" },
                    "filter_category": { "type": "string", "description": "Category filter (for list)" }
                },
                "required": ["action"]
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Security logs — query security events from DB
        self.register_builtin_action(ActionDef {
            name: "security_logs".to_string(),
            description: "View security event logs. Shows recent security events like injection attempts, auth failures, rate limit breaches. Use when the user asks about security events, attack attempts, or system security status.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max entries to return (default: 50)" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Audio transcription
        self.register_builtin_action(ActionDef {
            name: "transcribe_audio".to_string(),
            description: "Transcribe audio/video files to text using Whisper. Use when asked to transcribe, convert speech to text, or extract text from audio/video.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to audio/video file" },
                    "language": { "type": "string", "description": "Language code (e.g. en, hi). Default: auto-detect" },
                    "model": { "type": "string", "enum": ["tiny", "base", "small", "medium", "large"], "description": "Whisper model size (default: base)" }
                },
                "required": ["file_path"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Weekly review
        self.register_builtin_action(ActionDef {
            name: "weekly_review".to_string(),
            description: "Generate a weekly review summarizing completed tasks, key conversations, and progress. Use when asked for a weekly review, weekly summary, or progress report.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "period_days": { "type": "integer", "description": "Number of days to review (default: 7)" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // === Integration-backed actions ===

        // GitHub
        self.register_builtin_action(ActionDef {
            name: "github".to_string(),
            description: "Interact with GitHub repositories, issues, and pull requests. Actions: list_repos, create_issue, list_issues, list_prs, create_pr, search. Use when the user mentions GitHub, repos, issues, pull requests, or PRs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_repos", "create_issue", "list_issues", "list_prs", "create_pr", "search"], "description": "GitHub action to perform" },
                    "owner": { "type": "string", "description": "Repository owner (username or org)" },
                    "repo": { "type": "string", "description": "Repository name" },
                    "title": { "type": "string", "description": "Issue/PR title (for create)" },
                    "body": { "type": "string", "description": "Issue/PR body (for create)" },
                    "labels": { "type": "string", "description": "Comma-separated labels (for create_issue)" },
                    "head": { "type": "string", "description": "Head branch (for create_pr)" },
                    "base": { "type": "string", "description": "Base branch (for create_pr, default: main)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "state": { "type": "string", "enum": ["open", "closed", "all"], "description": "Filter by state (for list)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Notion
        self.register_builtin_action(ActionDef {
            name: "notion".to_string(),
            description: "Interact with Notion pages, databases, and blocks. Actions: search, create_page, update_page, get_page, append_blocks. Use when the user mentions Notion, notes, wiki, or knowledge base.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "create_page", "update_page", "get_page", "append_blocks"], "description": "Notion action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "page_id": { "type": "string", "description": "Page ID (for get/update/append)" },
                    "parent_id": { "type": "string", "description": "Parent page or database ID (for create)" },
                    "title": { "type": "string", "description": "Page title (for create)" },
                    "content": { "type": "string", "description": "Page content as markdown (for create/append)" },
                    "properties": { "type": "object", "description": "Page properties to update (for update)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Twitter/X
        self.register_builtin_action(ActionDef {
            name: "twitter".to_string(),
            description: "Read tweets, search Twitter/X, view bookmarks, and get user profiles. Actions: bookmarks, list_tweets, search, get_user. Use when the user mentions Twitter, X, tweets, or bookmarks.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["bookmarks", "list_tweets", "search", "get_user"], "description": "Twitter action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "username": { "type": "string", "description": "Twitter username (for get_user, list_tweets)" },
                    "max_results": { "type": "integer", "description": "Maximum results to return (default: 10)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // 1Password
        self.register_builtin_action(ActionDef {
            name: "onepassword".to_string(),
            description: "Access 1Password vault for secure credential management. Actions: list_vaults, get_item (metadata only), search, create_item. Never exposes raw secrets to the LLM.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_vaults", "get_item", "search", "create_item"], "description": "1Password action to perform" },
                    "vault_id": { "type": "string", "description": "Vault ID (optional filter)" },
                    "item_id": { "type": "string", "description": "Item ID (for get_item)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "title": { "type": "string", "description": "Item title (for create)" },
                    "category": { "type": "string", "description": "Item category: login, password, note, etc. (for create)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Google Places
        self.register_builtin_action(ActionDef {
            name: "places".to_string(),
            description: "Search for places, find nearby locations, get place details, and get directions using Google Places/Maps. Actions: search, nearby, details, directions. Use when the user asks about restaurants, shops, locations, directions, or nearby places.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "nearby", "details", "directions"], "description": "Places action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "latitude": { "type": "number", "description": "Latitude (for nearby)" },
                    "longitude": { "type": "number", "description": "Longitude (for nearby)" },
                    "radius": { "type": "integer", "description": "Search radius in meters (for nearby, default: 1000)" },
                    "place_id": { "type": "string", "description": "Place ID (for details)" },
                    "origin": { "type": "string", "description": "Origin address (for directions)" },
                    "destination": { "type": "string", "description": "Destination address (for directions)" },
                    "type": { "type": "string", "description": "Place type filter: restaurant, cafe, hospital, atm, etc." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Twilio (Voice & SMS)
        self.register_builtin_action(ActionDef {
            name: "twilio".to_string(),
            description: "Make phone calls and send SMS messages via Twilio. Actions: call, sms, list_calls, list_messages. Use when the user wants to call someone, send a text message, or check call/message history.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["call", "sms", "list_calls", "list_messages"], "description": "Twilio action to perform" },
                    "to": { "type": "string", "description": "Phone number to call/text (E.164 format: +1234567890)" },
                    "message": { "type": "string", "description": "Message body (for sms)" },
                    "twiml": { "type": "string", "description": "TwiML instructions for the call (for call)" },
                    "limit": { "type": "integer", "description": "Number of records to return (for list, default: 20)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Ordering & Purchasing
        self.register_builtin_action(ActionDef {
            name: "ordering".to_string(),
            description: "Search products and place orders via Shopify or custom webhook. Actions: search_products, create_order, order_status, list_orders. Use when the user wants to buy, order, or shop for something.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search_products", "create_order", "order_status", "list_orders"], "description": "Ordering action to perform" },
                    "query": { "type": "string", "description": "Product search query (for search_products)" },
                    "product_id": { "type": "string", "description": "Product ID (for create_order)" },
                    "quantity": { "type": "integer", "description": "Quantity to order (default: 1)" },
                    "order_id": { "type": "string", "description": "Order ID (for order_status)" },
                    "shipping_address": { "type": "object", "description": "Shipping address (for create_order)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Browser automation — full headless browser control with human-in-the-loop
        // Curated connectors
        // Garmin
        self.register_builtin_action(ActionDef {
            name: "garmin".to_string(),
            description: "Retrieve Garmin fitness data. Actions: daily_summary, activities.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["daily_summary", "activities"], "description": "Garmin action to perform" },
                    "date": { "type": "string", "description": "Date in YYYY-MM-DD (daily_summary)" },
                    "start_date": { "type": "string", "description": "Start date in YYYY-MM-DD (activities)" },
                    "end_date": { "type": "string", "description": "End date in YYYY-MM-DD (activities)" },
                    "limit": { "type": "integer", "description": "Maximum records (default: 50)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // WHOOP
        self.register_builtin_action(ActionDef {
            name: "whoop".to_string(),
            description: "Retrieve WHOOP performance data. Actions: profile, recovery, sleep, workouts.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["profile", "recovery", "sleep", "workouts"], "description": "WHOOP action to perform" },
                    "limit": { "type": "integer", "description": "Maximum records (default: 25)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // GA4
        self.register_builtin_action(ActionDef {
            name: "ga4".to_string(),
            description: "Run GA4 Data API reports. Action: run_report.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["run_report"], "description": "GA4 action to perform" },
                    "property_id": { "type": "string", "description": "GA4 property ID" },
                    "dimensions": { "type": "array", "items": { "type": "string" }, "description": "Dimension names" },
                    "metrics": { "type": "array", "items": { "type": "string" }, "description": "Metric names" },
                    "date_ranges": {
                        "type": "array",
                        "description": "GA4 date ranges payload",
                        "items": {
                            "type": "object",
                            "properties": {
                                "startDate": { "type": "string", "description": "Start date (e.g. 7daysAgo or YYYY-MM-DD)" },
                                "endDate": { "type": "string", "description": "End date (e.g. today or YYYY-MM-DD)" }
                            }
                        }
                    },
                    "limit": { "type": "integer", "description": "Maximum rows (default: 1000)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // GSC
        self.register_builtin_action(ActionDef {
            name: "gsc".to_string(),
            description: "Query Google Search Console analytics. Action: query.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["query"], "description": "GSC action to perform" },
                    "site_url": { "type": "string", "description": "Site URL (or sc-domain value)" },
                    "start_date": { "type": "string", "description": "Start date YYYY-MM-DD" },
                    "end_date": { "type": "string", "description": "End date YYYY-MM-DD" },
                    "dimensions": { "type": "array", "items": { "type": "string" }, "description": "Query dimensions" },
                    "row_limit": { "type": "integer", "description": "Maximum rows (default: 1000)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Social analytics
        self.register_builtin_action(ActionDef {
            name: "social_analytics".to_string(),
            description: "Cross-source social publishing analytics. Action: summary. Aggregates configured sources such as Twitter and GA4.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["summary"], "description": "Social analytics action to perform" },
                    "days": { "type": "integer", "description": "Lookback window in days (default: 7)" },
                    "post_limit": { "type": "integer", "description": "Max posts to evaluate from Twitter (default: 100)" },
                    "include_twitter": { "type": "boolean", "description": "Include Twitter source (default: true)" },
                    "include_ga4": { "type": "boolean", "description": "Include GA4 source (default: true)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Moltbook (agent social network)
        self.register_builtin_action(ActionDef {
            name: "moltbook".to_string(),
            description: "Interact with Moltbook (agent social network). Actions: register, status, me, feed, search, create_post, comment, upvote_post. When the user points you at a specific submolt/community and asks you to contribute or explore, start by reading context there and then take one concrete contribution step if it is safe. Posts/comments should be original agent-authored contributions, not a verbatim rewrite of the user's instruction. Outbound posting is privacy-guarded (no user/PII/secrets).".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["register", "status", "me", "feed", "search", "create_post", "comment", "upvote_post"], "description": "Moltbook action to perform" },
                    "name": { "type": "string", "description": "Agent name (register)" },
                    "description": { "type": "string", "description": "Agent description (register)" },
                    "sort": { "type": "string", "enum": ["hot", "new", "top", "rising"], "description": "Feed sort (feed)" },
                    "limit": { "type": "integer", "description": "Max items to fetch" },
                    "query": { "type": "string", "description": "Semantic search query" },
                    "submolt": { "type": "string", "description": "Community name for post" },
                    "title": { "type": "string", "description": "Post title" },
                    "content": { "type": "string", "description": "Post/comment content" },
                    "post_id": { "type": "string", "description": "Post ID for comment/upvote" },
                    "parent_id": { "type": "string", "description": "Parent comment ID for threaded reply" },
                    "allow_duplicate": { "type": "boolean", "description": "Repeat an identical Moltbook action in the same request. Default false." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "browser_auto".to_string(),
            description: "Automate browser tasks: navigate websites, fill forms, click buttons, take screenshots, read page content. Use when asked to 'go to a website', 'log into', 'fill out a form', 'book', 'check my account', or any web browsing/automation task. When stuck (CAPTCHA, 2FA, ambiguous UI), takes a screenshot and asks the user for help, then continues. Runs as a background session.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["start_session", "navigate", "click", "type_text", "screenshot", "get_content", "scroll", "press_key", "ask_user", "end_session"],
                        "description": "Browser action to perform. Use start_session first, then chain actions."
                    },
                    "task": { "type": "string", "description": "High-level description of what to accomplish (for start_session)" },
                    "url": { "type": "string", "description": "URL to navigate to" },
                    "session_id": { "type": "string", "description": "Browser session ID (required after start_session)" },
                    "selector": { "type": "string", "description": "CSS selector for click/type target" },
                    "text": { "type": "string", "description": "Text to type or link text to click" },
                    "x": { "type": "integer", "description": "X coordinate for click" },
                    "y": { "type": "integer", "description": "Y coordinate for click" },
                    "clear": { "type": "boolean", "description": "Clear field before typing (default: false)" },
                    "direction": { "type": "string", "enum": ["up", "down"], "description": "Scroll direction" },
                    "key": { "type": "string", "description": "Key to press (Enter, Tab, Escape, etc.)" },
                    "question": { "type": "string", "description": "Question to ask the user (for ask_user)" },
                    "channel": { "type": "string", "description": "Channel to notify on (telegram, whatsapp, web)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Google Calendar — list, create, find free time
        self.register_builtin_action(ActionDef {
            name: "calendar_today".to_string(),
            description: "List today's calendar events. Use when the user asks 'what's on my calendar today', 'do I have any meetings', etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_list".to_string(),
            description: "List calendar events in a date range. Use when asked about upcoming events, schedule for a specific date, etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start datetime (ISO 8601). Defaults to now." },
                    "end": { "type": "string", "description": "End datetime (ISO 8601). Defaults to 7 days from now." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_create".to_string(),
            description: "Create a new calendar event. Use when asked to schedule a meeting, add an event, block time, etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "Event title" },
                    "start": { "type": "string", "description": "Start datetime (ISO 8601)" },
                    "end": { "type": "string", "description": "End datetime (ISO 8601)" },
                    "description": { "type": "string", "description": "Event description/notes" },
                    "location": { "type": "string", "description": "Event location" },
                    "attendees": { "type": "array", "items": { "type": "string" }, "description": "List of attendee email addresses" }
                },
                "required": ["summary", "start", "end"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_free".to_string(),
            description: "Find free time slots in the calendar. Use when asked 'when am I free', 'find time for a meeting', etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start of range (ISO 8601). Defaults to now." },
                    "end": { "type": "string", "description": "End of range (ISO 8601). Defaults to end of today." },
                    "min_duration_minutes": { "type": "integer", "description": "Minimum free slot duration in minutes (default: 30)" }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_drive_search".to_string(),
            description: "Search Google Drive files using the connected Google Workspace account with read-only Drive access. Use when the user asks to find a file, document, folder, spreadsheet, or deck in Drive.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Optional Google Drive query, such as name contains 'roadmap' or mimeType='application/vnd.google-apps.spreadsheet'." },
                    "page_size": { "type": "integer", "description": "Max number of files to return (default 10)." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_docs_read".to_string(),
            description: "Read the text content of a Google Doc by document ID. Use when the user provides a Google Doc link or ID and wants the content summarized or inspected.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "document_id": { "type": "string", "description": "Google Doc document ID." }
                },
                "required": ["document_id"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_sheets_read".to_string(),
            description: "Read a range from Google Sheets. Use when the user provides a spreadsheet ID and range or asks for values from a connected Google Sheet.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "spreadsheet_id": { "type": "string", "description": "Google Sheets spreadsheet ID." },
                    "range": { "type": "string", "description": "A1 range notation, such as Sheet1!A1:D20." }
                },
                "required": ["spreadsheet_id", "range"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_chat_list_spaces".to_string(),
            description: "List the Google Chat spaces visible to the connected Google Workspace account with read-only Chat access.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "page_size": { "type": "integer", "description": "Max number of spaces to return (default 20)." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_admin_list_users".to_string(),
            description: "List Google Workspace users from the Admin Directory with read-only directory access. Use when the user asks about Workspace users, seats, or directory accounts.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "customer": { "type": "string", "description": "Optional Google customer ID. Defaults to my_customer." },
                    "domain": { "type": "string", "description": "Optional domain filter if customer is not provided." },
                    "max_results": { "type": "integer", "description": "Max number of users to return (default 20)." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_help".to_string(),
            description: "Show Google Workspace CLI help output. Use when you need to discover available gws commands or inspect help for a specific Google Workspace service. Pass argv as the command parts after `gws`, for example [\"drive\",\"--help\"] or [\"gmail\",\"users\",\"messages\",\"list\",\"--help\"].".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional arguments after `gws`. Leave empty for top-level help."
                    }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_schema".to_string(),
            description: "Inspect the request and response schema for any Google Workspace CLI method. Use when you need the exact shape for a gws command before executing it. Example target: drive.files.list or gmail.users.messages.list.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "The gws schema target, such as drive.files.list or sheets.spreadsheets.values.get."
                    }
                },
                "required": ["target"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_skills".to_string(),
            description: "List or read the generated Google Workspace CLI skill docs that ship with gws, including service skills, helper skills, recipes, and personas. Use this when you want exact gws examples for Docs, Drive, Sheets, Gmail, Calendar, Chat, Admin, or other Workspace services before calling google_workspace_gws_schema or google_workspace_gws_command. If name is provided, returns the full SKILL.md content for that skill.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Optional generated gws skill name to open, for example gws-docs, gws-gmail-triage, recipe-label-and-archive-emails, or persona-team-lead."
                    },
                    "filter": {
                        "type": "string",
                        "description": "Optional text filter to narrow the catalog by skill name, description, or cli help."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of catalog entries to list when name is omitted. Default: 80."
                    }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_command".to_string(),
            description: "Execute any non-auth Google Workspace CLI command against the connected Google Workspace account. Use this when you need broader Workspace API coverage than the built-in Gmail/Calendar/Drive helpers provide. Actual behavior is still limited by the granted OAuth scopes: Gmail supports read/send, Calendar supports calendar access, and the current Drive/Docs/Sheets/Chat/Admin bundles are read-only. Prefer google_workspace_gws_skills for examples and google_workspace_gws_schema for exact method shapes before executing unfamiliar commands. Provide argv as the command parts after `gws`, for example [\"drive\",\"files\",\"list\",\"--params\",\"{\\\"pageSize\\\":5}\"] , [\"calendar\",\"+agenda\"], or [\"gmail\",\"users\",\"messages\",\"list\",\"--params\",\"{\\\"maxResults\\\":5,\\\"labelIds\\\":[\\\"INBOX\\\"]}\"] . Set required_bundles when you know which Workspace bundles this command needs, such as [\"drive\"] or [\"gmail\",\"calendar\"].".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Arguments after `gws`. Do not include the `gws` binary itself."
                    },
                    "required_bundles": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional Workspace bundles needed by the command, such as [\"drive\"] or [\"gmail\",\"calendar\"]."
                    }
                },
                "required": ["argv"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // SSH — remote server execution (behind feature flag)
        #[cfg(feature = "ssh")]
        {
            self.register_builtin_action(ActionDef {
                name: "ssh".to_string(),
                description: "Execute a command on a configured remote server via SSH. Use when asked to check server status, deploy, manage services, run remote commands, or anything involving a remote server.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "Name of the SSH connection to use (from configured connections)" },
                        "command": { "type": "string", "description": "Shell command to execute on the remote server" }
                    },
                    "required": ["connection", "command"]
                }),
                capabilities: vec!["network".to_string(), "ssh".to_string()],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
            }).await;

            self.register_builtin_action(ActionDef {
                name: "ssh_connections".to_string(),
                description: "List available SSH connections. Use before ssh to know which servers are configured.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
                capabilities: vec![],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
            }).await;
        }

        self.register_builtin_action(ActionDef {
            name: "app_inspect".to_string(),
            description: "Inspect existing deployed apps. Use when the user asks which apps are deployed, refers to a deployed app by name/ID, wants to inspect a repo deployment bundle, or wants to debug, diagnose, fix, or update an existing app. Returns matched app metadata, its /app/data/apps/<id> directory, repo bundle metadata when present, runtime state, and recent logs so you can use file_read/file_write/app_restart/app_stop/app_delete/http_get on the live app.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match. Leave empty to list deployed apps."
                    },
                    "bundle_id": {
                        "type": "string",
                        "description": "Optional repo deployment bundle ID to inspect all services from a repo-based deployment together."
                    },
                    "include_files": {
                        "type": "boolean",
                        "description": "Include a file inventory for the matched app. Default: true."
                    },
                    "include_logs": {
                        "type": "boolean",
                        "description": "Include recent runtime log tail for the matched app when available. Default: true."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of apps to list in the summary. Default: 10."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string(), "file_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // App deployment — write files, start servers, return live URL
        self.register_builtin_action(ActionDef {
            name: "app_deploy".to_string(),
            description: "Deploy a web app or server and return a live URL. Supports either generated files OR a repository source. Use when asked to build a dashboard, create a tool, make a website, build an app, or deploy/run a repo locally for the user. For file-based apps, provide a `files` object. For repo-based apps, provide `repo_url` (and optionally `repo_ref`, `repo_subdir`, `service_mode`) so AgentArk can clone the repo, inspect the README/manifests, stand up the detected frontend/backend services, and return managed endpoints. For dynamic apps (Python/Node servers), include or infer entry_command. Repo-based deploys default to container runtime unless overridden. Public exposure defaults to enabled (expose_public=true) so the agent can return a tunnel-ready link. Declare required inputs via required_inputs and mark each item sensitive=true/false. Access guard is optional and defaults to false.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "object",
                        "description": "Object mapping filename to file content. e.g. {\"index.html\": \"<html>...\", \"style.css\": \"body{...}\", \"app.py\": \"from fastapi import...\"}"
                    },
                    "repo_url": {
                        "type": "string",
                        "description": "Public Git repository URL to clone and deploy, e.g. https://github.com/org/repo. Use this instead of `files` when the user wants AgentArk to run an existing repo locally."
                    },
                    "repo_ref": {
                        "type": "string",
                        "description": "Optional branch, tag, or commit-ish to check out after cloning the repo."
                    },
                    "repo_subdir": {
                        "type": "string",
                        "description": "Optional subdirectory inside the cloned repo to treat as the deployment root."
                    },
                    "service_mode": {
                        "type": "string",
                        "enum": ["auto", "frontend", "backend", "fullstack"],
                        "description": "For repo deploys, choose which service(s) to stand up. auto deploys the detected default services."
                    },
                    "title": { "type": "string", "description": "App name/title (default: App)" },
                    "entry_command": {
                        "type": "string",
                        "description": "Command to start the server process (omit for static HTML apps). Use {PORT} placeholder or PORT env var for the port. Python apps auto-activate their venv. Examples: 'python3 app.py', 'node server.js', 'uvicorn app:app --host 0.0.0.0 --port {PORT}'"
                    },
                    "install_command": {
                        "type": "string",
                        "description": "Command to install dependencies before starting (optional). Omit for Python apps with requirements.txt — a venv is auto-created. Each app runs in its own isolated environment (Python venv or local node_modules). Examples: 'pip install -r requirements.txt', 'npm install'"
                    },
                    "required_inputs": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "key": { "type": "string" },
                                        "sensitive": { "type": "boolean" }
                                    },
                                    "required": ["key"]
                                }
                            ]
                        },
                        "description": "Required runtime inputs. String entries default to sensitive=true. Use object entries for per-key sensitivity, e.g. [{\"key\":\"API_TOKEN\",\"sensitive\":true},{\"key\":\"BASE_URL\",\"sensitive\":false}]"
                    },
                    "required_secrets": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Compatibility alias for sensitive required inputs."
                    },
                    "required_config": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Compatibility alias for non-sensitive required inputs."
                    },
                    "required_env": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Legacy alias for required_secrets."
                    },
                    "config": {
                        "type": "object",
                        "additionalProperties": { "type": ["string", "number", "boolean"] },
                        "description": "Optional non-sensitive runtime config values (e.g. BASE_URL). Values are stored in app metadata for restart/restore."
                    },
                    "runtime_image": {
                        "type": "string",
                        "description": "Optional container image used to run the app (default: agentark-sandbox:latest)"
                    },
                    "runtime_preference": {
                        "type": "string",
                        "enum": ["local", "container"],
                        "description": "Preferred runtime for dynamic apps. Default: container when Docker is configured for AgentArk, otherwise local."
                    },
                    "expose_public": {
                        "type": "boolean",
                        "description": "Whether to expose the app on the configured remote-access provider when available. Default: true."
                    },
                    "access_guard": {
                        "type": "boolean",
                        "description": "Enable access-key guard for the shared app URL. Default: false."
                    },
                    "replace_existing": {
                        "type": "boolean",
                        "description": "Force recreation even if a matching deployed app already exists. Default: false."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Create another matching app deployment instead of reusing/updating a matching existing app. Default false."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // NOTE: Remotion video_generate action disabled.
        // Limitations that need addressing before re-enabling:
        //   - No TTS/speech integration — videos are visual-only
        //   - No video player in chat UI — only download links
        //   - No validation of LLM-generated React/Remotion code
        //   - 141MB image size overhead for Remotion template + node_modules
        //   - Requires Chromium for rendering (already in Playwright image)
        // To re-enable: uncomment this block and the Remotion Dockerfile sections.
        // self.register_builtin_action(ActionDef {
        //     name: "video_generate".to_string(),
        //     ...
        // }).await;

        // Provider-based text/image-to-video generation (Runway/Luma/Fal/Veo/etc.)
        self.register_builtin_action(ActionDef {
            name: "generate_video".to_string(),
            description: "Generate a normal AI video via configured video providers (Runway, Luma, Fal, Sora, Veo, etc.). Use for general text-to-video or image-to-video requests when no custom Remotion coding is needed. If the user specifically asks for product showcase/scripted animation with custom scenes, use video_generate instead. If unclear, ask which mode they want.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Video prompt/description" },
                    "image_url": { "type": "string", "description": "Optional source image URL for image-to-video models" },
                    "duration_seconds": { "type": "integer", "minimum": 1, "maximum": 12, "description": "Desired duration in seconds (model-dependent; default 4)" },
                    "aspect_ratio": { "type": "string", "description": "Optional aspect ratio (e.g. 16:9, 9:16, 1:1)" },
                    "model": { "type": "string", "description": "Optional provider model override" },
                    "provider": { "type": "string", "description": "Optional provider override (replicate, runway, luma, fal, openai_sora, google_veo, etc.)" }
                },
                "required": ["prompt"]
            }),
            capabilities: vec!["video_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Self-evolve - policy-first self-improvement
        self.register_builtin_action(ActionDef {
            name: "self_evolve".to_string(),
            description: "Evolve AgentArk behavior with an auditable promotion loop. Default mode is policy/strategy evolution (benchmark, lineage archive, statistical gating, canary rollout with replay gate, optional promotion). Code evolution is disabled by default and requires explicit allow_code_writes=true.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "Natural language description of what should evolve"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["policy", "code"],
                        "description": "Evolution mode. policy (default) evolves runtime strategy; code enables source mutation mode."
                    },
                    "allow_code_writes": {
                        "type": "boolean",
                        "description": "Required true to run code mode. Ignored for policy mode."
                    },
                    "apply_promotion": {
                        "type": "boolean",
                        "description": "For policy mode: apply promoted policy by activating canary rollout and replay gate. Default true."
                    },
                    "canary_rollout_percent": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Traffic percentage for candidate policy during canary rollout. Default 20."
                    },
                    "canary_min_samples_per_version": {
                        "type": "integer",
                        "minimum": 5,
                        "description": "Minimum baseline/candidate samples required for replay promotion. Default 25."
                    },
                    "canary_min_success_gain": {
                        "type": "number",
                        "minimum": 0.0,
                        "description": "Minimum success-rate improvement required for promotion. Default 0.03."
                    },
                    "canary_max_sign_test_p_value": {
                        "type": "number",
                        "minimum": 0.0001,
                        "maximum": 1.0,
                        "description": "Maximum one-sided sign-test p-value for promotion. Default 0.10."
                    },
                    "replay_log_limit": {
                        "type": "integer",
                        "minimum": 100,
                        "description": "Operational log window size used for replay evaluation. Default 4000."
                    }
                },
                "required": ["request"]
            }),
            capabilities: vec!["self_evolve".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        Ok(())
    }

    async fn register_builtin_action(&self, info: ActionDef) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
            },
        );
    }

    /// Register an action with workflow content (from SKILL.md or legacy ACTION.md)
    async fn register_workflow_action(&self, info: ActionDef, workflow: String) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: Some(workflow),
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
            },
        );
    }

    async fn register_cli_action(&self, info: ActionDef, binding: CliToolBinding) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                cli_binding: Some(binding),
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
            },
        );
    }

    /// Register an MCP-backed action (external tool/resource)
    pub async fn register_mcp_action(&self, info: ActionDef, binding: McpBinding) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: Some(binding),
                plugin_binding: None,
                custom_api_binding: None,
            },
        );
    }

    /// Register a plugin-backed action
    pub async fn register_plugin_action(&self, info: ActionDef, binding: PluginBinding) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: Some(binding),
                custom_api_binding: None,
            },
        );
    }

    /// Register an imported custom API action.
    pub async fn register_custom_api_action(&self, info: ActionDef, binding: CustomApiBinding) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: Some(binding),
            },
        );
    }

    fn build_cli_action_input_schema(action_name: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "description": format!("Argument list to pass to {}. Do not include the executable name itself.", action_name),
                    "items": { "type": "string" }
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory. Must stay within allowed workspace/data roots."
                },
                "stdin": {
                    "type": "string",
                    "description": "Optional text to pipe to stdin."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 300,
                    "description": "Optional timeout in seconds. Default 60."
                }
            },
            "required": ["args"]
        })
    }

    fn build_cli_action_def(manifest: &InstalledCliSkillManifest, skill_path: &Path) -> ActionDef {
        ActionDef {
            name: manifest.name.clone(),
            description: format!(
                "{} Use this action to call the verified local CLI directly. Pass exact argv items in `args`, and use `--help` whenever syntax is unclear.",
                manifest.description.trim()
            ),
            version: manifest.version.clone(),
            input_schema: Self::build_cli_action_input_schema(&manifest.name),
            capabilities: vec!["local_cli".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::Custom,
            file_path: Some(skill_path.to_string_lossy().to_string()),
        }
    }

    pub async fn install_cli_skill_action(
        &self,
        manifest: InstalledCliSkillManifest,
        skill_markdown: &str,
    ) -> Result<()> {
        let skill_name = manifest.name.trim();
        if skill_name.is_empty() {
            anyhow::bail!("CLI skill name cannot be empty");
        }

        {
            let actions = self.actions.read().await;
            if let Some(existing) = actions.get(skill_name) {
                if existing.info.source == ActionSource::System {
                    anyhow::bail!(
                        "Cannot install CLI skill '{}': a built-in action with that name already exists",
                        skill_name
                    );
                }
                if existing.cli_binding.is_none() && existing.workflow_content.is_some() {
                    anyhow::bail!(
                        "Cannot install CLI skill '{}': a markdown workflow skill with that name already exists",
                        skill_name
                    );
                }
            }
        }

        let skill_dir = self.cli_skills_dir.join(skill_name);
        tokio::fs::create_dir_all(&skill_dir).await?;
        let skill_path = skill_dir.join("SKILL.md");
        let manifest_path = skill_dir.join("manifest.json");

        tokio::fs::write(&skill_path, skill_markdown).await?;
        tokio::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?).await?;

        let info = Self::build_cli_action_def(&manifest, &skill_path);
        let binding = CliToolBinding {
            executable_path: manifest.executable_path.clone(),
            verify_args: manifest.verify_args.clone(),
        };
        self.register_cli_action(info, binding).await;
        tracing::info!(
            "Installed CLI skill '{}' backed by {}",
            skill_name,
            manifest.executable_path
        );
        Ok(())
    }

    async fn load_cli_skill_actions(&self) -> Result<()> {
        let entries = match std::fs::read_dir(&self.cli_skills_dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    "Could not read CLI skills directory {:?}: {}",
                    self.cli_skills_dir,
                    e
                );
                return Ok(());
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("manifest.json");
            let skill_path = path.join("SKILL.md");
            if !manifest_path.exists() || !skill_path.exists() {
                continue;
            }

            let manifest = match tokio::fs::read_to_string(&manifest_path).await {
                Ok(raw) => match serde_json::from_str::<InstalledCliSkillManifest>(&raw) {
                    Ok(manifest) => manifest,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse CLI skill manifest {:?}: {}",
                            manifest_path,
                            e
                        );
                        continue;
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        "Failed to read CLI skill manifest {:?}: {}",
                        manifest_path,
                        e
                    );
                    continue;
                }
            };

            let info = Self::build_cli_action_def(&manifest, &skill_path);
            let binding = CliToolBinding {
                executable_path: manifest.executable_path.clone(),
                verify_args: manifest.verify_args.clone(),
            };
            self.register_cli_action(info, binding).await;
        }

        Ok(())
    }

    /// Remove all MCP-backed actions
    pub async fn unregister_mcp_actions(&self) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| a.mcp_binding.is_none());
        before.saturating_sub(actions.len())
    }

    /// Remove MCP-backed actions for a specific server
    pub async fn unregister_mcp_actions_for_server(&self, server_id: &str) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| {
            if let Some(binding) = &a.mcp_binding {
                binding.server_id != server_id
            } else {
                true
            }
        });
        before.saturating_sub(actions.len())
    }

    /// Remove all plugin-backed actions
    pub async fn unregister_plugin_actions(&self) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| a.plugin_binding.is_none());
        before.saturating_sub(actions.len())
    }

    /// Remove plugin-backed actions for a specific plugin
    pub async fn unregister_plugin_actions_for_plugin(&self, plugin_id: &str) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| {
            if let Some(binding) = &a.plugin_binding {
                binding.plugin_id != plugin_id
            } else {
                true
            }
        });
        before.saturating_sub(actions.len())
    }

    /// Remove all imported custom API actions.
    pub async fn unregister_custom_api_actions(&self) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| a.custom_api_binding.is_none());
        before.saturating_sub(actions.len())
    }

    fn resolve_optional_cli_cwd(&self, raw: Option<&str>) -> Result<Option<PathBuf>> {
        let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let candidate = self.absolutize_tool_path(raw)?;
        let resolved = candidate.canonicalize().with_context(|| {
            format!(
                "CLI working directory '{}' does not exist",
                candidate.display()
            )
        })?;
        self.ensure_tool_path_allowed(&resolved)?;
        Ok(Some(resolved))
    }

    async fn execute_cli_action(
        &self,
        binding: CliToolBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let executable = binding.executable_path.trim();
        if executable.is_empty() {
            anyhow::bail!("CLI executable path is empty");
        }

        let args = arguments
            .get("args")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing 'args' array"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("CLI args must be strings"))
            })
            .collect::<Result<Vec<_>>>()?;
        let stdin_text = arguments
            .get("stdin")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let timeout_secs = arguments
            .get("timeout_secs")
            .and_then(|value| value.as_u64())
            .unwrap_or(60)
            .clamp(1, 300);
        let cwd =
            self.resolve_optional_cli_cwd(arguments.get("cwd").and_then(|value| value.as_str()))?;

        let mut command = tokio::process::Command::new(executable);
        command.args(&args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        if stdin_text.is_some() {
            command.stdin(std::process::Stdio::piped());
        }
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let mut child = command.spawn().with_context(|| {
            format!(
                "Failed to launch CLI executable '{}'",
                binding.executable_path
            )
        })?;

        if let Some(stdin_text) = stdin_text {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(stdin_text.as_bytes()).await?;
            }
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("CLI command timed out after {}s", timeout_secs))??;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let mut combined = String::new();
        if !stdout.is_empty() {
            combined.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n\nstderr:\n");
            } else {
                combined.push_str("stderr:\n");
            }
            combined.push_str(&stderr);
        }
        if combined.is_empty() {
            combined = "(no output)".to_string();
        }

        if output.status.success() {
            Ok(combined)
        } else {
            Err(anyhow::anyhow!(
                "CLI command exited with status {}. {}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                combined
            ))
        }
    }

    /// Execute an action with given arguments
    pub async fn execute_action(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let (sandbox_mode, cli_binding, mcp_binding, plugin_binding, custom_api_binding, source) = {
            let actions = self.actions.read().await;
            let action = actions
                .get(action_name)
                .ok_or_else(|| anyhow::anyhow!("Unknown action: {}", action_name))?;
            (
                action
                    .info
                    .sandbox_mode
                    .clone()
                    .unwrap_or(self.config.default_sandbox.clone()),
                action.cli_binding.clone(),
                action.mcp_binding.clone(),
                action.plugin_binding.clone(),
                action.custom_api_binding.clone(),
                action.info.source.clone(),
            )
        };

        if source != ActionSource::System {
            let disabled = self.disabled_actions.read().await;
            if disabled.contains(action_name) {
                return Err(anyhow::anyhow!(
                    "Action '{}' is disabled. Re-enable it in the UI before running.",
                    action_name
                ));
            }
        } else if !self.is_builtin_integration_action_enabled(action_name) {
            let integration_id = Self::builtin_integrations_for_action(action_name)
                .first()
                .copied()
                .unwrap_or("required");
            return Err(anyhow::anyhow!(
                "Action '{}' is unavailable because integration '{}' is disabled.",
                action_name,
                integration_id
            ));
        }

        // Resolve secrets at execution time so they never appear in LLM-visible
        // tool-call arguments or execution traces.
        let resolved_args = self.resolve_secret_placeholders(action_name, arguments)?;

        if let Some(binding) = cli_binding {
            return self.execute_cli_action(binding, &resolved_args).await;
        }

        if let Some(binding) = mcp_binding {
            return self.execute_mcp_action(binding, &resolved_args).await;
        }

        if let Some(binding) = plugin_binding {
            return self.execute_plugin_action(binding, &resolved_args).await;
        }

        if let Some(binding) = custom_api_binding {
            return self
                .execute_custom_api_action(binding, &resolved_args)
                .await;
        }

        // Start transaction if rollback is enabled
        let transaction = if self.config.enable_rollback {
            let mut tx_guard = self.transactions.lock().await;
            Some(tx_guard.begin().await?)
        } else {
            None
        };

        // Execute based on sandbox mode
        let result = match sandbox_mode {
            SandboxMode::Native => self.execute_native(action_name, &resolved_args).await,
            SandboxMode::Wasm => self.execute_wasm(action_name, &resolved_args).await,
            SandboxMode::Docker => self.execute_docker(action_name, &resolved_args).await,
        };

        // Handle transaction
        match (&result, transaction) {
            (Ok(_), Some(tx)) => {
                let mut tx_guard = self.transactions.lock().await;
                tx_guard.commit(tx).await?;
            }
            (Err(_), Some(tx)) => {
                tracing::warn!("Rolling back transaction due to error");
                let mut tx_guard = self.transactions.lock().await;
                tx_guard.rollback(tx).await?;
            }
            _ => {}
        }

        result
    }

    /// Resolve secret placeholders inside action arguments.
    ///
    /// Supported syntax:
    /// - `{{secret:KEY}}` looks up an encrypted custom secret:
    ///   - `secret:KEY` (preferred)
    ///   - `env:KEY` (compat)
    /// - `{{env:ENV_NAME}}` resolves ENV_NAME using an optional per-action binding:
    ///   - binding key: `action_envmap:{action}:{ENV_NAME}` -> {target}
    ///   - if target == "builtin", uses the agent's configured provider key(s) where applicable
    ///   - else looks up `env:{target}` in encrypted custom secrets
    ///
    /// NOTE: Returns the resolved arguments, but does not mutate the original `arguments`,
    /// so traces / tool calls remain safe.
    pub fn resolve_secret_placeholders(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mgr = SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let secrets = mgr.load_secrets()?;
        let config = mgr.load().ok();

        fn builtin_env_from_config(cfg: &AgentConfig, env: &str) -> Option<String> {
            let mut providers: Vec<&crate::core::LlmProvider> = vec![&cfg.llm];
            if let Some(fb) = cfg.llm_fallback.as_ref() {
                providers.push(fb);
            }
            for slot in &cfg.model_pool.slots {
                if slot.enabled {
                    providers.push(&slot.provider);
                }
            }

            match env {
                "OPENAI_API_KEY" => providers.into_iter().find_map(|p| match p {
                    crate::core::LlmProvider::OpenAI { api_key, .. } if !api_key.is_empty() => {
                        Some(api_key.clone())
                    }
                    _ => None,
                }),
                "OPENROUTER_API_KEY" => providers
                    .into_iter()
                    .find_map(|p| match p {
                        crate::core::LlmProvider::OpenAI {
                            api_key, base_url, ..
                        } => {
                            if !api_key.is_empty()
                                && base_url.as_deref().unwrap_or("").contains("openrouter")
                            {
                                Some(api_key.clone())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    })
                    .or_else(|| builtin_env_from_config(cfg, "OPENAI_API_KEY")),
                "ANTHROPIC_API_KEY" => providers.into_iter().find_map(|p| match p {
                    crate::core::LlmProvider::Anthropic { api_key, .. } if !api_key.is_empty() => {
                        Some(api_key.clone())
                    }
                    _ => None,
                }),
                _ => None,
            }
        }

        fn legacy_env_alias_lookup(
            custom: &std::collections::HashMap<String, String>,
            env: &str,
        ) -> Option<String> {
            // Compatibility: existing integrations store provider tokens under non-env keys.
            let legacy_key = match env {
                "GITHUB_TOKEN" => Some("github_token"),
                "NOTION_TOKEN" => Some("notion_token"),
                "TWITTER_BEARER_TOKEN" => Some("twitter_bearer_token"),
                "ONEPASSWORD_TOKEN" => Some("onepassword_token"),
                "GOOGLE_PLACES_API_KEY" => Some("google_places_api_key"),
                "TWILIO_AUTH_TOKEN" => Some("twilio_auth_token"),
                "TWILIO_ACCOUNT_SID" => Some("twilio_account_sid"),
                "GARMIN_TOKEN" => Some("garmin_token"),
                "GARMIN_API_BASE" => Some("garmin_api_base"),
                "WHOOP_TOKEN" => Some("whoop_token"),
                "GA4_ACCESS_TOKEN" => Some("ga4_access_token"),
                "GA4_PROPERTY_ID" => Some("ga4_property_id"),
                "GSC_ACCESS_TOKEN" => Some("gsc_access_token"),
                "GSC_SITE_URL" => Some("gsc_site_url"),
                "SOCIAL_TWITTER_BEARER_TOKEN" => Some("social_twitter_bearer_token"),
                "SOCIAL_GA4_ACCESS_TOKEN" => Some("social_ga4_access_token"),
                "SOCIAL_GA4_PROPERTY_ID" => Some("social_ga4_property_id"),
                _ => None,
            }?;
            custom.get(legacy_key).cloned()
        }

        let re = regex::Regex::new(r"\{\{\s*(secret|env)\s*:\s*([A-Za-z0-9_\-:.]+)\s*\}\}")
            .expect("valid placeholder regex");
        let custom = &secrets.custom;

        let resolve_secret = |key: &str| -> Option<String> {
            custom
                .get(&format!("secret:{}", key))
                .cloned()
                .or_else(|| custom.get(&format!("env:{}", key)).cloned())
                .or_else(|| {
                    config
                        .as_ref()
                        .and_then(|cfg| builtin_env_from_config(cfg, key))
                })
        };

        let resolve_env = |env: &str| -> Option<String> {
            let binding_key = format!("action_envmap:{}:{}", action_name, env);
            let target = custom
                .get(&binding_key)
                .cloned()
                .unwrap_or_else(|| env.to_string());

            if target == "builtin" {
                return config
                    .as_ref()
                    .and_then(|cfg| builtin_env_from_config(cfg, env));
            }

            custom
                .get(&format!("env:{}", target))
                .cloned()
                .or_else(|| custom.get(&format!("secret:{}", target)).cloned())
                .or_else(|| legacy_env_alias_lookup(custom, env))
                .or_else(|| {
                    config
                        .as_ref()
                        .and_then(|cfg| builtin_env_from_config(cfg, env))
                })
        };

        fn substitute_in_str(
            s: &str,
            re: &regex::Regex,
            action_name: &str,
            resolve_secret: &impl Fn(&str) -> Option<String>,
            resolve_env: &impl Fn(&str) -> Option<String>,
        ) -> Result<String> {
            let mut out = String::with_capacity(s.len());
            let mut last = 0usize;
            for caps in re.captures_iter(s) {
                let m = caps.get(0).unwrap();
                out.push_str(&s[last..m.start()]);
                let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let key = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let val = match kind {
                    "secret" => resolve_secret(key),
                    "env" => resolve_env(key),
                    _ => None,
                }
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing secret/env '{}:{}' for action '{}'",
                        kind,
                        key,
                        action_name
                    )
                })?;
                out.push_str(&val);
                last = m.end();
            }
            out.push_str(&s[last..]);
            Ok(out)
        }

        fn walk(
            v: &serde_json::Value,
            re: &regex::Regex,
            action_name: &str,
            resolve_secret: &impl Fn(&str) -> Option<String>,
            resolve_env: &impl Fn(&str) -> Option<String>,
        ) -> Result<serde_json::Value> {
            Ok(match v {
                serde_json::Value::String(s) => serde_json::Value::String(substitute_in_str(
                    s,
                    re,
                    action_name,
                    resolve_secret,
                    resolve_env,
                )?),
                serde_json::Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for item in arr {
                        out.push(walk(item, re, action_name, resolve_secret, resolve_env)?);
                    }
                    serde_json::Value::Array(out)
                }
                serde_json::Value::Object(map) => {
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (k, val) in map {
                        out.insert(
                            k.clone(),
                            walk(val, re, action_name, resolve_secret, resolve_env)?,
                        );
                    }
                    serde_json::Value::Object(out)
                }
                other => other.clone(),
            })
        }

        walk(arguments, &re, action_name, &resolve_secret, &resolve_env)
    }

    async fn execute_mcp_action(
        &self,
        binding: McpBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let registry = self
            .mcp_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MCP registry not initialized"))?;
        let mut registry = registry.write().await;
        match binding.kind {
            McpBindingKind::Tool { name } => {
                registry
                    .call_tool(&binding.server_id, &name, arguments)
                    .await
            }
            McpBindingKind::Resource { uri } => {
                registry.read_resource(&binding.server_id, &uri).await
            }
        }
    }

    async fn execute_plugin_action(
        &self,
        binding: PluginBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let registry = self
            .plugin_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Plugin registry not initialized"))?;
        registry
            .write()
            .await
            .invoke_action(&binding.plugin_id, &binding.action_name, arguments)
            .await
    }

    async fn execute_custom_api_action(
        &self,
        binding: CustomApiBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let mut path = binding.path.clone();
        let mut query_pairs = binding.default_query.clone();
        let mut dynamic_headers = reqwest::header::HeaderMap::new();

        for parameter in &binding.parameters {
            let maybe_value = arguments.get(&parameter.name);
            match parameter.location {
                crate::custom_apis::CustomApiParameterLocation::Path => {
                    let value = maybe_value
                        .and_then(Self::value_to_http_string)
                        .filter(|value| !value.is_empty());
                    let value = match value {
                        Some(value) => value,
                        None if parameter.required => {
                            return Err(anyhow::anyhow!(
                                "Missing required path parameter '{}'",
                                parameter.name
                            ));
                        }
                        None => continue,
                    };
                    let encoded = urlencoding::encode(&value).to_string();
                    path = path.replace(&format!("{{{}}}", parameter.name), encoded.as_str());
                    path = path.replace(&format!(":{}", parameter.name), encoded.as_str());
                }
                crate::custom_apis::CustomApiParameterLocation::Query => {
                    if let Some(value) = maybe_value
                        .and_then(Self::value_to_http_string)
                        .filter(|v| !v.is_empty())
                    {
                        query_pairs.insert(parameter.name.clone(), value);
                    } else if parameter.required && !query_pairs.contains_key(&parameter.name) {
                        return Err(anyhow::anyhow!(
                            "Missing required query parameter '{}'",
                            parameter.name
                        ));
                    }
                }
                crate::custom_apis::CustomApiParameterLocation::Header => {
                    if let Some(value) = maybe_value
                        .and_then(Self::value_to_http_string)
                        .filter(|v| !v.is_empty())
                    {
                        let header_name =
                            reqwest::header::HeaderName::from_bytes(parameter.name.as_bytes())
                                .map_err(|_| {
                                    anyhow::anyhow!(
                                        "Invalid header parameter name '{}'",
                                        parameter.name
                                    )
                                })?;
                        let header_value =
                            reqwest::header::HeaderValue::from_str(&value).map_err(|_| {
                                anyhow::anyhow!("Invalid header value for '{}'", parameter.name)
                            })?;
                        dynamic_headers.insert(header_name, header_value);
                    } else if parameter.required {
                        return Err(anyhow::anyhow!(
                            "Missing required header parameter '{}'",
                            parameter.name
                        ));
                    }
                }
                crate::custom_apis::CustomApiParameterLocation::Body => {}
            }
        }

        let base = binding.base_url.trim_end_matches('/');
        let path = if path.starts_with('/') {
            path
        } else {
            format!("/{}", path)
        };
        let mut url = reqwest::Url::parse(&format!("{}{}", base, path))
            .map_err(|e| anyhow::anyhow!("Invalid custom API URL: {}", e))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &query_pairs {
                pairs.append_pair(key, value);
            }
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;
        let method = reqwest::Method::from_bytes(binding.method.as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid HTTP method '{}': {}", binding.method, e))?;
        let manager =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let secret = manager.get_custom_secret(&binding.secret_key)?;
        let secret = secret.as_deref().filter(|value| !value.trim().is_empty());

        if matches!(
            binding.auth_mode,
            crate::custom_apis::CustomApiAuthMode::ApiKeyQuery
        ) {
            let token = secret.ok_or_else(|| {
                anyhow::anyhow!(
                    "Auth secret '{}' is not configured for '{}'",
                    binding.secret_key,
                    binding.api_name
                )
            })?;
            let query_name = binding.auth_name.as_deref().unwrap_or("api_key");
            url.query_pairs_mut().append_pair(query_name, token.trim());
        }

        let mut request = client.request(method, url.clone());
        for (key, value) in &binding.default_headers {
            let header_name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| anyhow::anyhow!("Invalid default header name '{}'", key))?;
            let header_value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| anyhow::anyhow!("Invalid default header value for '{}'", key))?;
            request = request.header(header_name, header_value);
        }
        for (key, value) in dynamic_headers.iter() {
            request = request.header(key, value);
        }

        request = match binding.auth_mode {
            crate::custom_apis::CustomApiAuthMode::None
            | crate::custom_apis::CustomApiAuthMode::ApiKeyQuery => request,
            crate::custom_apis::CustomApiAuthMode::Bearer
            | crate::custom_apis::CustomApiAuthMode::OAuth2 => {
                let token = secret.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Auth secret '{}' is not configured for '{}'",
                        binding.secret_key,
                        binding.api_name
                    )
                })?;
                let header_name = binding.auth_header.as_deref().unwrap_or("Authorization");
                if header_name.eq_ignore_ascii_case("authorization") {
                    request.bearer_auth(token.trim())
                } else {
                    request.header(header_name, format!("Bearer {}", token.trim()))
                }
            }
            crate::custom_apis::CustomApiAuthMode::ApiKeyHeader => {
                let token = secret.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Auth secret '{}' is not configured for '{}'",
                        binding.secret_key,
                        binding.api_name
                    )
                })?;
                let header_name = binding
                    .auth_name
                    .as_deref()
                    .or(binding.auth_header.as_deref())
                    .unwrap_or("X-API-Key");
                request.header(header_name, token.trim())
            }
            crate::custom_apis::CustomApiAuthMode::Basic => {
                let password = secret.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Auth secret '{}' is not configured for '{}'",
                        binding.secret_key,
                        binding.api_name
                    )
                })?;
                request.basic_auth(
                    binding.auth_username.clone().unwrap_or_default(),
                    Some(password.trim().to_string()),
                )
            }
        };

        if binding.body_required || arguments.get("body").is_some() {
            let body = arguments.get("body").cloned().ok_or_else(|| {
                anyhow::anyhow!("This endpoint requires a JSON body under the 'body' field")
            })?;
            request = request.json(&body);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("custom API call '{}' failed", binding.operation_name))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = response.text().await.unwrap_or_default();
        let rendered = if content_type.contains("json") {
            serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .unwrap_or_else(|| body.clone())
        } else {
            body.clone()
        };
        let rendered = if rendered.chars().count() > 6_000 {
            format!("{}...", rendered.chars().take(6_000).collect::<String>())
        } else {
            rendered
        };
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Custom API '{}' returned HTTP {}:\n{}",
                binding.operation_name,
                status,
                rendered
            ));
        }
        Ok(format!(
            "{} {} succeeded.\n{}",
            binding.method.to_ascii_uppercase(),
            binding.operation_name,
            rendered
        ))
    }

    fn value_to_http_string(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Bool(v) => Some(v.to_string()),
            serde_json::Value::Number(v) => Some(v.to_string()),
            other => serde_json::to_string(other).ok(),
        }
    }

    /// Execute an action natively (no sandbox)
    async fn execute_native(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        match action_name {
            "file_read" => {
                let path = arguments["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing path"))?;
                let path = self.resolve_tool_read_path(path)?;
                let content = tokio::fs::read_to_string(&path).await?;
                Ok(content)
            }
            "file_write" => {
                let path = arguments["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing path"))?;
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let path = self.resolve_tool_write_path(path)?;
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, content).await?;
                Ok(format!("Written to {}", path.display()))
            }
            "clipboard_read" => {
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;
                let content = clipboard
                    .get_text()
                    .map_err(|e| anyhow::anyhow!("Failed to read clipboard: {}", e))?;
                Ok(content)
            }
            "clipboard_write" => {
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;
                clipboard
                    .set_text(content)
                    .map_err(|e| anyhow::anyhow!("Failed to write clipboard: {}", e))?;
                Ok("Content copied to clipboard".to_string())
            }
            "list_tasks" => {
                let queue = self
                    .task_queue
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Task queue not available"))?;
                let tasks = queue.read().await;
                let filter = arguments
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");

                let filtered: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| match filter {
                        "pending" => matches!(
                            t.status,
                            crate::core::TaskStatus::Pending
                                | crate::core::TaskStatus::AwaitingApproval
                        ),
                        "paused" => matches!(t.status, crate::core::TaskStatus::Paused),
                        "goals" => t.action == "goal",
                        "routines" => t.cron.is_some(),
                        "completed" => matches!(t.status, crate::core::TaskStatus::Completed),
                        "failed" => matches!(t.status, crate::core::TaskStatus::Failed { .. }),
                        _ => true, // "all"
                    })
                    .collect();

                if filtered.is_empty() {
                    return Ok(format!("No {} items found.", filter));
                }

                let mut output = format!("Found {} {} item(s):\n\n", filtered.len(), filter);
                for t in &filtered {
                    let status_str = match &t.status {
                        crate::core::TaskStatus::Pending => "Pending",
                        crate::core::TaskStatus::AwaitingApproval => "Awaiting Approval",
                        crate::core::TaskStatus::ExpiredNeedsReapproval => {
                            "Expired - Needs Reapproval"
                        }
                        crate::core::TaskStatus::Paused => "Paused",
                        crate::core::TaskStatus::InProgress => "In Progress",
                        crate::core::TaskStatus::Completed => "Completed",
                        crate::core::TaskStatus::Failed { .. } => "Failed",
                        crate::core::TaskStatus::Cancelled => "Cancelled",
                    };
                    output.push_str(&format!("- {} (status: {})\n", t.description, status_str));
                    if let Some(ref cron) = t.cron {
                        output.push_str(&format!("  Schedule: {}\n", cron));
                    }
                }
                Ok(output)
            }
            "tunnel_control" => self.execute_tunnel_control(arguments).await,
            "schedule_task" => {
                let task_desc = arguments["task"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing task description"))?;

                let schedule_info =
                    if let Some(cron_expr) = arguments.get("cron").and_then(|v| v.as_str()) {
                        // Auto-convert standard 5-field cron to 6-field (with seconds)
                        // Standard: "minute hour day month weekday" -> "0 9 * * *"
                        // Rust cron: "second minute hour day month weekday" -> "0 0 9 * * *"
                        let cron_6field = if cron_expr.split_whitespace().count() == 5 {
                            format!("0 {}", cron_expr) // Prepend "0 " for seconds
                        } else {
                            cron_expr.to_string()
                        };

                        // Validate cron expression
                        cron_6field.parse::<cron::Schedule>().map_err(|e| {
                            anyhow::anyhow!("Invalid cron expression '{}': {}", cron_6field, e)
                        })?;
                        format!("cron:{}", cron_6field)
                    } else if let Some(at_time) = arguments.get("at").and_then(|v| v.as_str()) {
                        // Validate ISO timestamp
                        chrono::DateTime::parse_from_rfc3339(at_time)
                            .map_err(|e| anyhow::anyhow!("Invalid timestamp: {}", e))?;
                        format!("at:{}", at_time)
                    } else {
                        return Err(anyhow::anyhow!(
                            "Must specify either 'cron' or 'at' for scheduling"
                        ));
                    };

                // Return scheduling info - actual scheduling is handled by the agent's task queue
                Ok(format!(
                    "Task scheduled: {} | Schedule: {}",
                    task_desc, schedule_info
                ))
            }
            "watch" => {
                // Return a marker — actual watcher creation is handled by Agent::handle_watch
                let desc = arguments
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("watcher");
                Ok(format!("Watch created: {}", desc))
            }
            "manage_actions" => self.execute_manage_actions(arguments).await,
            "capability_acquire" => self.execute_capability_acquire(arguments).await,
            "connector_request" => self.execute_connector_request(arguments).await,
            "pipeline_compile" => self.execute_pipeline_compile(arguments).await,
            "pipeline_run" => self.execute_pipeline_run(arguments).await,
            "signal_consensus" => self.execute_signal_consensus(arguments).await,
            "gmail_scan" => crate::actions::gmail::gmail_scan(&self.config_dir, arguments).await,
            "gmail_reply" => crate::actions::gmail::gmail_reply(&self.config_dir, arguments).await,
            "google_drive_search" => {
                crate::actions::google_workspace::drive_search(&self.config_dir, arguments).await
            }
            "google_docs_read" => {
                crate::actions::google_workspace::docs_read(&self.config_dir, arguments).await
            }
            "google_sheets_read" => {
                crate::actions::google_workspace::sheets_read(&self.config_dir, arguments).await
            }
            "google_chat_list_spaces" => {
                crate::actions::google_workspace::chat_list_spaces(&self.config_dir, arguments)
                    .await
            }
            "google_admin_list_users" => {
                crate::actions::google_workspace::admin_list_users(&self.config_dir, arguments)
                    .await
            }
            "google_workspace_gws_help" => {
                crate::actions::google_workspace::gws_help(arguments).await
            }
            "google_workspace_gws_schema" => {
                crate::actions::google_workspace::gws_schema(arguments).await
            }
            "google_workspace_gws_skills" => {
                crate::actions::google_workspace::gws_skills(&self.config_dir, arguments).await
            }
            "google_workspace_gws_command" => {
                crate::actions::google_workspace::gws_command(&self.config_dir, arguments).await
            }
            "web_search" => {
                let args: crate::actions::search::SearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid search arguments: {}", e))?;

                let config = build_search_config(&self.config_dir).await;
                crate::actions::search::execute_search(&args, &config).await
            }
            "research" => {
                let args: crate::actions::research::ResearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid research arguments: {}", e))?;

                let config = build_search_config(&self.config_dir).await;
                crate::actions::research::execute_research(&args, &config).await
            }
            "video-frames" => {
                // This action requires ffmpeg - check arguments
                let video = arguments
                    .get("video")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'video' path argument"))?;
                let time = arguments
                    .get("time")
                    .and_then(|v| v.as_str())
                    .unwrap_or("00:00:00");
                let output = arguments
                    .get("out")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{}_frame.jpg", video.trim_end_matches(".mp4")));

                // Execute ffmpeg
                let output_result = tokio::process::Command::new("ffmpeg")
                    .args([
                        "-ss",
                        time,
                        "-i",
                        video,
                        "-frames:v",
                        "1",
                        "-q:v",
                        "2",
                        &output,
                        "-y",
                    ])
                    .output()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to run ffmpeg: {}", e))?;

                if output_result.status.success() {
                    Ok(format!("Frame extracted to: {}", output))
                } else {
                    let stderr = String::from_utf8_lossy(&output_result.stderr);
                    Err(anyhow::anyhow!("ffmpeg failed: {}", stderr))
                }
            }
            "code_execute" => {
                // Native fallback for code execution (when Docker mode falls through)
                self.execute_code_native(arguments).await
            }
            "browse" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let extract = arguments
                    .get("extract")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text");

                // Fetch the page
                let client = reqwest::Client::builder()
                    .user_agent("AgentArk/0.1 (AI Agent Browser)")
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to fetch URL: {}", e))?;

                let status = response.status();
                if !status.is_success() {
                    return Err(anyhow::anyhow!(
                        "HTTP error {}: {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("Unknown")
                    ));
                }

                let html = response
                    .text()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

                // Extract content based on the extract parameter
                let title_re = regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap();
                let title = title_re
                    .captures(&html)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();

                match extract {
                    "title" => Ok(if title.is_empty() {
                        "(no title found)".to_string()
                    } else {
                        title
                    }),
                    "links" => {
                        let link_re = regex::Regex::new(
                            r#"(?is)<a[^>]+href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#,
                        )
                        .unwrap();
                        let mut links = Vec::new();
                        for cap in link_re.captures_iter(&html) {
                            let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                            let text = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                            // Strip HTML tags from link text
                            let clean_text = tag_re.replace_all(text, "").trim().to_string();
                            if !href.is_empty()
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                links.push(format!(
                                    "[{}]({})",
                                    if clean_text.is_empty() {
                                        href
                                    } else {
                                        &clean_text
                                    },
                                    href
                                ));
                            }
                        }
                        if links.is_empty() {
                            Ok("(no links found)".to_string())
                        } else {
                            // Limit to 50 links to avoid overwhelming output
                            let display_links: Vec<&str> =
                                links.iter().take(50).map(|s| s.as_str()).collect();
                            Ok(format!(
                                "Found {} links (showing up to 50):\n{}",
                                links.len(),
                                display_links.join("\n")
                            ))
                        }
                    }
                    "all" => {
                        // Extract text
                        let text = Self::html_to_text(&html);
                        // Extract links
                        let link_re = regex::Regex::new(
                            r#"(?is)<a[^>]+href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#,
                        )
                        .unwrap();
                        let mut links = Vec::new();
                        for cap in link_re.captures_iter(&html) {
                            let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                            let link_text = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                            let clean_text = tag_re.replace_all(link_text, "").trim().to_string();
                            if !href.is_empty()
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                links.push(format!(
                                    "[{}]({})",
                                    if clean_text.is_empty() {
                                        href
                                    } else {
                                        &clean_text
                                    },
                                    href
                                ));
                            }
                        }
                        let links_section = if links.is_empty() {
                            "(no links found)".to_string()
                        } else {
                            let display_links: Vec<&str> =
                                links.iter().take(30).map(|s| s.as_str()).collect();
                            format!(
                                "{} links (showing up to 30):\n{}",
                                links.len(),
                                display_links.join("\n")
                            )
                        };
                        Ok(format!(
                            "## Title\n{}\n\n## Content\n{}\n\n## Links\n{}",
                            if title.is_empty() {
                                "(no title)"
                            } else {
                                &title
                            },
                            text,
                            links_section
                        ))
                    }
                    _ => {
                        // Default: extract text content
                        let text = Self::html_to_text(&html);
                        Ok(if text.is_empty() {
                            "(no text content extracted)".to_string()
                        } else {
                            text
                        })
                    }
                }
            }
            "pdf_generate" => {
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let title = arguments
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Document");
                let filename = arguments
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output.pdf");
                let style = arguments
                    .get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("plain");

                let output_path = self.data_dir().join("outputs").join(filename);
                if let Some(parent) = output_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                // Generate Python script that creates the PDF using fpdf2
                let escaped_content = content
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n");
                let escaped_title = title.replace('\\', "\\\\").replace('"', "\\\"");
                let out_str = output_path.to_string_lossy().replace('\\', "/");

                let python_code = format!(
                    r#"
import subprocess, sys
subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "fpdf2"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
from fpdf import FPDF

pdf = FPDF()
pdf.set_auto_page_break(auto=True, margin=15)
pdf.add_page()
style = "{style}"

if style == "invoice":
    pdf.set_font("Helvetica", "B", 20)
    pdf.cell(0, 15, "{escaped_title}", new_x="LMARGIN", new_y="NEXT")
    pdf.set_font("Helvetica", "", 10)
elif style == "report":
    pdf.set_font("Helvetica", "B", 16)
    pdf.cell(0, 12, "{escaped_title}", new_x="LMARGIN", new_y="NEXT")
    pdf.line(10, pdf.get_y(), 200, pdf.get_y())
    pdf.ln(5)
    pdf.set_font("Helvetica", "", 11)
elif style == "letter":
    pdf.set_font("Helvetica", "", 11)
    pdf.ln(20)
else:
    pdf.set_font("Helvetica", "", 11)

content = "{escaped_content}"
for line in content.split("\\n"):
    pdf.multi_cell(0, 6, line)

pdf.output("{out_str}")
print("PDF generated: {out_str}")
"#
                );
                let code_args = serde_json::json!({
                    "language": "python",
                    "code": python_code.trim()
                });
                self.execute_code_native(&code_args).await
            }
            "expense" => {
                let action = arguments["action"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing action parameter"))?;
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;

                match action {
                    "add" => {
                        let amount = arguments
                            .get("amount")
                            .and_then(|v| v.as_f64())
                            .ok_or_else(|| anyhow::anyhow!("Missing amount"))?;
                        let description = arguments
                            .get("description")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing description"))?;
                        let currency = arguments
                            .get("currency")
                            .and_then(|v| v.as_str())
                            .unwrap_or("USD");
                        let category = arguments
                            .get("category")
                            .and_then(|v| v.as_str())
                            .unwrap_or("other");
                        let date = arguments
                            .get("date")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

                        let id = format!(
                            "exp-{}",
                            uuid::Uuid::new_v4()
                                .to_string()
                                .split('-')
                                .next()
                                .unwrap_or("0")
                        );
                        let model = crate::storage::entities::expense::Model {
                            id: id.clone(),
                            amount,
                            currency: currency.to_string(),
                            category: category.to_string(),
                            description: description.to_string(),
                            date,
                            payment_method: arguments
                                .get("payment_method")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            vendor: arguments
                                .get("vendor")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            tags: arguments
                                .get("tags")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            split_with: None,
                            receipt_path: None,
                            created_at: chrono::Utc::now().to_rfc3339(),
                        };
                        storage.insert_expense(model).await?;
                        Ok(format!(
                            "Expense recorded: {} {} for '{}' (category: {}, id: {})",
                            currency, amount, description, category, id
                        ))
                    }
                    "list" => {
                        let from = arguments.get("from_date").and_then(|v| v.as_str());
                        let to = arguments.get("to_date").and_then(|v| v.as_str());
                        let cat = arguments.get("filter_category").and_then(|v| v.as_str());
                        let expenses = storage.get_expenses(from, to, cat).await?;
                        if expenses.is_empty() {
                            return Ok("No expenses found.".to_string());
                        }
                        let mut output = format!("Found {} expense(s):\n\n", expenses.len());
                        let mut total = 0.0f64;
                        for e in &expenses {
                            output.push_str(&format!(
                                "- [{}] {} {} — {} ({}){}\n",
                                e.id,
                                e.currency,
                                e.amount,
                                e.description,
                                e.category,
                                e.vendor
                                    .as_ref()
                                    .map(|v| format!(" @ {}", v))
                                    .unwrap_or_default()
                            ));
                            total += e.amount;
                        }
                        output.push_str(&format!("\nTotal: {:.2}", total));
                        Ok(output)
                    }
                    "summary" => {
                        let from = arguments.get("from_date").and_then(|v| v.as_str());
                        let to = arguments.get("to_date").and_then(|v| v.as_str());
                        let expenses = storage.get_expense_summary(from, to).await?;
                        if expenses.is_empty() {
                            return Ok("No expenses found for the period.".to_string());
                        }
                        // Aggregate by category
                        let mut by_category: std::collections::HashMap<String, f64> =
                            std::collections::HashMap::new();
                        for e in &expenses {
                            *by_category.entry(e.category.clone()).or_insert(0.0) += e.amount;
                        }
                        let mut output = "Expense Summary by Category:\n\n".to_string();
                        let mut grand_total = 0.0f64;
                        let mut cats: Vec<_> = by_category.into_iter().collect();
                        cats.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        for (category, total) in &cats {
                            output.push_str(&format!("  {}: {:.2}\n", category, total));
                            grand_total += total;
                        }
                        output.push_str(&format!("\nGrand Total: {:.2}", grand_total));
                        Ok(output)
                    }
                    "delete" => {
                        let id = arguments
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing expense ID"))?;
                        let deleted = storage.delete_expense(id).await?;
                        if deleted {
                            Ok(format!("Expense {} deleted.", id))
                        } else {
                            Ok(format!("Expense {} not found.", id))
                        }
                    }
                    _ => Err(anyhow::anyhow!("Unknown expense action: {}", action)),
                }
            }
            "security_logs" => {
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50);
                let logs = storage.list_security_logs(limit).await?;
                if logs.is_empty() {
                    return Ok("No security events recorded. All clear.".to_string());
                }
                let mut output = format!("Security Log ({} entries):\n\n", logs.len());
                for log in &logs {
                    output.push_str(&format!(
                        "- [{}] {} ({}): {} (count: {})\n",
                        log.created_at.split('T').next().unwrap_or(&log.created_at),
                        log.event_type,
                        log.severity,
                        log.message,
                        log.count,
                    ));
                }
                Ok(output)
            }
            "transcribe_audio" => {
                let file_path = arguments["file_path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
                let language = arguments
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("auto");
                let model = arguments
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("base");

                let escaped_path = file_path.replace('\\', "/");
                let lang_arg = if language == "auto" {
                    "None".to_string()
                } else {
                    format!("\"{}\"", language)
                };

                let python_code = format!(
                    r#"
import subprocess, sys
subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "openai-whisper"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
import whisper

model = whisper.load_model("{model}")
result = model.transcribe("{escaped_path}", language={lang_arg})
print(result["text"])
"#
                );
                let code_args = serde_json::json!({
                    "language": "python",
                    "code": python_code.trim()
                });
                self.execute_code_native(&code_args).await
            }
            "weekly_review" => {
                let period = arguments
                    .get("period_days")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(7);
                let queue = self
                    .task_queue
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Task queue not available"))?;
                let tasks = queue.read().await;

                let mut output = format!("Weekly Review (last {} days):\n\n", period);

                // Completed tasks
                let completed: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Completed))
                    .collect();
                output.push_str(&format!("**Completed Tasks** ({})\n", completed.len()));
                for t in &completed {
                    output.push_str(&format!("  - {}\n", t.description));
                }

                // Pending tasks
                let pending: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.status,
                            crate::core::TaskStatus::Pending
                                | crate::core::TaskStatus::AwaitingApproval
                        )
                    })
                    .collect();
                output.push_str(&format!("\n**Pending Tasks** ({})\n", pending.len()));
                for t in &pending {
                    output.push_str(&format!("  - {}\n", t.description));
                }

                let paused: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Paused))
                    .collect();
                if !paused.is_empty() {
                    output.push_str(&format!("\n**Paused Tasks** ({})\n", paused.len()));
                    for t in &paused {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                let paused: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Paused))
                    .collect();
                if !paused.is_empty() {
                    output.push_str(&format!("\n**Paused Tasks** ({})\n", paused.len()));
                    for t in &paused {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                // Failed tasks
                let failed: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Failed { .. }))
                    .collect();
                if !failed.is_empty() {
                    output.push_str(&format!("\n**Failed Tasks** ({})\n", failed.len()));
                    for t in &failed {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                // Expense summary if storage available
                if let Some(ref storage) = self.storage {
                    let from_date = (chrono::Utc::now() - chrono::Duration::days(period))
                        .format("%Y-%m-%d")
                        .to_string();
                    if let Ok(expenses) = storage.get_expense_summary(Some(&from_date), None).await
                    {
                        if !expenses.is_empty() {
                            let mut by_cat: std::collections::HashMap<String, f64> =
                                std::collections::HashMap::new();
                            for e in &expenses {
                                *by_cat.entry(e.category.clone()).or_insert(0.0) += e.amount;
                            }
                            output.push_str("\n**Spending Summary**\n");
                            let mut total = 0.0;
                            for (cat, amt) in &by_cat {
                                output.push_str(&format!("  {}: {:.2}\n", cat, amt));
                                total += amt;
                            }
                            output.push_str(&format!("  Total: {:.2}\n", total));
                        }
                    }
                }

                Ok(output)
            }
            "current_time" => {
                let timezone_name = arguments
                    .get("timezone")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let now_utc = chrono::Utc::now();
                if let Some(timezone_name) = timezone_name {
                    let timezone = timezone_name.parse::<chrono_tz::Tz>().map_err(|_| {
                        anyhow::anyhow!("Invalid timezone '{}'. Expected an IANA name such as Asia/Kolkata.", timezone_name)
                    })?;
                    let local = now_utc.with_timezone(&timezone);
                    Ok(format!(
                        "Timezone: {}\nISO: {}\nDate: {}\nReadable date: {}\nTime: {}\nWeekday: {}\nUnix: {}",
                        timezone_name,
                        local.to_rfc3339(),
                        local.format("%Y-%m-%d"),
                        local.format("%B %d, %Y"),
                        local.format("%H:%M:%S %Z"),
                        local.format("%A"),
                        now_utc.timestamp()
                    ))
                } else {
                    Ok(format!(
                        "Timezone: UTC\nISO: {}\nDate: {}\nReadable date: {}\nTime: {}\nWeekday: {}\nUnix: {}",
                        now_utc.to_rfc3339(),
                        now_utc.format("%Y-%m-%d"),
                        now_utc.format("%B %d, %Y"),
                        now_utc.format("%H:%M:%S UTC"),
                        now_utc.format("%A"),
                        now_utc.timestamp()
                    ))
                }
            }
            "notify_user" => {
                let message = arguments
                    .get("message")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("notify_user requires a non-empty `message`"))?;
                let title = arguments
                    .get("title")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(title) = title {
                    Ok(format!("{}\n\n{}", title, message))
                } else {
                    Ok(message.to_string())
                }
            }
            // Google Calendar actions
            "calendar_today" => {
                crate::actions::calendar::calendar_today(&self.config_dir, arguments).await
            }
            "calendar_list" => {
                crate::actions::calendar::calendar_list(&self.config_dir, arguments).await
            }
            "calendar_create" => {
                crate::actions::calendar::calendar_create(&self.config_dir, arguments).await
            }
            "calendar_free" => {
                crate::actions::calendar::calendar_free(&self.config_dir, arguments).await
            }
            // SSH remote execution
            #[cfg(feature = "ssh")]
            "ssh" => crate::actions::ssh::ssh_execute(&self.config_dir, arguments).await,
            #[cfg(feature = "ssh")]
            "ssh_connections" => crate::actions::ssh::ssh_list_connections(&self.config_dir).await,
            // Video generation via Remotion
            "video_generate" => {
                crate::actions::video::video_generate(&self.config_dir, self.data_dir(), arguments)
                    .await
            }
            // Handle workflow actions - return marker for agent to process with LLM
            other => {
                let actions = self.actions.read().await;
                if let Some(action) = actions.get(other) {
                    if action.workflow_content.is_some() {
                        // Return a special marker that tells the agent to use LLM-driven execution.
                        let user_query = Self::build_workflow_user_query(arguments);
                        let has_freeform_query = arguments
                            .get("query")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| !s.trim().is_empty());
                        if !has_freeform_query {
                            let required = Self::collect_required_fields_from_schema(
                                &action.info.input_schema,
                            );
                            let missing: Vec<String> = required
                                .iter()
                                .filter(|k| !Self::has_non_empty_argument(arguments, k))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                let payload = WorkflowMissingInputsPayload {
                                    action: other.to_string(),
                                    missing,
                                    required,
                                    provided: Self::collect_provided_argument_keys(arguments),
                                    query: user_query,
                                };
                                return Ok(Self::build_workflow_missing_inputs_marker(&payload));
                            }
                        }
                        return Ok(format!(
                            "{}{}:{}",
                            WORKFLOW_ACTION_MARKER, other, user_query
                        ));
                    }
                }
                Err(anyhow::anyhow!("Unknown native action: {}", action_name))
            }
        }
    }

    async fn execute_manage_actions(&self, arguments: &serde_json::Value) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'operation' parameter"))?;

        match operation {
            "create" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for create"))?;
                let content = arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'content' for create"))?;
                if !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    return Err(anyhow::anyhow!(
                        "Invalid action name. Use kebab-case (e.g., 'check-weather')"
                    ));
                }
                if self.actions.read().await.contains_key(name) {
                    return Err(anyhow::anyhow!(
                        "Action '{}' already exists. Use 'update' instead.",
                        name
                    ));
                }
                let verdict = self.create_action(name, content, false).await?;
                let mut msg = format!("Action '{}' created and is immediately available.", name);
                if let Some(ref v) = verdict {
                    if !v.warnings.is_empty() {
                        msg.push_str(&format!("\nSecurity warnings: {}", v.warnings.join(", ")));
                    }
                    if !v.allow_load {
                        msg = format!(
                            "Action '{}' was blocked by security verification: {}",
                            name,
                            v.warnings.join(", ")
                        );
                    }
                }
                Ok(msg)
            }
            "update" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for update"))?;
                let content = arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'content' for update"))?;
                match self.update_action_content(name, content).await? {
                    true => Ok(format!("Action '{}' updated.", name)),
                    false => Err(anyhow::anyhow!(
                        "Cannot update '{}'. System actions are read-only.",
                        name
                    )),
                }
            }
            "delete" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for delete"))?;
                match self.delete_action(name).await? {
                    true => Ok(format!("Action '{}' deleted.", name)),
                    false => Err(anyhow::anyhow!(
                        "Cannot delete '{}'. Only custom actions can be deleted.",
                        name
                    )),
                }
            }
            "list" => {
                let actions = self.list_actions().await?;
                let list: Vec<String> = actions
                    .iter()
                    .map(|a| {
                        let source = match a.source {
                            ActionSource::System => "system",
                            ActionSource::Bundled => "bundled",
                            ActionSource::Custom => "custom",
                        };
                        format!("- **{}** ({}): {}", a.name, source, a.description)
                    })
                    .collect();
                Ok(format!(
                    "Available actions ({}):\n{}",
                    actions.len(),
                    list.join("\n")
                ))
            }
            _ => Err(anyhow::anyhow!(
                "Unknown operation '{}'. Use create, update, delete, or list.",
                operation
            )),
        }
    }

    async fn execute_capability_acquire(&self, arguments: &serde_json::Value) -> Result<String> {
        let arguments = Self::enrich_capability_acquisition_arguments(arguments).await;
        let raw_name = arguments
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for capability acquisition"))?;
        let description = arguments
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'description' for capability acquisition"))?;
        let name = Self::normalize_generated_action_name(raw_name);
        if name.is_empty() {
            return Err(anyhow::anyhow!(
                "Generated action name is empty after normalization"
            ));
        }
        let content = self.render_capability_action_markdown(&arguments, &name, description);
        let force = arguments
            .get("force")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let verdict = self.create_action(&name, &content, force).await?;
        let mut lines = vec![
            format!("Capability scaffolded as action `{}`.", name),
            "It is now available immediately in the action catalog.".to_string(),
        ];
        if let Some(kind) = arguments.get("kind").and_then(|value| value.as_str()) {
            lines.push(format!("Mode: {}", kind));
        }
        if let Some(base_url) = arguments.get("base_url").and_then(|value| value.as_str()) {
            lines.push(format!("Base URL: {}", base_url));
        }
        if let Some(path) = arguments.get("path").and_then(|value| value.as_str()) {
            lines.push(format!("Primary endpoint: {}", path));
        }
        if let Some(method) = arguments.get("method").and_then(|value| value.as_str()) {
            lines.push(format!("Method: {}", method.to_ascii_uppercase()));
        }
        if let Some(secret_name) = arguments
            .get("auth_secret_name")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
        {
            lines.push(format!(
                "Expected secret/config key for auth: `{}`.",
                secret_name
            ));
        }
        if let Some(source_notes) = arguments
            .get("source_notes")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
        {
            lines.push(format!("Derived from: {}", source_notes));
        }
        if let Some(verdict) = verdict {
            if !verdict.warnings.is_empty() {
                lines.push(format!(
                    "Security review notes: {}",
                    verdict.warnings.join(", ")
                ));
            }
            if !verdict.allow_load {
                lines.push(
                    "The scaffold was blocked by security policy and was not loaded.".to_string(),
                );
            }
        }
        Ok(lines.join("\n"))
    }

    async fn execute_pipeline_compile(&self, arguments: &serde_json::Value) -> Result<String> {
        let spec_value = arguments
            .get("spec")
            .ok_or_else(|| anyhow::anyhow!("Missing spec"))?;
        let spec: crate::core::pipeline::PipelineSpec = serde_json::from_value(spec_value.clone())
            .map_err(|e| anyhow::anyhow!("Invalid pipeline spec: {}", e))?;
        let compiled = crate::core::pipeline::compile_pipeline(&spec)?;

        let save = arguments
            .get("save")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let mut persisted = false;
        let mut warnings = compiled.warnings.clone();
        if save {
            if let Some(storage) = self.storage.as_ref() {
                let key = format!("pipeline:spec:{}", Self::pipeline_key_slug(&spec.name));
                storage.set(&key, &serde_json::to_vec(&spec)?).await?;
                persisted = true;
            } else {
                warnings.push("storage unavailable; pipeline spec not persisted".to_string());
            }
        }

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "compiled",
            "pipeline": compiled.name,
            "node_count": compiled.node_count,
            "ordered_nodes": compiled.ordered_nodes,
            "warnings": warnings,
            "persisted": persisted,
        }))?)
    }

    async fn execute_signal_consensus(&self, arguments: &serde_json::Value) -> Result<String> {
        let request: crate::core::pipeline::SignalConsensusRequest =
            serde_json::from_value(arguments.clone())
                .map_err(|e| anyhow::anyhow!("Invalid signal_consensus arguments: {}", e))?;
        let result = crate::core::pipeline::run_signal_consensus(&request)?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_connector_request(&self, arguments: &serde_json::Value) -> Result<String> {
        let spec: crate::core::connector::ConnectorRequestSpec =
            serde_json::from_value(arguments.clone())
                .map_err(|e| anyhow::anyhow!("Invalid connector_request arguments: {}", e))?;

        if spec.url.trim().is_empty() {
            return Err(anyhow::anyhow!("connector_request requires non-empty url"));
        }

        let retry = spec.retry.normalized();
        let timeout_secs = spec.timeout_secs.clamp(1, 300);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

        let mut pages = Vec::new();
        let mut items = Vec::new();
        let mut total_requests = 0usize;

        let pagination = &spec.pagination;
        let max_pages = pagination.max_pages.clamp(1, 1_000);
        let mut page_no = pagination.start_page.max(1);
        let mut cursor: Option<String> = None;

        for _ in 0..max_pages {
            let mut query = spec.query.clone();
            if let Some(page_size) = pagination.page_size {
                if page_size > 0 {
                    query.insert(pagination.page_size_param.clone(), page_size.to_string());
                }
            }
            match pagination.mode {
                crate::core::connector::PaginationMode::Page => {
                    query.insert(pagination.page_param.clone(), page_no.to_string());
                }
                crate::core::connector::PaginationMode::Cursor => {
                    if let Some(ref c) = cursor {
                        query.insert(pagination.cursor_param.clone(), c.clone());
                    }
                }
                crate::core::connector::PaginationMode::None => {}
            }

            let mut attempt = 1u32;
            let mut backoff_ms = retry.initial_backoff_ms;
            let mut refreshed = false;
            let (status, body_text, request_url) = loop {
                total_requests += 1;
                match self.connector_send_once(&client, &spec, &query).await {
                    Ok((status, body_text, request_url)) => {
                        if !(200..300).contains(&status) {
                            if let Some(refresh) = spec.auth_refresh.as_ref() {
                                if refresh.retry_statuses.contains(&status) && !refreshed {
                                    if refresh.action.eq_ignore_ascii_case("connector_request") {
                                        return Err(anyhow::anyhow!(
                                            "auth_refresh.action cannot be connector_request"
                                        ));
                                    }
                                    // Break async recursion cycle:
                                    // execute_action -> execute_native -> execute_connector_request -> execute_action
                                    std::pin::Pin::from(Box::new(
                                        self.execute_action(&refresh.action, &refresh.arguments),
                                    ))
                                    .await?;
                                    refreshed = true;
                                    continue;
                                }
                            }
                            if attempt < retry.max_attempts
                                && retry.retry_on_status.contains(&status)
                            {
                                Self::sleep_with_backoff(backoff_ms, retry.jitter_ratio).await;
                                backoff_ms =
                                    (backoff_ms.saturating_mul(2)).min(retry.max_backoff_ms);
                                attempt += 1;
                                continue;
                            }
                            let snippet = if body_text.len() > 500 {
                                format!("{}...", &body_text[..500])
                            } else {
                                body_text.clone()
                            };
                            return Err(anyhow::anyhow!(
                                "Connector request failed (status {}): {}",
                                status,
                                snippet
                            ));
                        }
                        break (status, body_text, request_url);
                    }
                    Err(e) => {
                        if attempt < retry.max_attempts {
                            Self::sleep_with_backoff(backoff_ms, retry.jitter_ratio).await;
                            backoff_ms = (backoff_ms.saturating_mul(2)).min(retry.max_backoff_ms);
                            attempt += 1;
                            continue;
                        }
                        return Err(e);
                    }
                }
            };

            let payload: serde_json::Value = serde_json::from_str(&body_text)
                .unwrap_or_else(|_| serde_json::json!({ "raw_body": body_text }));

            let mut page_items =
                crate::core::connector::extract_items(&payload, &pagination.items_path);
            if page_items.is_empty()
                && pagination.mode == crate::core::connector::PaginationMode::None
            {
                page_items = match &payload {
                    serde_json::Value::Array(arr) => arr.clone(),
                    other => vec![other.clone()],
                };
            }

            let next_cursor =
                crate::core::connector::extract_next_cursor(&payload, &pagination.next_cursor_path);

            pages.push(crate::core::connector::ConnectorPageResult {
                request_url,
                status,
                item_count: page_items.len(),
                next_cursor: next_cursor.clone(),
            });
            items.extend(page_items);

            let done = match pagination.mode {
                crate::core::connector::PaginationMode::None => true,
                crate::core::connector::PaginationMode::Page => {
                    if pages.last().map(|p| p.item_count == 0).unwrap_or(true) {
                        true
                    } else {
                        page_no = page_no.saturating_add(1);
                        false
                    }
                }
                crate::core::connector::PaginationMode::Cursor => {
                    if next_cursor.as_ref().is_none() || next_cursor.as_ref() == cursor.as_ref() {
                        true
                    } else {
                        cursor = next_cursor;
                        false
                    }
                }
            };
            if done {
                break;
            }

            if spec.rate_limit_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(spec.rate_limit_ms)).await;
            }
        }

        let out = crate::core::connector::ConnectorRunResult {
            method: spec.method.as_str().to_string(),
            total_requests,
            total_items: items.len(),
            pages,
            items,
        };
        Ok(serde_json::to_string_pretty(&out)?)
    }

    async fn connector_send_once(
        &self,
        client: &reqwest::Client,
        spec: &crate::core::connector::ConnectorRequestSpec,
        query: &BTreeMap<String, String>,
    ) -> Result<(u16, String, String)> {
        let method = reqwest::Method::from_bytes(spec.method.as_str().as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid HTTP method: {}", e))?;
        let mut req = client.request(method.clone(), &spec.url);
        for (k, v) in &spec.headers {
            req = req.header(k, v);
        }
        if !query.is_empty() {
            req = req.query(query);
        }
        if method != reqwest::Method::GET && method != reqwest::Method::DELETE {
            if let Some(body) = spec.body.as_ref() {
                req = req.json(body);
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Connector request network error: {}", e))?;
        let status = resp.status().as_u16();
        let request_url = resp.url().to_string();
        let body = resp.text().await.unwrap_or_default();
        Ok((status, body, request_url))
    }

    async fn execute_pipeline_run(&self, arguments: &serde_json::Value) -> Result<String> {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct IdempotencyRecord {
            pipeline: String,
            node: String,
            status: String,
            stored_at: String,
            expires_at: String,
            output: serde_json::Value,
        }

        let allow_privileged = arguments
            .get("allow_privileged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dry_run = arguments
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let spec = if let Some(spec_value) = arguments.get("spec") {
            serde_json::from_value::<crate::core::pipeline::PipelineSpec>(spec_value.clone())
                .map_err(|e| anyhow::anyhow!("Invalid pipeline spec: {}", e))?
        } else {
            let pipeline_name = arguments
                .get("pipeline_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing spec or pipeline_name"))?;
            self.load_saved_pipeline_spec(pipeline_name).await?
        };

        let compiled = crate::core::pipeline::compile_pipeline(&spec)?;

        if dry_run {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "dry_run",
                "pipeline": spec.name,
                "node_count": compiled.node_count,
                "ordered_nodes": compiled.ordered_nodes,
                "warnings": compiled.warnings,
            }))?);
        }

        let mut context = Self::context_map_from_json(arguments.get("context"));
        let now = chrono::Utc::now();
        context.insert("pipeline".to_string(), spec.name.clone());
        context.insert("date".to_string(), now.format("%Y-%m-%d").to_string());
        context.insert("timestamp".to_string(), now.to_rfc3339());

        let nodes_by_id: HashMap<String, crate::core::pipeline::PipelineNode> = spec
            .nodes
            .iter()
            .cloned()
            .map(|n| (n.id.clone(), n))
            .collect();

        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = chrono::Utc::now();

        let mut outputs: HashMap<String, serde_json::Value> = HashMap::new();
        let mut failed_nodes: HashSet<String> = HashSet::new();
        let mut node_reports: Vec<serde_json::Value> = Vec::new();

        for node_id in &compiled.ordered_nodes {
            let node = nodes_by_id
                .get(node_id)
                .ok_or_else(|| anyhow::anyhow!("Missing compiled node '{}'", node_id))?;

            if let Some(dep) = node
                .depends_on
                .iter()
                .find(|dep| failed_nodes.contains(*dep))
                .cloned()
            {
                let msg = format!("Skipped: dependency '{}' failed", dep);
                node_reports.push(serde_json::json!({
                    "node_id": node.id,
                    "status": "skipped_dependency_failed",
                    "reason": msg,
                }));
                failed_nodes.insert(node.id.clone());
                if node.on_error == crate::core::pipeline::NodeErrorMode::Fail {
                    let failed = serde_json::json!({
                        "status": "failed",
                        "pipeline": spec.name.clone(),
                        "run_id": run_id.clone(),
                        "started_at": started_at.to_rfc3339(),
                        "finished_at": chrono::Utc::now().to_rfc3339(),
                        "node_reports": node_reports,
                    });
                    self.persist_pipeline_run(&spec.name, &run_id, &failed)
                        .await?;
                    return Err(anyhow::anyhow!(
                        "Pipeline '{}' failed: node '{}' blocked by failed dependency '{}'",
                        spec.name,
                        node.id,
                        dep
                    ));
                }
                continue;
            }

            let mut node_ctx = context.clone();
            node_ctx.insert("node".to_string(), node.id.clone());
            for dep in &node.depends_on {
                if let Some(dep_out) = outputs.get(dep) {
                    node_ctx.insert(format!("output_{}", dep), dep_out.to_string());
                }
            }

            let rendered_args = Self::render_json_templates(&node.arguments, &node_ctx);
            let action_name = match node.kind {
                crate::core::pipeline::NodeKind::Action => node.action.clone(),
                crate::core::pipeline::NodeKind::ConnectorRequest => {
                    "connector_request".to_string()
                }
                crate::core::pipeline::NodeKind::SignalConsensus => "signal_consensus".to_string(),
            };
            if action_name.trim().is_empty() {
                return Err(anyhow::anyhow!(
                    "Node '{}' resolved to empty action",
                    node.id
                ));
            }
            if action_name == "pipeline_run" || action_name == "pipeline_compile" {
                return Err(anyhow::anyhow!(
                    "Node '{}' uses forbidden nested orchestration action '{}'",
                    node.id,
                    action_name
                ));
            }
            if !allow_privileged && self.action_requires_privileged_allow(&action_name).await {
                let msg = format!(
                    "Node '{}' requires privileged action '{}'; set allow_privileged=true to run",
                    node.id, action_name
                );
                node_reports.push(serde_json::json!({
                    "node_id": node.id,
                    "action": action_name,
                    "status": "blocked_privileged",
                    "error": msg,
                }));
                failed_nodes.insert(node.id.clone());
                if node.on_error == crate::core::pipeline::NodeErrorMode::Fail {
                    let failed = serde_json::json!({
                        "status": "failed",
                        "pipeline": spec.name.clone(),
                        "run_id": run_id.clone(),
                        "started_at": started_at.to_rfc3339(),
                        "finished_at": chrono::Utc::now().to_rfc3339(),
                        "node_reports": node_reports,
                    });
                    self.persist_pipeline_run(&spec.name, &run_id, &failed)
                        .await?;
                    return Err(anyhow::anyhow!("{}", msg));
                }
                continue;
            }

            let mut idempotent_hit = false;
            if let (Some(storage), Some(idem)) = (self.storage.as_ref(), node.idempotency.as_ref())
            {
                let mut idem_ctx = node_ctx.clone();
                idem_ctx.insert("pipeline".to_string(), spec.name.clone());
                idem_ctx.insert("node".to_string(), node.id.clone());
                let idem_key =
                    crate::core::pipeline::render_template(&idem.key_template, &idem_ctx);
                let storage_key = format!("pipeline:idem:{}", idem_key);
                if let Some(raw) = storage.get(&storage_key).await? {
                    if let Ok(record) = serde_json::from_slice::<IdempotencyRecord>(&raw) {
                        if let Ok(expires) =
                            chrono::DateTime::parse_from_rfc3339(&record.expires_at)
                        {
                            if expires.with_timezone(&chrono::Utc) > chrono::Utc::now()
                                && record.status == "completed"
                            {
                                idempotent_hit = true;
                                outputs.insert(node.id.clone(), record.output.clone());
                                node_reports.push(serde_json::json!({
                                    "node_id": node.id,
                                    "action": action_name,
                                    "status": "idempotent_hit",
                                    "attempts": 0,
                                }));
                            } else if expires.with_timezone(&chrono::Utc) <= chrono::Utc::now() {
                                let _ = storage.delete(&storage_key).await;
                            }
                        }
                    }
                }
            }
            if idempotent_hit {
                continue;
            }

            let started = std::time::Instant::now();
            let retry = node.retry.normalized();
            match self
                .execute_action_with_retry(&action_name, &rendered_args, &retry)
                .await
            {
                Ok((output, attempts)) => {
                    let output_json = Self::coerce_to_json(&output);
                    outputs.insert(node.id.clone(), output_json.clone());

                    if let (Some(storage), Some(idem)) =
                        (self.storage.as_ref(), node.idempotency.as_ref())
                    {
                        let mut idem_ctx = node_ctx.clone();
                        idem_ctx.insert("pipeline".to_string(), spec.name.clone());
                        idem_ctx.insert("node".to_string(), node.id.clone());
                        let idem_key =
                            crate::core::pipeline::render_template(&idem.key_template, &idem_ctx);
                        let storage_key = format!("pipeline:idem:{}", idem_key);
                        let ttl_secs = idem.ttl_secs.clamp(60, 30 * 24 * 60 * 60);
                        let now_utc = chrono::Utc::now();
                        let expires_at = now_utc + chrono::Duration::seconds(ttl_secs as i64);
                        let record = IdempotencyRecord {
                            pipeline: spec.name.clone(),
                            node: node.id.clone(),
                            status: "completed".to_string(),
                            stored_at: now_utc.to_rfc3339(),
                            expires_at: expires_at.to_rfc3339(),
                            output: output_json,
                        };
                        storage
                            .set(&storage_key, &serde_json::to_vec(&record)?)
                            .await?;
                    }

                    node_reports.push(serde_json::json!({
                        "node_id": node.id,
                        "action": action_name,
                        "status": "completed",
                        "attempts": attempts,
                        "duration_ms": started.elapsed().as_millis(),
                    }));
                }
                Err(e) => {
                    failed_nodes.insert(node.id.clone());
                    let msg = e.to_string();
                    node_reports.push(serde_json::json!({
                        "node_id": node.id,
                        "action": action_name,
                        "status": if node.on_error == crate::core::pipeline::NodeErrorMode::Continue { "failed_continue" } else { "failed" },
                        "error": msg,
                        "duration_ms": started.elapsed().as_millis(),
                    }));
                    if node.on_error == crate::core::pipeline::NodeErrorMode::Fail {
                        let failed = serde_json::json!({
                            "status": "failed",
                            "pipeline": spec.name.clone(),
                            "run_id": run_id.clone(),
                            "started_at": started_at.to_rfc3339(),
                            "finished_at": chrono::Utc::now().to_rfc3339(),
                            "node_reports": node_reports,
                            "outputs": outputs,
                        });
                        self.persist_pipeline_run(&spec.name, &run_id, &failed)
                            .await?;
                        return Err(anyhow::anyhow!(
                            "Pipeline '{}' failed at node '{}': {}",
                            spec.name,
                            node.id,
                            e
                        ));
                    }
                }
            }
        }

        let mut selected_outputs = serde_json::Map::new();
        if spec.outputs.is_empty() {
            for (k, v) in &outputs {
                selected_outputs.insert(k.clone(), v.clone());
            }
        } else {
            for key in &spec.outputs {
                if let Some(v) = outputs.get(key) {
                    selected_outputs.insert(key.clone(), v.clone());
                }
            }
        }

        let status = if failed_nodes.is_empty() {
            "completed"
        } else {
            "completed_with_errors"
        };
        let result = serde_json::json!({
            "status": status,
            "pipeline": spec.name.clone(),
            "run_id": run_id.clone(),
            "started_at": started_at.to_rfc3339(),
            "finished_at": chrono::Utc::now().to_rfc3339(),
            "node_reports": node_reports,
            "outputs": selected_outputs,
        });
        self.persist_pipeline_run(&spec.name, &run_id, &result)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn load_saved_pipeline_spec(
        &self,
        pipeline_name: &str,
    ) -> Result<crate::core::pipeline::PipelineSpec> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is not available for saved pipelines"))?;
        let key = format!("pipeline:spec:{}", Self::pipeline_key_slug(pipeline_name));
        let raw = storage
            .get(&key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Saved pipeline '{}' not found", pipeline_name))?;
        let spec = serde_json::from_slice::<crate::core::pipeline::PipelineSpec>(&raw)
            .map_err(|e| anyhow::anyhow!("Saved pipeline '{}' is invalid: {}", pipeline_name, e))?;
        Ok(spec)
    }

    async fn persist_pipeline_run(
        &self,
        pipeline_name: &str,
        run_id: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        if let Some(storage) = self.storage.as_ref() {
            let key = format!(
                "pipeline:run:{}:{}",
                Self::pipeline_key_slug(pipeline_name),
                run_id
            );
            storage.set(&key, &serde_json::to_vec(payload)?).await?;
        }
        Ok(())
    }

    async fn execute_action_with_retry(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        retry: &crate::core::pipeline::RetryPolicy,
    ) -> Result<(String, u32)> {
        let policy = retry.normalized();
        let mut attempt = 1u32;
        let mut backoff_ms = policy.initial_backoff_ms;

        loop {
            match std::pin::Pin::from(Box::new(self.execute_action(action_name, arguments))).await {
                Ok(output) => return Ok((output, attempt)),
                Err(err) => {
                    let message = err.to_string();
                    if attempt >= policy.max_attempts
                        || !Self::is_retryable_error(&message, &policy)
                    {
                        return Err(anyhow::anyhow!("{}", message));
                    }
                    Self::sleep_with_backoff(backoff_ms, policy.jitter_ratio).await;
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(policy.max_backoff_ms);
                    attempt += 1;
                }
            }
        }
    }

    async fn action_requires_privileged_allow(&self, action_name: &str) -> bool {
        if matches!(
            action_name,
            "pipeline_run" | "pipeline_compile" | "manage_actions"
        ) {
            return true;
        }
        let (source, capabilities) = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(action_name) else {
                return true;
            };
            (action.info.source.clone(), action.info.capabilities.clone())
        };

        let has_dangerous_cap = capabilities.iter().any(|cap| {
            matches!(
                cap.to_ascii_lowercase().as_str(),
                "shell"
                    | "file_write"
                    | "clipboard_write"
                    | "gmail"
                    | "google_workspace"
                    | "code_execute"
                    | "app_hosting"
                    | "orchestration"
                    | "ssh"
            )
        });

        has_dangerous_cap || (source != ActionSource::System && capabilities.is_empty())
    }

    fn builtin_integrations_for_action(action_name: &str) -> &'static [&'static str] {
        match action_name {
            "gmail_scan" | "gmail_reply" => &["gmail", "google_workspace"],
            "calendar_today" | "calendar_list" | "calendar_create" | "calendar_free" => {
                &["google_calendar", "google_workspace"]
            }
            "google_drive_search"
            | "google_docs_read"
            | "google_sheets_read"
            | "google_chat_list_spaces"
            | "google_admin_list_users"
            | "google_workspace_gws_help"
            | "google_workspace_gws_schema"
            | "google_workspace_gws_skills"
            | "google_workspace_gws_command" => &["google_workspace"],
            _ => &[],
        }
    }

    fn is_builtin_integration_action_enabled(&self, action_name: &str) -> bool {
        let integration_ids = Self::builtin_integrations_for_action(action_name);
        if integration_ids.is_empty() {
            return true;
        }
        integration_ids.iter().any(|integration_id| {
            crate::integrations::effective_integration_enabled(&self.config_dir, integration_id)
        })
    }

    fn pipeline_key_slug(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push('_');
            }
        }
        out.trim_matches('_').to_string()
    }

    fn context_map_from_json(value: Option<&serde_json::Value>) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(obj) = value.and_then(|v| v.as_object()) else {
            return out;
        };
        for (k, v) in obj {
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            out.insert(k.clone(), val);
        }
        out
    }

    fn render_json_templates(
        value: &serde_json::Value,
        context: &BTreeMap<String, String>,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => {
                serde_json::Value::String(crate::core::pipeline::render_template(s, context))
            }
            serde_json::Value::Array(arr) => serde_json::Value::Array(
                arr.iter()
                    .map(|v| Self::render_json_templates(v, context))
                    .collect(),
            ),
            serde_json::Value::Object(obj) => {
                let mut map = serde_json::Map::with_capacity(obj.len());
                for (k, v) in obj {
                    map.insert(k.clone(), Self::render_json_templates(v, context));
                }
                serde_json::Value::Object(map)
            }
            other => other.clone(),
        }
    }

    fn coerce_to_json(output: &str) -> serde_json::Value {
        serde_json::from_str(output)
            .unwrap_or_else(|_| serde_json::Value::String(output.to_string()))
    }

    fn extract_status_code(message: &str) -> Option<u16> {
        for token in message.split(|c: char| !c.is_ascii_digit()) {
            if token.len() == 3 {
                if let Ok(code) = token.parse::<u16>() {
                    if (100..=599).contains(&code) {
                        return Some(code);
                    }
                }
            }
        }
        None
    }

    fn is_retryable_error(message: &str, retry: &crate::core::pipeline::RetryPolicy) -> bool {
        if let Some(status) = Self::extract_status_code(message) {
            return retry.retry_on_status.contains(&status);
        }
        let lower = message.to_ascii_lowercase();
        if lower.contains("missing ")
            || lower.contains("invalid ")
            || lower.contains("unknown action")
            || lower.contains("permission")
            || lower.contains("denied")
            || lower.contains("not found")
        {
            return false;
        }
        true
    }

    async fn sleep_with_backoff(backoff_ms: u64, jitter_ratio: f64) {
        let sleep_ms = if jitter_ratio <= 0.0 {
            backoff_ms.max(25)
        } else {
            use rand::Rng;
            let span = ((backoff_ms as f64) * jitter_ratio).round() as i64;
            if span <= 0 {
                backoff_ms.max(25)
            } else {
                let mut rng = rand::thread_rng();
                let jitter = rng.gen_range(-span..=span);
                ((backoff_ms as i64 + jitter).max(25)) as u64
            }
        };
        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
    }

    async fn execute_tunnel_control(&self, arguments: &serde_json::Value) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status")
            .to_ascii_lowercase();
        let provider = arguments
            .get("provider")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let base_url = crate::core::net::internal_api_base_url();
        let client = crate::core::net::build_internal_control_client(5)
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

        let endpoint = match action.as_str() {
            "start" => "/tunnel/start",
            "stop" => "/tunnel/stop",
            "status" => "/tunnel/status",
            other => {
                return Err(anyhow::anyhow!(
                    "Invalid action '{}'. Use start, stop, or status.",
                    other
                ))
            }
        };

        let mut req = match action.as_str() {
            "status" => client.get(format!("{}{}", base_url, endpoint)),
            "start" => {
                let body = match provider.as_deref() {
                    Some(value) => serde_json::json!({ "provider": value }),
                    None => serde_json::json!({}),
                };
                client.post(format!("{}{}", base_url, endpoint)).json(&body)
            }
            _ => client
                .post(format!("{}{}", base_url, endpoint))
                .json(&serde_json::json!({})),
        };

        if let Ok(mgr) =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))
        {
            if let Ok(Some(key)) = mgr.get_api_key() {
                if !key.trim().is_empty() {
                    req = req.bearer_auth(key);
                }
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reach tunnel controller: {}", e))?;
        let status = resp.status();
        let raw_body = resp.text().await.unwrap_or_default();
        let payload: serde_json::Value =
            serde_json::from_str(&raw_body).unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            let err = payload
                .get("error")
                .and_then(|v| v.as_str())
                .or_else(|| payload.get("message").and_then(|v| v.as_str()))
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().to_string())
                .or_else(|| {
                    let trimmed = raw_body.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.chars().take(400).collect::<String>())
                    }
                })
                .unwrap_or_else(|| format!("HTTP {}", status));
            return Err(anyhow::anyhow!("Tunnel command failed: {}", err));
        }

        match action.as_str() {
            "start" => {
                let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if !url.is_empty() {
                    Ok(format!("Tunnel started.\nExternal URL: {}", url))
                } else {
                    Ok(payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Tunnel is starting; URL pending.")
                        .to_string())
                }
            }
            "stop" => Ok(payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Tunnel stopped.")
                .to_string()),
            _ => {
                let active = payload
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut out = format!(
                    "Tunnel status: {}",
                    if active { "active" } else { "inactive" }
                );
                if let Some(url) = payload.get("url").and_then(|v| v.as_str()) {
                    if !url.is_empty() {
                        out.push_str(&format!("\nExternal URL: {}", url));
                    }
                }
                if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
                    if !err.is_empty() {
                        out.push_str(&format!("\nLast error: {}", err));
                    }
                }
                Ok(out)
            }
        }
    }

    /// Convert HTML to plain text by stripping tags and decoding entities
    fn html_to_text(html: &str) -> String {
        // Remove script and style blocks entirely
        let script_re =
            regex::Regex::new(r"(?is)<(script|style|noscript)[^>]*>.*?</(script|style|noscript)>")
                .unwrap();
        let cleaned = script_re.replace_all(html, "");

        // Remove HTML comments
        let comment_re = regex::Regex::new(r"(?s)<!--.*?-->").unwrap();
        let cleaned = comment_re.replace_all(&cleaned, "");

        // Replace block-level elements with newlines
        let block_re = regex::Regex::new(r"(?i)</(p|div|h[1-6]|li|tr|br|hr)[^>]*>").unwrap();
        let cleaned = block_re.replace_all(&cleaned, "\n");
        let br_re = regex::Regex::new(r"(?i)<br[^>]*/?>").unwrap();
        let cleaned = br_re.replace_all(&cleaned, "\n");

        // Strip all remaining HTML tags
        let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
        let cleaned = tag_re.replace_all(&cleaned, "");

        // Decode common HTML entities
        let text = cleaned
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
            .replace("&#x27;", "'")
            .replace("&#x2F;", "/");

        // Collapse multiple whitespace/newlines
        let ws_re = regex::Regex::new(r"[ \t]+").unwrap();
        let text = ws_re.replace_all(&text, " ");
        let nl_re = regex::Regex::new(r"\n{3,}").unwrap();
        let text = nl_re.replace_all(&text, "\n\n");

        // Trim lines and overall result
        let text: String = text
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n");

        // Truncate to reasonable size (10000 chars)
        if text.len() > 10000 {
            format!(
                "{}...\n\n(content truncated at 10000 characters)",
                &text[..10000]
            )
        } else {
            text.trim().to_string()
        }
    }

    /// Execute an action in WASM sandbox
    async fn execute_wasm(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        // For built-in actions, fall back to native with some wrapping
        match action_name {
            "http_get" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let parsed_url = self.validate_http_get_url(url).await?;

                // Fast-path: try Lightpanda for external URLs (returns clean markdown)
                let has_custom_headers = arguments
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|h| !h.is_empty())
                    .unwrap_or(false);
                let is_loopback = parsed_url
                    .host_str()
                    .map(|h| h == "localhost" || h == "127.0.0.1" || h == "::1")
                    .unwrap_or(true);
                if !is_loopback && !has_custom_headers {
                    match crate::integrations::lightpanda::fetch_markdown(parsed_url.as_str()).await
                    {
                        Ok(markdown) => return Ok(markdown),
                        Err(e) => {
                            tracing::debug!("Lightpanda fast-path skipped for {}: {}", url, e);
                        }
                    }
                }

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(HTTP_GET_TIMEOUT_SECS))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()?;
                let mut req = client.get(parsed_url);
                if let Some(headers) = arguments.get("headers").and_then(|v| v.as_object()) {
                    for (k, v) in headers {
                        let blocked = matches!(
                            k.to_ascii_lowercase().as_str(),
                            "host"
                                | "connection"
                                | "content-length"
                                | "transfer-encoding"
                                | "proxy-authorization"
                                | "x-forwarded-for"
                                | "x-forwarded-host"
                                | "x-real-ip"
                        );
                        if blocked {
                            anyhow::bail!("Header '{}' is not allowed for http_get", k);
                        }
                        if let Some(s) = v.as_str() {
                            req = req.header(k, s);
                        }
                    }
                }
                let response = req.send().await?;
                let body_bytes = response.bytes().await?;
                let body = if body_bytes.len() > HTTP_GET_MAX_BODY_BYTES {
                    format!(
                        "{}\n\n(response truncated at {} bytes)",
                        String::from_utf8_lossy(&body_bytes[..HTTP_GET_MAX_BODY_BYTES]),
                        HTTP_GET_MAX_BODY_BYTES
                    )
                } else {
                    String::from_utf8_lossy(&body_bytes).to_string()
                };

                Ok(body)
            }
            "manage_actions" => {
                let operation = arguments
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'operation' parameter"))?;

                match operation {
                    "create" => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for create"))?;
                        let content = arguments
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'content' for create"))?;
                        if !name
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                        {
                            return Err(anyhow::anyhow!(
                                "Invalid action name. Use kebab-case (e.g., 'check-weather')"
                            ));
                        }
                        if self.actions.read().await.contains_key(name) {
                            return Err(anyhow::anyhow!(
                                "Action '{}' already exists. Use 'update' instead.",
                                name
                            ));
                        }
                        let verdict = self.create_action(name, content, false).await?;
                        let mut msg =
                            format!("Action '{}' created and is immediately available.", name);
                        if let Some(ref v) = verdict {
                            if !v.warnings.is_empty() {
                                msg.push_str(&format!(
                                    "\n⚠️ Security warnings: {}",
                                    v.warnings.join(", ")
                                ));
                            }
                            if !v.allow_load {
                                msg = format!(
                                    "Action '{}' was blocked by security verification: {}",
                                    name,
                                    v.warnings.join(", ")
                                );
                            }
                        }
                        Ok(msg)
                    }
                    "update" => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for update"))?;
                        let content = arguments
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'content' for update"))?;
                        match self.update_action_content(name, content).await? {
                            true => Ok(format!("Action '{}' updated.", name)),
                            false => Err(anyhow::anyhow!(
                                "Cannot update '{}'. System actions are read-only.",
                                name
                            )),
                        }
                    }
                    "delete" => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for delete"))?;
                        match self.delete_action(name).await? {
                            true => Ok(format!("Action '{}' deleted.", name)),
                            false => Err(anyhow::anyhow!(
                                "Cannot delete '{}'. Only custom actions can be deleted.",
                                name
                            )),
                        }
                    }
                    "list" => {
                        let actions = self.list_actions().await?;
                        let list: Vec<String> = actions
                            .iter()
                            .map(|a| {
                                let source = match a.source {
                                    ActionSource::System => "system",
                                    ActionSource::Bundled => "bundled",
                                    ActionSource::Custom => "custom",
                                };
                                format!("- **{}** ({}): {}", a.name, source, a.description)
                            })
                            .collect();
                        Ok(format!(
                            "Available actions ({}):\n{}",
                            actions.len(),
                            list.join("\n")
                        ))
                    }
                    _ => Err(anyhow::anyhow!(
                        "Unknown operation '{}'. Use create, update, delete, or list.",
                        operation
                    )),
                }
            }
            "capability_acquire" => {
                let arguments = Self::enrich_capability_acquisition_arguments(arguments).await;
                let raw_name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for capability acquisition"))?;
                let description = arguments
                    .get("description")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Missing 'description' for capability acquisition")
                    })?;
                let name = Self::normalize_generated_action_name(raw_name);
                if name.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Generated action name is empty after normalization"
                    ));
                }
                let content =
                    self.render_capability_action_markdown(&arguments, &name, description);
                let force = arguments
                    .get("force")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let verdict = self.create_action(&name, &content, force).await?;
                let mut lines = vec![
                    format!("Capability scaffolded as action `{}`.", name),
                    "It is now available immediately in the action catalog.".to_string(),
                ];
                if let Some(kind) = arguments.get("kind").and_then(|value| value.as_str()) {
                    lines.push(format!("Mode: {}", kind));
                }
                if let Some(base_url) = arguments.get("base_url").and_then(|value| value.as_str()) {
                    lines.push(format!("Base URL: {}", base_url));
                }
                if let Some(path) = arguments.get("path").and_then(|value| value.as_str()) {
                    lines.push(format!("Primary endpoint: {}", path));
                }
                if let Some(method) = arguments.get("method").and_then(|value| value.as_str()) {
                    lines.push(format!("Method: {}", method.to_ascii_uppercase()));
                }
                if let Some(secret_name) = arguments
                    .get("auth_secret_name")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                {
                    lines.push(format!(
                        "Expected secret/config key for auth: `{}`.",
                        secret_name
                    ));
                }
                if let Some(source_notes) = arguments
                    .get("source_notes")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                {
                    lines.push(format!("Derived from: {}", source_notes));
                }
                if let Some(verdict) = verdict {
                    if !verdict.warnings.is_empty() {
                        lines.push(format!(
                            "Security review notes: {}",
                            verdict.warnings.join(", ")
                        ));
                    }
                    if !verdict.allow_load {
                        lines.push(
                            "The scaffold was blocked by security policy and was not loaded."
                                .to_string(),
                        );
                    }
                }
                Ok(lines.join("\n"))
            }
            _ => {
                // Check if we have a WASM module for this action
                let actions = self.actions.read().await;
                if let Some(action) = actions.get(action_name) {
                    if let Some(wasm_bytes) = &action.wasm_module {
                        let wasm = wasm_bytes.clone();
                        drop(actions); // Release lock before async call
                        return self.run_wasm_module(&wasm, arguments).await;
                    }
                }
                drop(actions); // Release lock before async call
                               // Fall back to native
                self.execute_native(action_name, arguments).await
            }
        }
    }

    /// Connect to Docker, preferring DOCKER_HOST env var (for socket proxy) over local defaults
    #[cfg(feature = "docker")]
    fn connect_docker() -> Result<bollard::Docker> {
        if let Ok(host) = std::env::var("DOCKER_HOST") {
            tracing::debug!("Connecting to Docker via DOCKER_HOST={}", host);
            bollard::Docker::connect_with_http_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker at {}: {}", host, e))
        } else {
            bollard::Docker::connect_with_local_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))
        }
    }

    /// Execute an action in Docker sandbox
    async fn execute_docker(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        #[cfg(feature = "docker")]
        {
            // Check Docker availability first — fall back to native if unavailable
            let docker_available = Self::connect_docker().is_ok();

            if !docker_available {
                tracing::warn!("Docker not available (socket not found), falling back to native execution for '{}'", action_name);
                return self.execute_native(action_name, arguments).await;
            }

            match action_name {
                "shell" => {
                    const PUBLIC_SHELL_SANDBOX_IMAGE: &str = "alpine:3.20";
                    self.run_isolated_container(
                        PUBLIC_SHELL_SANDBOX_IMAGE,
                        vec![
                            "sh".to_string(),
                            "-c".to_string(),
                            arguments["command"]
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("Missing command"))?
                                .to_string(),
                        ],
                        None,
                        30,
                        ContainerIsolation::Strict,
                    )
                    .await
                }
                "code_execute" => self.execute_code_docker(arguments).await,
                _ => Err(anyhow::anyhow!("Unknown docker action: {}", action_name)),
            }
        }

        #[cfg(not(feature = "docker"))]
        {
            // Fall back to native sandboxed execution when Docker is not available
            if action_name == "code_execute" {
                return self.execute_code_native(arguments).await;
            }
            Err(anyhow::anyhow!(
                "Docker support not enabled. Recompile with --features docker"
            ))
        }
    }

    /// Force-remove a Docker container (stop + remove), ignoring errors.
    /// Guaranteed to not leave containers behind.
    #[cfg(feature = "docker")]
    async fn force_remove_container(docker: &bollard::Docker, id: &str) {
        // Kill first (faster than stop for stuck containers)
        let _ = docker.kill_container::<String>(id, None).await;
        // Stop as fallback (handles already-stopped containers)
        let _ = docker
            .stop_container(id, Some(bollard::container::StopContainerOptions { t: 0 }))
            .await;
        // Force remove — deletes container, volumes, and anonymous volumes
        let _ = docker
            .remove_container(
                id,
                Some(bollard::container::RemoveContainerOptions {
                    force: true,
                    v: true, // Remove anonymous volumes attached to the container
                    ..Default::default()
                }),
            )
            .await;
        tracing::debug!("Cleaned up container {}", &id[..12.min(id.len())]);
    }

    /// Ensure a Docker image is available locally, pulling it if necessary.
    #[cfg(feature = "docker")]
    async fn ensure_image(docker: &bollard::Docker, image: &str) -> Result<()> {
        // Check if image exists locally
        if docker.inspect_image(image).await.is_ok() {
            return Ok(());
        }

        tracing::info!("Pulling Docker image '{}' (first-time download)...", image);

        let mut stream = docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image,
                ..Default::default()
            }),
            None,
            None,
        );

        use futures::StreamExt;
        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = &info.status {
                        tracing::debug!("Pull {}: {}", image, status);
                    }
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to pull image '{}': {}", image, e));
                }
            }
        }

        tracing::info!("Image '{}' pulled successfully", image);
        Ok(())
    }

    /// Run a command in a fully isolated, ephemeral Docker container.
    /// Automatically pulls the image if not available locally.
    /// Container is ALWAYS destroyed after execution — no leftovers.
    #[cfg(feature = "docker")]
    async fn run_isolated_container(
        &self,
        image: &str,
        cmd: Vec<String>,
        env: Option<Vec<String>>,
        timeout_secs: u64,
        isolation: ContainerIsolation,
    ) -> Result<String> {
        let docker = Self::connect_docker()?;

        // Auto-pull image if not available
        Self::ensure_image(&docker, image).await?;

        let host_config = match isolation {
            ContainerIsolation::Strict => {
                // Full lockdown: no network, read-only root, noexec /tmp
                bollard::models::HostConfig {
                    memory: Some(256 * 1024 * 1024),
                    memory_swap: Some(256 * 1024 * 1024),
                    cpu_period: Some(100_000),
                    cpu_quota: Some(50_000),
                    pids_limit: Some(64),
                    network_mode: Some("none".to_string()),
                    readonly_rootfs: Some(true),
                    tmpfs: Some(HashMap::from([(
                        "/tmp".to_string(),
                        "size=64M,noexec".to_string(),
                    )])),
                    auto_remove: Some(false),
                    ..Default::default()
                }
            }
            ContainerIsolation::Standard => {
                // Allows pip/npm install: writable fs, network enabled, but still
                // resource-limited and ephemeral (destroyed after execution)
                bollard::models::HostConfig {
                    memory: Some(512 * 1024 * 1024), // 512MB for installs
                    memory_swap: Some(512 * 1024 * 1024),
                    cpu_period: Some(100_000),
                    cpu_quota: Some(50_000),
                    pids_limit: Some(128), // More PIDs for package managers
                    auto_remove: Some(false),
                    ..Default::default()
                }
            }
        };

        let network_disabled = matches!(isolation, ContainerIsolation::Strict);

        let container_config = bollard::container::Config {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            env,
            host_config: Some(host_config),
            network_disabled: Some(network_disabled),
            working_dir: Some("/tmp".to_string()),
            ..Default::default()
        };

        let container = docker
            .create_container::<String, String>(None, container_config)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create container: {}", e))?;

        let container_id = container.id.clone();
        tracing::info!(
            "Created isolated container {} for code execution",
            &container_id[..12.min(container_id.len())]
        );

        // Start container — if this fails, clean up immediately
        if let Err(e) = docker.start_container::<String>(&container_id, None).await {
            Self::force_remove_container(&docker, &container_id).await;
            return Err(anyhow::anyhow!("Failed to start container: {}", e));
        }

        // Wait for container by polling inspect (more reliable than wait_container stream)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        let exit_code = loop {
            if std::time::Instant::now() > deadline {
                Self::force_remove_container(&docker, &container_id).await;
                return Err(anyhow::anyhow!(
                    "Code execution timed out after {} seconds",
                    timeout_secs
                ));
            }

            match docker.inspect_container(&container_id, None).await {
                Ok(info) => {
                    let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
                    if !running {
                        let code = info.state.as_ref().and_then(|s| s.exit_code).unwrap_or(-1);
                        tracing::debug!(
                            "Container {} exited with code {}",
                            &container_id[..12.min(container_id.len())],
                            code
                        );
                        break code;
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to inspect container: {:?}", e);
                    Self::force_remove_container(&docker, &container_id).await;
                    return Err(anyhow::anyhow!("Container inspection failed: {}", e));
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        };

        // Collect stdout and stderr before cleanup
        let logs = docker
            .logs::<String>(
                &container_id,
                Some(bollard::container::LogsOptions {
                    stdout: true,
                    stderr: true,
                    ..Default::default()
                }),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap_or_default();

        // Always destroy the container — no leftovers
        Self::force_remove_container(&docker, &container_id).await;

        let mut stdout = String::new();
        let mut stderr = String::new();
        for log in &logs {
            match log {
                bollard::container::LogOutput::StdOut { message } => {
                    stdout.push_str(&String::from_utf8_lossy(message));
                }
                bollard::container::LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(message));
                }
                _ => {}
            }
        }

        let result = serde_json::json!({
            "output": stdout,
            "error": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr.clone()) },
            "exit_code": exit_code,
        });

        Ok(serde_json::to_string(&result)?)
    }

    /// Resolve a language name to (docker_image, file_extension, build_cmd, run_cmd).
    /// build_cmd is optional (for compiled languages like Java, Go, Rust, C).
    /// Returns None only if the language is completely unrecognized.
    fn resolve_language(
        lang: &str,
    ) -> Option<(
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static str,
    )> {
        // (image, extension, optional_build_cmd, run_cmd)
        // {file} is replaced with /tmp/code.{ext} at runtime
        match lang {
            // Interpreted
            "python" | "python3" | "py"
                => Some(("python:3-slim", "py", None, "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 {file}")),
            "javascript" | "js" | "node"
                => Some(("node:22-slim", "js", None, "node {file}")),
            "typescript" | "ts"
                => Some(("node:22-slim", "ts", Some("npm i -g tsx 2>/dev/null"), "npx tsx {file}")),
            "bash" | "sh" | "shell"
                => Some(("bash:latest", "sh", None, "bash {file}")),
            "ruby" | "rb"
                => Some(("ruby:3-slim", "rb", None, "ruby {file}")),
            "php"
                => Some(("php:8-cli", "php", None, "php {file}")),
            "perl" | "pl"
                => Some(("perl:5-slim", "pl", None, "perl {file}")),
            "lua"
                => Some(("nickblah/lua:5.4", "lua", None, "lua {file}")),
            "r" | "rlang"
                => Some(("r-base:latest", "R", None, "Rscript {file}")),

            // Compiled
            "java"
                => Some(("eclipse-temurin:21-jdk", "java", Some("javac {file}"), "java -cp /tmp Main")),
            "c"
                => Some(("gcc:latest", "c", Some("gcc {file} -o /tmp/a.out -lm"), "/tmp/a.out")),
            "cpp" | "c++"
                => Some(("gcc:latest", "cpp", Some("g++ {file} -o /tmp/a.out -lm"), "/tmp/a.out")),
            "go" | "golang"
                => Some(("golang:1-bookworm", "go", None, "go run {file}")),
            "rust" | "rs"
                => Some(("rust:1-slim-bookworm", "rs", Some("rustc {file} -o /tmp/a.out"), "/tmp/a.out")),
            "swift"
                => Some(("swift:latest", "swift", None, "swift {file}")),
            "kotlin" | "kt"
                => Some(("zenika/kotlin:latest", "kt", Some("kotlinc {file} -include-runtime -d /tmp/out.jar 2>/dev/null"), "java -jar /tmp/out.jar")),

            // Jupyter notebook — execute in-place and output results
            "jupyter" | "notebook" | "ipynb"
                => Some(("python:3-slim", "ipynb",
                    Some("PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 pip install -q jupyter nbconvert nbformat matplotlib pandas numpy scikit-learn seaborn 2>/dev/null"),
                    "jupyter nbconvert --to notebook --execute --inplace {file} 2>&1 && python3 -c \"import json; nb=json.load(open('{file}')); [print(o.get('text','')) for c in nb['cells'] for o in c.get('outputs',[]) if o.get('output_type')=='stream']\" ")),

            _ => None,
        }
    }

    /// Detect non-stdlib Python imports and return a pip install command.
    /// Scans `import X` and `from X import` statements, filters out stdlib modules.
    fn detect_python_deps(code: &str) -> String {
        // Python stdlib modules (comprehensive but not exhaustive — errs on side of not installing)
        const STDLIB: &[&str] = &[
            "abc",
            "aifc",
            "argparse",
            "array",
            "ast",
            "asynchat",
            "asyncio",
            "asyncore",
            "atexit",
            "base64",
            "bdb",
            "binascii",
            "binhex",
            "bisect",
            "builtins",
            "bz2",
            "calendar",
            "cgi",
            "cgitb",
            "chunk",
            "cmath",
            "cmd",
            "code",
            "codecs",
            "codeop",
            "collections",
            "colorsys",
            "compileall",
            "concurrent",
            "configparser",
            "contextlib",
            "contextvars",
            "copy",
            "copyreg",
            "cProfile",
            "crypt",
            "csv",
            "ctypes",
            "curses",
            "dataclasses",
            "datetime",
            "dbm",
            "decimal",
            "difflib",
            "dis",
            "distutils",
            "doctest",
            "email",
            "encodings",
            "enum",
            "errno",
            "faulthandler",
            "fcntl",
            "filecmp",
            "fileinput",
            "fnmatch",
            "formatter",
            "fractions",
            "ftplib",
            "functools",
            "gc",
            "getopt",
            "getpass",
            "gettext",
            "glob",
            "grp",
            "gzip",
            "hashlib",
            "heapq",
            "hmac",
            "html",
            "http",
            "idlelib",
            "imaplib",
            "imghdr",
            "imp",
            "importlib",
            "inspect",
            "io",
            "ipaddress",
            "itertools",
            "json",
            "keyword",
            "lib2to3",
            "linecache",
            "locale",
            "logging",
            "lzma",
            "mailbox",
            "mailcap",
            "marshal",
            "math",
            "mimetypes",
            "mmap",
            "modulefinder",
            "multiprocessing",
            "netrc",
            "nis",
            "nntplib",
            "numbers",
            "operator",
            "optparse",
            "os",
            "ossaudiodev",
            "parser",
            "pathlib",
            "pdb",
            "pickle",
            "pickletools",
            "pipes",
            "pkgutil",
            "platform",
            "plistlib",
            "poplib",
            "posix",
            "posixpath",
            "pprint",
            "profile",
            "pstats",
            "pty",
            "pwd",
            "py_compile",
            "pyclbr",
            "pydoc",
            "queue",
            "quopri",
            "random",
            "re",
            "readline",
            "reprlib",
            "resource",
            "rlcompleter",
            "runpy",
            "sched",
            "secrets",
            "select",
            "selectors",
            "shelve",
            "shlex",
            "shutil",
            "signal",
            "site",
            "smtpd",
            "smtplib",
            "sndhdr",
            "socket",
            "socketserver",
            "ssl",
            "stat",
            "statistics",
            "string",
            "stringprep",
            "struct",
            "subprocess",
            "sunau",
            "symtable",
            "sys",
            "sysconfig",
            "syslog",
            "tabnanny",
            "tarfile",
            "telnetlib",
            "tempfile",
            "termios",
            "test",
            "textwrap",
            "threading",
            "time",
            "timeit",
            "tkinter",
            "token",
            "tokenize",
            "trace",
            "traceback",
            "tracemalloc",
            "tty",
            "turtle",
            "turtledemo",
            "types",
            "typing",
            "unicodedata",
            "unittest",
            "urllib",
            "uu",
            "uuid",
            "venv",
            "warnings",
            "wave",
            "weakref",
            "webbrowser",
            "winreg",
            "winsound",
            "wsgiref",
            "xdrlib",
            "xml",
            "xmlrpc",
            "zipapp",
            "zipfile",
            "zipimport",
            "zlib",
            "_thread",
            "__future__",
        ];

        // Well-known pip package name mappings (import name -> pip package)
        fn pip_name(module: &str) -> String {
            match module {
                "PIL" | "Pillow" => "Pillow".to_string(),
                "cv2" => "opencv-python".to_string(),
                "sklearn" | "scikit_learn" => "scikit-learn".to_string(),
                "bs4" => "beautifulsoup4".to_string(),
                "yaml" => "pyyaml".to_string(),
                "dotenv" => "python-dotenv".to_string(),
                "gi" => "PyGObject".to_string(),
                "attr" | "attrs" => "attrs".to_string(),
                "dateutil" => "python-dateutil".to_string(),
                "jwt" => "PyJWT".to_string(),
                "crypto" | "Crypto" => "pycryptodome".to_string(),
                "serial" => "pyserial".to_string(),
                "usb" => "pyusb".to_string(),
                "wx" => "wxPython".to_string(),
                "skimage" => "scikit-image".to_string(),
                _ => module.to_string(),
            }
        }

        // Valid pip package names: alphanumeric, hyphens, underscores, dots
        fn is_valid_package_name(name: &str) -> bool {
            !name.is_empty()
                && name.len() <= 100
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
                && name.chars().next().is_some_and(|c| c.is_alphanumeric())
        }

        let mut deps = std::collections::HashSet::new();

        for line in code.lines() {
            let line = line.trim();
            // Skip lines inside strings/comments (heuristic: skip if line starts with #, ', ", or is indented code with non-import content)
            if line.starts_with('#') || line.starts_with('"') || line.starts_with('\'') {
                continue;
            }
            // import X, Y, Z
            if let Some(rest) = line.strip_prefix("import ") {
                for part in rest.split(',') {
                    let module = part
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .split('.')
                        .next()
                        .unwrap_or("");
                    if !module.is_empty()
                        && !STDLIB.contains(&module)
                        && is_valid_package_name(module)
                    {
                        deps.insert(pip_name(module));
                    }
                }
            }
            // from X import ...
            else if let Some(rest) = line.strip_prefix("from ") {
                let module = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("");
                if !module.is_empty() && !STDLIB.contains(&module) && is_valid_package_name(module)
                {
                    deps.insert(pip_name(module));
                }
            }
        }

        if deps.is_empty() {
            return String::new();
        }

        let dep_list: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        tracing::info!("Auto-detected Python deps: {:?}", dep_list);
        format!(
            "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 pip install -q {} && ",
            dep_list.join(" ")
        )
    }

    /// Detect non-builtin Node.js requires/imports and return an npm install command.
    fn detect_node_deps(code: &str) -> String {
        // Node.js built-in modules
        const BUILTINS: &[&str] = &[
            "assert",
            "buffer",
            "child_process",
            "cluster",
            "console",
            "constants",
            "crypto",
            "dgram",
            "dns",
            "domain",
            "events",
            "fs",
            "http",
            "https",
            "module",
            "net",
            "os",
            "path",
            "perf_hooks",
            "process",
            "punycode",
            "querystring",
            "readline",
            "repl",
            "stream",
            "string_decoder",
            "sys",
            "timers",
            "tls",
            "tty",
            "url",
            "util",
            "v8",
            "vm",
            "worker_threads",
            "zlib",
        ];

        let mut deps = std::collections::HashSet::new();

        for line in code.lines() {
            let line = line.trim();
            // require('pkg') or require("pkg")
            if line.contains("require(") {
                for cap in line.split("require(").skip(1) {
                    let pkg = cap
                        .trim_start_matches(['\'', '"'])
                        .split(['\'', '"'])
                        .next()
                        .unwrap_or("");
                    let root = pkg.split('/').next().unwrap_or("");
                    if !root.is_empty() && !root.starts_with('.') && !BUILTINS.contains(&root) {
                        deps.insert(root.to_string());
                    }
                }
            }
            // import ... from 'pkg'
            if line.starts_with("import ") {
                if let Some(from_part) = line.rsplit("from ").next() {
                    let pkg = from_part.trim().trim_matches([' ', '\'', '"', ';']);
                    let root = pkg.split('/').next().unwrap_or("");
                    if !root.is_empty() && !root.starts_with('.') && !BUILTINS.contains(&root) {
                        deps.insert(root.to_string());
                    }
                }
            }
        }

        if deps.is_empty() {
            return String::new();
        }

        let dep_list: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        tracing::info!("Auto-detected Node.js deps: {:?}", dep_list);
        format!(
            "npm install --no-fund --no-audit -q {} 2>/dev/null && ",
            dep_list.join(" ")
        )
    }

    /// Execute code in an isolated Docker container.
    /// Supports any language with a Docker image — auto-pulls if needed.
    /// Container is ephemeral — fully destroyed after execution.
    /// Output files (images, CSVs, etc.) are extracted before container cleanup.
    #[cfg(feature = "docker")]
    async fn execute_code_docker(&self, arguments: &serde_json::Value) -> Result<String> {
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code_raw = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

        // Strip Jupyter magic commands (!pip, !apt, !conda, %pip, %conda, etc.)
        // LLMs often generate these in regular Python scripts — our auto-dependency
        // detection handles installs, so these lines are unnecessary and cause SyntaxError.
        let code = if matches!(language.as_str(), "python" | "python3" | "py") {
            let cleaned: Vec<&str> = code_raw
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("!pip ")
                        && !trimmed.starts_with("!pip3 ")
                        && !trimmed.starts_with("!apt ")
                        && !trimmed.starts_with("!apt-get ")
                        && !trimmed.starts_with("!conda ")
                        && !trimmed.starts_with("%pip ")
                        && !trimmed.starts_with("%conda ")
                        && !trimmed.starts_with("!sudo ")
                })
                .collect();
            cleaned.join("\n")
        } else {
            code_raw.to_string()
        };
        let code = code.as_str();

        let (image, ext, build_cmd, run_cmd) = Self::resolve_language(&language)
            .ok_or_else(|| anyhow::anyhow!(
                "Unsupported language '{}'. Supported: python, javascript, typescript, bash, ruby, php, perl, lua, r, java, c, cpp, go, rust, swift, kotlin",
                language
            ))?;

        let code_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, code);
        let file_path = format!("/tmp/code.{}", ext);
        // Java needs the file named Main.java
        let file_path = if language == "java" {
            "/tmp/Main.java".to_string()
        } else {
            file_path
        };

        // Build file injection commands for user-attached files
        // Files are read from host, base64-encoded, and decoded into /data/ inside container
        let mut file_inject_cmds = String::new();
        if let Some(files_arr) = arguments.get("files").and_then(|v| v.as_array()) {
            file_inject_cmds.push_str("mkdir -p /data && ");
            for file_val in files_arr {
                if let Some(local_path) = file_val.as_str() {
                    let path = std::path::Path::new(local_path);
                    // Validate path exists and get filename
                    if let Ok(data) = tokio::fs::read(path).await {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                        let data_b64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &data,
                        );
                        file_inject_cmds.push_str(&format!(
                            "echo '{}' | base64 -d > /data/{} && ",
                            data_b64, filename
                        ));
                        tracing::info!(
                            "Injecting file into container: {} ({} bytes)",
                            filename,
                            data.len()
                        );
                    } else {
                        tracing::warn!("Could not read attached file: {}", local_path);
                    }
                }
            }
        }

        // Auto-detect dependencies for Python: scan imports, pip install non-stdlib packages
        let auto_install_cmd = if matches!(language.as_str(), "python" | "python3" | "py") {
            Self::detect_python_deps(code)
        } else if matches!(
            language.as_str(),
            "javascript" | "js" | "node" | "typescript" | "ts"
        ) {
            Self::detect_node_deps(code)
        } else {
            String::new()
        };

        let run = run_cmd.replace("{file}", &file_path);
        let main_cmd = if let Some(build) = build_cmd {
            let build = build.replace("{file}", &file_path);
            format!(
                "{}{}echo '{}' | base64 -d > {} && {} && {}",
                file_inject_cmds, auto_install_cmd, code_b64, file_path, build, run
            )
        } else {
            format!(
                "{}{}echo '{}' | base64 -d > {} && {}",
                file_inject_cmds, auto_install_cmd, code_b64, file_path, run
            )
        };

        // Append file extraction: finds files created/modified during execution,
        // excludes build artifacts, base64-encodes each file (up to 5MB) and
        // outputs them with markers so we can extract before container dies.
        // For notebooks (.ipynb), we also capture the executed notebook itself.
        let is_notebook = ext == "ipynb";
        let notebook_extra = if is_notebook {
            // Also extract the executed notebook file
            format!(
                r#" echo "FILE:$(basename {file}):$(base64 {file} | tr -d '\n')";"#,
                file = file_path
            )
        } else {
            String::new()
        };
        let shell_cmd = format!(
            r#"{}; __AGENTARK_EXIT=$?; echo; echo '__AGENTARK_OUTPUT_FILES__';{} find /tmp -maxdepth 2 -type f ! -name 'code.*' ! -name 'a.out' ! -name 'Main.*' ! -name '*.class' ! -name 'out.jar' ! -name '*.ipynb' -newer {} 2>/dev/null | head -20 | while IFS= read -r __f; do __sz=$(stat -c%s "$__f" 2>/dev/null || echo 999999999); if [ "$__sz" -lt 5242880 ]; then echo "FILE:$(basename "$__f"):$(base64 "$__f" | tr -d '\n')"; fi; done; exit $__AGENTARK_EXIT"#,
            main_cmd, notebook_extra, file_path
        );

        // Notebooks get 10 min (install deps + execute all cells + ML training)
        // Compiled languages get 120s (build + run), interpreted get 60s
        let timeout = if is_notebook {
            600
        } else if build_cmd.is_some() {
            120
        } else {
            60
        };

        // Optional env vars for execution (resolved placeholders are already applied by the runtime).
        let env_vec: Option<Vec<String>> = arguments
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| format!("{}={}", k, s)))
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());

        let raw_result = self
            .run_isolated_container(
                image,
                vec!["sh".to_string(), "-c".to_string(), shell_cmd],
                env_vec,
                timeout,
                ContainerIsolation::Standard,
            )
            .await?;

        // Parse result and extract output files from stdout
        let parsed: serde_json::Value = serde_json::from_str(&raw_result)?;
        let output = parsed["output"].as_str().unwrap_or("");

        let exec_id = uuid::Uuid::new_v4().to_string();
        let output_dir = self.data_dir().join("outputs").join(&exec_id);

        let (user_output, saved_files) = if let Some(marker_pos) =
            output.find("__AGENTARK_OUTPUT_FILES__")
        {
            let user_output = output[..marker_pos].trim_end().to_string();
            let files_section = &output[marker_pos..];

            let mut saved = Vec::new();

            // Save the code file first so user can download it
            {
                let _ = tokio::fs::create_dir_all(&output_dir).await;
                let code_filename = format!("code.{}", ext);
                let code_path = output_dir.join(&code_filename);
                if tokio::fs::write(&code_path, code).await.is_ok() {
                    saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
                    tracing::debug!("Saved code file: {}", code_path.display());
                }
            }

            // Extract output files from container stdout
            for line in files_section.lines() {
                if let Some(rest) = line.strip_prefix("FILE:") {
                    let parts: Vec<&str> = rest.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        let filename = parts[0];
                        let b64_data = parts[1];
                        use base64::Engine as _;
                        if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(b64_data)
                        {
                            let out_path = output_dir.join(filename);
                            if let Ok(()) = tokio::fs::write(&out_path, &data).await {
                                let web_path = format!("/api/outputs/{}/{}", exec_id, filename);
                                saved.push(web_path);
                                tracing::info!(
                                    "Extracted output file: {} ({} bytes)",
                                    out_path.display(),
                                    data.len()
                                );
                            }
                        }
                    }
                }
            }

            (user_output, saved)
        } else {
            // No file marker found — still save the code file
            let mut saved = Vec::new();
            let _ = tokio::fs::create_dir_all(&output_dir).await;
            let code_filename = format!("code.{}", ext);
            let code_path = output_dir.join(&code_filename);
            if tokio::fs::write(&code_path, code).await.is_ok() {
                saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
            }
            (output.to_string(), saved)
        };

        // Build final result with file paths
        let mut result = serde_json::json!({
            "output": user_output,
            "error": parsed.get("error").cloned().unwrap_or(serde_json::Value::Null),
            "exit_code": parsed.get("exit_code").cloned().unwrap_or(serde_json::json!(-1)),
        });

        if !saved_files.is_empty() {
            result["files"] = serde_json::json!(saved_files);
        }

        Ok(serde_json::to_string(&result)?)
    }

    /// Fallback: execute code natively in an isolated temp directory (no Docker)
    async fn execute_code_native(&self, arguments: &serde_json::Value) -> Result<String> {
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

        // Native fallback: try to find the runtime on the host
        let (program, args): (&str, Vec<String>) = match language.as_str() {
            "python" | "python3" | "py" => ("python3", vec!["-c".to_string(), code.to_string()]),
            "javascript" | "js" | "node" => ("node", vec!["-e".to_string(), code.to_string()]),
            "bash" | "sh" | "shell" => ("bash", vec!["-c".to_string(), code.to_string()]),
            "ruby" | "rb" => ("ruby", vec!["-e".to_string(), code.to_string()]),
            "php" => ("php", vec!["-r".to_string(), code.to_string()]),
            "perl" | "pl" => ("perl", vec!["-e".to_string(), code.to_string()]),
            _ => {
                return Err(anyhow::anyhow!(
                "Native fallback only supports interpreted languages. Docker required for '{}'.",
                language
            ))
            }
        };

        // Create isolated temp directory for execution
        let temp_dir = std::env::temp_dir().join(format!("agentark-exec-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await?;

        // Execute with timeout, cleared env, isolated working dir
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&args)
            .current_dir(&temp_dir)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", temp_dir.to_string_lossy().to_string())
            .env("TMPDIR", temp_dir.to_string_lossy().to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (key, value) in Self::collect_native_env_overrides(arguments)? {
            cmd.env(key, value);
        }

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await;

        // Always clean up the temp directory
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        let output = result
            .map_err(|_| anyhow::anyhow!("Code execution timed out after 30 seconds"))?
            .map_err(|e| anyhow::anyhow!("Failed to execute {}: {}", program, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        let result = serde_json::json!({
            "output": stdout,
            "error": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr) },
            "exit_code": exit_code,
        });

        Ok(serde_json::to_string(&result)?)
    }

    /// List available actions
    pub async fn list_actions(&self) -> Result<Vec<ActionDef>> {
        let actions = self.actions.read().await;
        Ok(actions.values().map(|s| s.info.clone()).collect())
    }

    /// List only actions that are currently executable by the agent.
    /// Non-system actions honor the disabled set; integration-backed system actions honor
    /// the integration enable/disable toggle.
    pub async fn list_enabled_actions(&self) -> Result<Vec<ActionDef>> {
        let actions = self.actions.read().await;
        let disabled = self.disabled_actions.read().await;
        Ok(actions
            .values()
            .filter(|loaded| {
                if loaded.info.source == ActionSource::System {
                    self.is_builtin_integration_action_enabled(&loaded.info.name)
                } else {
                    !disabled.contains(loaded.info.name.as_str())
                }
            })
            .map(|loaded| loaded.info.clone())
            .collect())
    }

    /// Returns true if an action is enabled (not in the disabled set).
    pub async fn is_action_enabled(&self, name: &str) -> bool {
        let disabled = self.disabled_actions.read().await;
        !disabled.contains(name)
    }

    /// Enable or disable an action without deleting it.
    /// - System actions cannot be disabled.
    pub async fn set_action_enabled(&self, name: &str, enabled: bool) -> Result<bool> {
        let source = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => action.info.source.clone(),
                None => return Ok(false),
            }
        };

        if source == ActionSource::System {
            return Ok(false);
        }

        {
            let mut disabled = self.disabled_actions.write().await;
            if enabled {
                disabled.remove(name);
            } else {
                disabled.insert(name.to_string());
            }
        }

        self.save_disabled_actions().await?;
        Ok(true)
    }

    /// Get action count
    pub async fn action_count(&self) -> usize {
        self.actions.read().await.len()
    }

    /// Get action info and content for editing
    pub async fn get_action_content(&self, name: &str) -> Result<Option<(ActionDef, String)>> {
        let actions = self.actions.read().await;
        if let Some(action) = actions.get(name) {
            let info = action.info.clone();
            let file_path = action.info.file_path.clone();
            let workflow = action.workflow_content.clone();
            drop(actions); // Release lock before async file I/O

            if let Some(ref fp) = file_path {
                let content = tokio::fs::read_to_string(fp).await?;
                return Ok(Some((info, content)));
            } else if let Some(wf) = workflow {
                return Ok(Some((info, wf)));
            }
            return Ok(Some((info, String::new())));
        }
        Ok(None)
    }

    fn preferred_skill_markdown_path(dir: &Path) -> std::path::PathBuf {
        dir.join("SKILL.md")
    }

    fn resolve_skill_markdown_path(dir: &Path) -> Option<std::path::PathBuf> {
        let skill_md = dir.join("SKILL.md");
        if skill_md.exists() {
            return Some(skill_md);
        }
        let legacy_action_md = dir.join("ACTION.md");
        if legacy_action_md.exists() {
            return Some(legacy_action_md);
        }
        None
    }

    /// Update action content - for bundled actions, creates a custom copy first
    pub async fn update_action_content(&self, name: &str, content: &str) -> Result<bool> {
        let actions = self.actions.read().await;
        if let Some(action) = actions.get(name) {
            // System actions cannot be edited
            if action.info.source == ActionSource::System {
                return Ok(false);
            }

            // For Bundled actions, create a custom copy in the data directory
            if action.info.source == ActionSource::Bundled {
                drop(actions); // Release lock before async file I/O

                // Create custom action directory
                let custom_action_dir = self.actions_dir.join(name);
                tokio::fs::create_dir_all(&custom_action_dir).await?;

                // Write content to custom location
                let custom_action_file = Self::preferred_skill_markdown_path(&custom_action_dir);
                tokio::fs::write(&custom_action_file, content).await?;

                // Re-sign the action manifest after edit
                if let Some(ref guard) = self.action_guard {
                    if let Err(e) = guard.resign_action(&custom_action_dir, name).await {
                        tracing::warn!("Failed to re-sign action '{}': {}", name, e);
                    }
                }

                tracing::info!(
                    "Created custom copy of bundled action '{}' at {:?}",
                    name,
                    custom_action_file
                );

                // Update the in-memory action to point to the new custom location
                let mut actions = self.actions.write().await;
                if let Some(action) = actions.get_mut(name) {
                    action.info.source = ActionSource::Custom;
                    action.info.file_path = Some(custom_action_file.to_string_lossy().to_string());
                    action.workflow_content = Some(content.to_string());
                }

                return Ok(true);
            }

            // Custom actions - edit in place and update in-memory
            if let Some(ref file_path) = action.info.file_path {
                let fp = file_path.clone();
                drop(actions); // Release lock before async file I/O
                tokio::fs::write(&fp, content).await?;

                // Re-sign the action manifest after edit
                let action_dir = std::path::Path::new(&fp)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                if let Some(ref guard) = self.action_guard {
                    if let Err(e) = guard.resign_action(action_dir, name).await {
                        tracing::warn!("Failed to re-sign action '{}': {}", name, e);
                    }
                }

                // Re-parse and update in-memory action
                if let Ok((new_info, new_content, _frontmatter)) = self
                    .parse_action_md(std::path::Path::new(&fp), ActionSource::Custom)
                    .await
                {
                    let mut actions = self.actions.write().await;
                    if let Some(action) = actions.get_mut(name) {
                        action.info = new_info;
                        action.workflow_content = Some(new_content);
                    }
                }

                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Create a new custom action with security verification
    /// Returns the security verdict so the caller can present it to the user.
    /// If `force` is true, the action is loaded even if security blocks it.
    pub async fn create_action(
        &self,
        name: &str,
        content: &str,
        force: bool,
    ) -> Result<Option<crate::security::action_guard::ActionSecurityVerdict>> {
        let action_dir = self.actions_dir.join(name);
        tokio::fs::create_dir_all(&action_dir).await?;

        let action_file = Self::preferred_skill_markdown_path(&action_dir);
        tokio::fs::write(&action_file, content).await?;

        // Sign the new action manifest
        if let Some(ref guard) = self.action_guard {
            if let Err(e) = guard.resign_action(&action_dir, name).await {
                tracing::warn!("Failed to sign new action '{}': {}", name, e);
            }
        }

        // Immediately register into runtime (no restart needed)
        match self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
        {
            Ok((info, workflow_content, frontmatter)) => {
                // Security evaluation for newly created actions
                let verdict = if let Some(ref guard) = self.action_guard {
                    match guard
                        .evaluate_action(&action_dir, name, &workflow_content, &frontmatter)
                        .await
                    {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!(
                                "Security check failed for new action '{}': {} — loading anyway",
                                name,
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                // Check if blocked
                let blocked = verdict.as_ref().map(|v| !v.allow_load).unwrap_or(false);

                if blocked && !force {
                    tracing::warn!(
                        "New action '{}' BLOCKED by security guard: {:?}",
                        name,
                        verdict.as_ref().map(|v| &v.warnings)
                    );
                    // Delete the saved files since we're not loading it
                    let _ = tokio::fs::remove_dir_all(&action_dir).await;
                    return Ok(verdict);
                }

                if let Some(ref v) = verdict {
                    for w in &v.warnings {
                        tracing::warn!("Action '{}': {}", name, w);
                    }
                    if blocked && force {
                        tracing::warn!("Action '{}' force-loaded despite security warnings", name);
                    }
                }

                self.register_workflow_action(info, workflow_content).await;
                tracing::info!(
                    "Created and registered action '{}' at {:?}",
                    name,
                    action_file
                );
                Ok(verdict)
            }
            Err(e) => {
                tracing::warn!("Created action file but failed to parse: {}", e);
                Err(anyhow::anyhow!("Failed to parse action: {}", e))
            }
        }
    }

    fn capability_acquire_required_inputs(arguments: &serde_json::Value) -> Vec<String> {
        arguments
            .get("required_inputs")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn capability_string_argument(arguments: &serde_json::Value, key: &str) -> Option<String> {
        arguments
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    fn merge_capability_string_field(
        root: &mut serde_json::Map<String, serde_json::Value>,
        key: &str,
        value: Option<String>,
    ) {
        let Some(value) = value
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
        else {
            return;
        };
        let should_set = root
            .get(key)
            .and_then(|existing| existing.as_str())
            .map(|existing| existing.trim().is_empty())
            .unwrap_or(true);
        if should_set {
            root.insert(key.to_string(), serde_json::Value::String(value));
        }
    }

    fn merge_capability_required_inputs(
        root: &mut serde_json::Map<String, serde_json::Value>,
        values: Vec<String>,
    ) {
        let mut merged = root
            .get("required_inputs")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for value in values {
            let trimmed = value.trim();
            if trimmed.is_empty() || merged.iter().any(|existing| existing == trimmed) {
                continue;
            }
            merged.push(trimmed.to_string());
        }

        if !merged.is_empty() {
            root.insert(
                "required_inputs".to_string(),
                serde_json::Value::Array(
                    merged
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect::<Vec<_>>(),
                ),
            );
        }
    }

    fn capability_base_url_from_url(raw_url: &str) -> Option<String> {
        let parsed = reqwest::Url::parse(raw_url).ok()?;
        let host = parsed.host_str()?;
        let mut base = format!("{}://{}", parsed.scheme(), host);
        if let Some(port) = parsed.port() {
            base.push(':');
            base.push_str(&port.to_string());
        }
        Some(base)
    }

    fn first_openapi_operation(
        paths: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<(String, String, serde_json::Map<String, serde_json::Value>)> {
        for (path, item) in paths {
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            for method in ["get", "post", "put", "patch", "delete"] {
                let Some(operation) = item_obj.get(method).and_then(|value| value.as_object())
                else {
                    continue;
                };
                return Some((path.clone(), method.to_string(), operation.clone()));
            }
        }
        None
    }

    fn derive_capability_from_openapi_json(
        spec_text: &str,
        source_url: Option<&str>,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let spec = serde_json::from_str::<serde_json::Value>(spec_text).ok()?;
        let mut derived = serde_json::Map::new();

        if let Some(server_url) = spec
            .get("servers")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.get("url"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            derived.insert(
                "base_url".to_string(),
                serde_json::Value::String(server_url),
            );
        } else if let Some(host) = spec.get("host").and_then(|value| value.as_str()) {
            let scheme = spec
                .get("schemes")
                .and_then(|value| value.as_array())
                .and_then(|items| items.first())
                .and_then(|value| value.as_str())
                .unwrap_or("https");
            let base_path = spec
                .get("basePath")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim_end_matches('/');
            derived.insert(
                "base_url".to_string(),
                serde_json::Value::String(format!("{}://{}{}", scheme, host, base_path)),
            );
        } else if let Some(url) = source_url.and_then(Self::capability_base_url_from_url) {
            derived.insert("base_url".to_string(), serde_json::Value::String(url));
        }

        if let Some((path, method, operation)) = spec
            .get("paths")
            .and_then(|value| value.as_object())
            .and_then(Self::first_openapi_operation)
        {
            derived.insert("path".to_string(), serde_json::Value::String(path.clone()));
            derived.insert(
                "method".to_string(),
                serde_json::Value::String(method.clone()),
            );

            let mut required_inputs = Vec::new();
            for parameter in operation
                .get("parameters")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
            {
                let is_required = parameter
                    .get("required")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if !is_required {
                    continue;
                }
                if let Some(name) = parameter.get("name").and_then(|value| value.as_str()) {
                    required_inputs.push(name.to_string());
                }
            }
            if operation
                .get("requestBody")
                .and_then(|value| value.get("required"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                required_inputs.push("body".to_string());
            }
            if !required_inputs.is_empty() {
                derived.insert(
                    "required_inputs".to_string(),
                    serde_json::Value::Array(
                        required_inputs
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect::<Vec<_>>(),
                    ),
                );
            }

            let mut response_notes = operation
                .get("summary")
                .and_then(|value| value.as_str())
                .or_else(|| {
                    operation
                        .get("description")
                        .and_then(|value| value.as_str())
                })
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            if response_notes.is_empty() {
                if let Some(description) = operation
                    .get("responses")
                    .and_then(|value| value.as_object())
                    .and_then(|responses| {
                        ["200", "201", "default"]
                            .iter()
                            .find_map(|code| responses.get(*code))
                    })
                    .and_then(|value| value.get("description"))
                    .and_then(|value| value.as_str())
                {
                    response_notes = description.trim().to_string();
                }
            }
            if !response_notes.is_empty() {
                derived.insert(
                    "response_notes".to_string(),
                    serde_json::Value::String(response_notes),
                );
            }
        }

        let security_schemes = spec
            .get("components")
            .and_then(|value| value.get("securitySchemes"))
            .and_then(|value| value.as_object())
            .or_else(|| {
                spec.get("securityDefinitions")
                    .and_then(|value| value.as_object())
            });
        if let Some((scheme_name, scheme)) =
            security_schemes.and_then(|schemes| schemes.iter().next())
        {
            let scheme_type = scheme
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            match scheme_type.as_str() {
                "http" => {
                    let auth_scheme = scheme
                        .get("scheme")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if auth_scheme == "bearer" {
                        derived.insert(
                            "auth_type".to_string(),
                            serde_json::Value::String("bearer".to_string()),
                        );
                    } else if auth_scheme == "basic" {
                        derived.insert(
                            "auth_type".to_string(),
                            serde_json::Value::String("basic".to_string()),
                        );
                    }
                }
                "apikey" | "api_key" => {
                    let location = scheme
                        .get("in")
                        .and_then(|value| value.as_str())
                        .unwrap_or("header")
                        .to_ascii_lowercase();
                    let auth_type = if location == "query" {
                        "api_key_query"
                    } else {
                        "api_key_header"
                    };
                    derived.insert(
                        "auth_type".to_string(),
                        serde_json::Value::String(auth_type.to_string()),
                    );
                    if let Some(name) = scheme.get("name").and_then(|value| value.as_str()) {
                        derived.insert(
                            "auth_header_name".to_string(),
                            serde_json::Value::String(name.to_string()),
                        );
                    }
                }
                "oauth2" => {
                    derived.insert(
                        "auth_type".to_string(),
                        serde_json::Value::String("oauth2".to_string()),
                    );
                }
                _ => {}
            }
            derived.insert(
                "auth_secret_name".to_string(),
                serde_json::Value::String(format!(
                    "{}_auth",
                    Self::normalize_generated_action_name(scheme_name)
                )),
            );
        }

        let mut notes = Vec::new();
        if let Some(title) = spec
            .get("info")
            .and_then(|value| value.get("title"))
            .and_then(|value| value.as_str())
        {
            notes.push(format!("Spec title: {}", title.trim()));
        }
        if let Some(version) = spec
            .get("info")
            .and_then(|value| value.get("version"))
            .and_then(|value| value.as_str())
        {
            notes.push(format!("Spec version: {}", version.trim()));
        }
        if let Some(url) = source_url {
            notes.push(format!("Source URL: {}", url.trim()));
        }
        if !notes.is_empty() {
            derived.insert(
                "source_notes".to_string(),
                serde_json::Value::String(notes.join(" | ")),
            );
        }

        Some(derived)
    }

    fn derive_capability_from_docs_text(
        docs_text: &str,
        docs_url: Option<&str>,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let mut derived = serde_json::Map::new();
        let endpoint_re = Regex::new(
            r#"(?i)\b(GET|POST|PUT|PATCH|DELETE)\s+(https?://[^\s`"'<>]+|/[A-Za-z0-9._~!$&'()*+,;=:@/%?-]+)"#,
        )
        .ok()?;
        if let Some(captures) = endpoint_re.captures(docs_text) {
            let method = captures.get(1).map(|m| m.as_str().to_ascii_lowercase());
            let endpoint = captures.get(2).map(|m| m.as_str().trim().to_string());
            if let Some(method) = method {
                derived.insert("method".to_string(), serde_json::Value::String(method));
            }
            if let Some(endpoint) = endpoint {
                if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
                    if let Ok(url) = reqwest::Url::parse(&endpoint) {
                        if let Some(base) = Self::capability_base_url_from_url(url.as_str()) {
                            derived.insert("base_url".to_string(), serde_json::Value::String(base));
                        }
                        derived.insert(
                            "path".to_string(),
                            serde_json::Value::String(url.path().to_string()),
                        );
                    }
                } else {
                    derived.insert("path".to_string(), serde_json::Value::String(endpoint));
                    if let Some(base) = docs_url.and_then(Self::capability_base_url_from_url) {
                        derived.insert("base_url".to_string(), serde_json::Value::String(base));
                    }
                }
            }
        }

        let mut required_inputs = Vec::new();
        let path_param_re = Regex::new(r#"\{([A-Za-z0-9_]+)\}|:([A-Za-z0-9_]+)"#).ok()?;
        for captures in path_param_re.captures_iter(docs_text) {
            if let Some(name) = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map(|m| m.as_str().trim().to_string())
            {
                if !required_inputs.iter().any(|existing| existing == &name) {
                    required_inputs.push(name);
                }
            }
        }
        if !required_inputs.is_empty() {
            derived.insert(
                "required_inputs".to_string(),
                serde_json::Value::Array(
                    required_inputs
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect::<Vec<_>>(),
                ),
            );
        }

        let lower = docs_text.to_ascii_lowercase();
        if lower.contains("oauth") {
            derived.insert(
                "auth_type".to_string(),
                serde_json::Value::String("oauth2".to_string()),
            );
        } else if lower.contains("bearer token") || lower.contains("authorization: bearer") {
            derived.insert(
                "auth_type".to_string(),
                serde_json::Value::String("bearer".to_string()),
            );
        } else if lower.contains("x-api-key") || lower.contains("api key") {
            derived.insert(
                "auth_type".to_string(),
                serde_json::Value::String("api_key_header".to_string()),
            );
            if lower.contains("x-api-key") {
                derived.insert(
                    "auth_header_name".to_string(),
                    serde_json::Value::String("X-API-Key".to_string()),
                );
            }
        }

        let summary = docs_text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(180).collect::<String>());
        if let Some(summary) = summary {
            derived.insert(
                "source_notes".to_string(),
                serde_json::Value::String(summary),
            );
        }

        if derived.is_empty() {
            None
        } else {
            Some(derived)
        }
    }

    async fn load_capability_source_text(raw_url: &str) -> Option<String> {
        let parsed = reqwest::Url::parse(raw_url.trim()).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let response = reqwest::Client::new().get(parsed).send().await.ok()?;
        let response = response.error_for_status().ok()?;
        response
            .text()
            .await
            .ok()
            .map(|text| text.trim().to_string())
    }

    async fn enrich_capability_acquisition_arguments(
        arguments: &serde_json::Value,
    ) -> serde_json::Value {
        let mut enriched = arguments.clone();
        if !enriched.is_object() {
            enriched = serde_json::json!({});
        }
        let Some(root) = enriched.as_object_mut() else {
            return enriched;
        };

        let openapi_url = Self::capability_string_argument(arguments, "openapi_url");
        let docs_url = Self::capability_string_argument(arguments, "docs_url");
        let openapi_text =
            if let Some(text) = Self::capability_string_argument(arguments, "openapi_text") {
                Some(text)
            } else if let Some(url) = openapi_url.as_deref() {
                Self::load_capability_source_text(url).await
            } else {
                None
            };
        let docs_text = if let Some(text) = Self::capability_string_argument(arguments, "docs_text")
        {
            Some(text)
        } else if let Some(url) = docs_url.as_deref() {
            Self::load_capability_source_text(url).await
        } else {
            None
        };

        let mut derived_from_openapi = false;
        if let Some(ref spec_text) = openapi_text {
            if let Some(derived) =
                Self::derive_capability_from_openapi_json(spec_text, openapi_url.as_deref())
            {
                derived_from_openapi = true;
                Self::merge_capability_string_field(
                    root,
                    "base_url",
                    derived
                        .get("base_url")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "path",
                    derived
                        .get("path")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "method",
                    derived
                        .get("method")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "auth_type",
                    derived
                        .get("auth_type")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "auth_secret_name",
                    derived
                        .get("auth_secret_name")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "auth_header_name",
                    derived
                        .get("auth_header_name")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "response_notes",
                    derived
                        .get("response_notes")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_string_field(
                    root,
                    "source_notes",
                    derived
                        .get("source_notes")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                );
                Self::merge_capability_required_inputs(
                    root,
                    derived
                        .get("required_inputs")
                        .and_then(|value| value.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str())
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                );
                if root
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .map(|kind| kind.trim().is_empty())
                    .unwrap_or(true)
                {
                    root.insert(
                        "kind".to_string(),
                        serde_json::Value::String("openapi".to_string()),
                    );
                }
            }
        }
        if !derived_from_openapi {
            if let Some(ref docs_text) = docs_text {
                if let Some(derived) =
                    Self::derive_capability_from_docs_text(docs_text, docs_url.as_deref())
                {
                    Self::merge_capability_string_field(
                        root,
                        "base_url",
                        derived
                            .get("base_url")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string),
                    );
                    Self::merge_capability_string_field(
                        root,
                        "path",
                        derived
                            .get("path")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string),
                    );
                    Self::merge_capability_string_field(
                        root,
                        "method",
                        derived
                            .get("method")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string),
                    );
                    Self::merge_capability_string_field(
                        root,
                        "auth_type",
                        derived
                            .get("auth_type")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string),
                    );
                    Self::merge_capability_string_field(
                        root,
                        "auth_header_name",
                        derived
                            .get("auth_header_name")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string),
                    );
                    Self::merge_capability_string_field(
                        root,
                        "source_notes",
                        derived
                            .get("source_notes")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string),
                    );
                    Self::merge_capability_required_inputs(
                        root,
                        derived
                            .get("required_inputs")
                            .and_then(|value| value.as_array())
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| item.as_str())
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                    );
                }
            }
        }

        if root.get("docs_url").is_none() {
            if let Some(url) = docs_url {
                root.insert("docs_url".to_string(), serde_json::Value::String(url));
            }
        }
        if root.get("openapi_url").is_none() {
            if let Some(url) = openapi_url {
                root.insert("openapi_url".to_string(), serde_json::Value::String(url));
            }
        }

        enriched
    }

    fn normalize_generated_action_name(raw: &str) -> String {
        let mut out = String::new();
        let mut prev_dash = false;
        for ch in raw.chars() {
            let mapped = if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            };
            if mapped == '-' {
                if !prev_dash && !out.is_empty() {
                    out.push('-');
                }
                prev_dash = true;
            } else {
                out.push(mapped);
                prev_dash = false;
            }
        }
        out.trim_matches('-').to_string()
    }

    fn render_capability_action_markdown(
        &self,
        arguments: &serde_json::Value,
        name: &str,
        description: &str,
    ) -> String {
        let kind = arguments
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("rest_api");
        let method = arguments
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("get")
            .to_ascii_uppercase();
        let base_url = arguments
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let path = arguments
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let auth_type = arguments
            .get("auth_type")
            .and_then(|value| value.as_str())
            .unwrap_or("none");
        let auth_secret_name = arguments
            .get("auth_secret_name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let auth_header_name = arguments
            .get("auth_header_name")
            .and_then(|value| value.as_str())
            .unwrap_or(if auth_type == "api_key_header" {
                "X-API-Key"
            } else {
                "Authorization"
            });
        let response_notes = arguments
            .get("response_notes")
            .and_then(|value| value.as_str())
            .unwrap_or(
                "Return a concise user-facing summary plus any stable identifiers and links.",
            );
        let source_notes = arguments
            .get("source_notes")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let required_inputs = Self::capability_acquire_required_inputs(arguments);
        let required_block = if required_inputs.is_empty() {
            " []".to_string()
        } else {
            format!(
                "\n{}",
                required_inputs
                    .iter()
                    .map(|item| format!("  - {}", item))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let connector_template = serde_json::json!({
            "url": if path.is_empty() { base_url.to_string() } else { format!("{}{}", base_url, path) },
            "method": method.to_ascii_lowercase(),
            "headers": arguments
                .get("default_headers")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            "query": arguments
                .get("default_query")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            "body": arguments
                .get("body_template")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "pagination": arguments
                .get("pagination")
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        });
        let connector_template =
            serde_json::to_string_pretty(&connector_template).unwrap_or_else(|_| "{}".to_string());

        let auth_notes = match auth_type {
            "bearer" => format!(
                "- Read the bearer token from secret storage key `{}`.\n- Send it in the `{}` header as `Bearer <token>`.\n- If the token is unavailable, report that auth still needs to be configured.",
                if auth_secret_name.is_empty() {
                    format!("{}_token", name)
                } else {
                    auth_secret_name.to_string()
                },
                auth_header_name
            ),
            "api_key_header" => format!(
                "- Read the API key from secret storage key `{}`.\n- Send it in the `{}` header.\n- If the key is unavailable, report that auth still needs to be configured.",
                if auth_secret_name.is_empty() {
                    format!("{}_api_key", name)
                } else {
                    auth_secret_name.to_string()
                },
                auth_header_name
            ),
            "api_key_query" => format!(
                "- Read the API key from secret storage key `{}`.\n- Add it to the provider query params before calling `connector_request`.\n- If the key is unavailable, report that auth still needs to be configured.",
                if auth_secret_name.is_empty() {
                    format!("{}_api_key", name)
                } else {
                    auth_secret_name.to_string()
                }
            ),
            "oauth2" => format!(
                "- Prefer an existing connected integration if one already covers this provider.\n- Otherwise use secret/config key `{}` for OAuth credentials or refresh tokens.\n- If OAuth is not connected yet, say that the capability is scaffolded but still needs OAuth setup.",
                if auth_secret_name.is_empty() {
                    format!("{}_oauth", name)
                } else {
                    auth_secret_name.to_string()
                }
            ),
            "basic" => format!(
                "- Read credentials from secret storage key `{}`.\n- Use HTTP Basic auth when calling `connector_request`.\n- If credentials are unavailable, report that auth still needs to be configured.",
                if auth_secret_name.is_empty() {
                    format!("{}_basic_auth", name)
                } else {
                    auth_secret_name.to_string()
                }
            ),
            _ => "- No provider auth is required for this capability.".to_string(),
        };

        let acquisition_mode = match kind {
            "web_automation" => {
                "If the provider has no stable API, use `browser_auto` as the fallback execution path after trying the connector flow."
            }
            "oauth_api" => {
                "Prefer the direct API path, but explicitly report missing OAuth setup when credentials are not connected yet."
            }
            "openapi" => {
                "Preserve the documented API structure from the supplied spec/notes and keep request/response handling predictable."
            }
            _ => "Use the documented HTTP surface directly with `connector_request`.",
        };

        let required_inputs_section = if required_inputs.is_empty() {
            "- No additional required inputs beyond optional `query`.".to_string()
        } else {
            required_inputs
                .iter()
                .map(|item| format!("- `{}`", item))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"---
name: {name}
description: {description}
version: "1.0.0"
permissions: [network]
required:{required_block}
---

# {name}

## Purpose
{description}

## Required Inputs
{required_inputs_section}

## Capability Acquisition Context
- Kind: `{kind}`
- Base URL: `{base_url}`
- Path: `{path}`
- Method: `{method}`
- {acquisition_mode}

## Authentication
{auth_notes}

## Execution
1. Gather any missing required inputs before making a request.
2. Prefer a direct `connector_request` call using this template:

```json
{connector_template}
```

3. Merge user-provided inputs into the request path, query, and body instead of ignoring them.
4. Preserve pagination, retries, and auth requirements when the provider needs them.
5. If the capability cannot run yet because auth/config is missing, say exactly what must be connected or stored next.

## Response Contract
- {response_notes}
- Include stable IDs, URLs, and next steps when available.
- Never reveal raw secrets or credential values.

## Source Notes
{source_notes}
"#,
            name = name,
            description = description,
            required_block = required_block,
            required_inputs_section = required_inputs_section,
            kind = kind,
            base_url = if base_url.is_empty() { "-" } else { base_url },
            path = if path.is_empty() { "-" } else { path },
            method = method,
            acquisition_mode = acquisition_mode,
            auth_notes = auth_notes,
            connector_template = connector_template,
            response_notes = response_notes,
            source_notes = if source_notes.trim().is_empty() {
                "- No additional provider notes supplied.".to_string()
            } else {
                source_notes.to_string()
            },
        )
    }

    /// Preview security verdict for an action without persisting or registering it.
    pub async fn preview_action_security(
        &self,
        name: &str,
        content: &str,
    ) -> Result<Option<crate::security::action_guard::ActionSecurityVerdict>> {
        let preview_dir = self.actions_dir.join(format!(
            ".preview-{}-{}",
            name,
            uuid::Uuid::new_v4().simple()
        ));
        tokio::fs::create_dir_all(&preview_dir).await?;
        let action_file = Self::preferred_skill_markdown_path(&preview_dir);
        tokio::fs::write(&action_file, content).await?;

        let parsed = self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse action preview: {}", e));

        let verdict = match parsed {
            Ok((_info, workflow_content, frontmatter)) => {
                if let Some(ref guard) = self.action_guard {
                    match guard
                        .evaluate_action(&preview_dir, name, &workflow_content, &frontmatter)
                        .await
                    {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!(
                                "Security check failed for action preview '{}': {}",
                                name,
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&preview_dir).await;
                return Err(e);
            }
        };

        let _ = tokio::fs::remove_dir_all(&preview_dir).await;
        Ok(verdict)
    }

    /// Delete/disable an action.
    /// - Custom actions: deleted from disk and runtime.
    /// - Bundled actions: persisted as disabled and removed from runtime.
    /// - System actions: cannot be deleted/disabled.
    pub async fn delete_action(&self, name: &str) -> Result<bool> {
        let (source, file_path) = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => (action.info.source.clone(), action.info.file_path.clone()),
                None => return Ok(false),
            }
        };

        match source {
            ActionSource::System => Ok(false),
            ActionSource::Bundled => {
                {
                    let mut disabled = self.disabled_actions.write().await;
                    disabled.insert(name.to_string());
                }
                self.save_disabled_actions().await?;
                tracing::info!("Disabled bundled action '{}'", name);
                Ok(true)
            }
            ActionSource::Custom => {
                if let Some(fp) = file_path {
                    let action_path = std::path::Path::new(&fp);
                    if let Some(action_dir) = action_path.parent() {
                        let dir_path = action_dir.to_path_buf();
                        if dir_path.exists() {
                            tokio::fs::remove_dir_all(&dir_path).await?;
                        }
                    }
                }
                let mut actions = self.actions.write().await;
                actions.remove(name);
                tracing::info!("Deleted custom action '{}'", name);
                Ok(true)
            }
        }
    }

    /// Check if an action is a workflow action (LLM-driven) and get its workflow content
    /// Returns None if action doesn't exist or has no workflow content
    pub async fn get_workflow_content(&self, action_name: &str) -> Option<String> {
        self.actions
            .read()
            .await
            .get(action_name)
            .and_then(|s| s.workflow_content.clone())
    }

    pub async fn is_cli_action(&self, action_name: &str) -> bool {
        self.actions
            .read()
            .await
            .get(action_name)
            .map(|action| action.cli_binding.is_some())
            .unwrap_or(false)
    }

    /// Execute a workflow action with LLM orchestration
    /// This performs web searches based on the workflow, then passes everything to the LLM
    pub async fn execute_workflow_action(
        &self,
        action_name: &str,
        workflow_content: &str,
        user_query: &str,
        llm: &crate::core::LlmClient,
    ) -> Result<String> {
        tracing::info!("Executing LLM-driven workflow action: {}", action_name);

        // Step 1: Extract search queries from the workflow
        let search_queries = self.extract_search_queries(workflow_content, action_name, user_query);

        // Step 2: Perform web searches
        let mut search_results = Vec::new();
        let search_config = build_search_config(&self.config_dir).await;

        for query in &search_queries {
            tracing::debug!("Searching: {}", query);
            let args = crate::actions::search::SearchArgs {
                query: query.clone(),
                num_results: 5,
                backend: None,
            };
            match crate::actions::search::execute_search(&args, &search_config).await {
                Ok(results) => {
                    search_results.push(format!("### Search: {}\n{}", query, results));
                }
                Err(e) => {
                    tracing::warn!("Search failed for '{}': {}", query, e);
                    search_results.push(format!("### Search: {} (failed: {})", query, e));
                }
            }
        }

        // Step 3: Build the LLM prompt with workflow instructions and search results
        let combined_results = search_results.join("\n\n");

        let system_prompt = format!(
            r#"You are executing an action workflow. Your task is to analyze the search results and produce output that EXACTLY follows the output format specified in the workflow.

## ACTION WORKFLOW INSTRUCTIONS
{}

## IMPORTANT RULES
1. Follow the "Output Format" section EXACTLY - use the same structure, headings, and formatting
2. Fill in all placeholder sections with actual content based on the search results
3. The LinkedIn post must be 800-1200 characters as specified
4. Include real data, trends, and insights from the search results
5. If search results are insufficient, note this but still produce the best output possible
6. Use today's date where [Date] is specified: {}

## SEARCH RESULTS TO ANALYZE
{}
"#,
            workflow_content,
            chrono::Utc::now().format("%Y-%m-%d"),
            combined_results
        );

        let user_prompt = format!(
            "Execute the workflow above. User's additional context/query: '{}'. Generate the complete output following the exact format specified in the workflow.",
            if user_query.is_empty() { "none" } else { user_query }
        );

        // Step 4: Call LLM to generate the formatted output
        let response = llm
            .chat(
                &system_prompt,
                &user_prompt,
                &[], // No memory entries needed
                &[], // No additional tools
            )
            .await?;

        Ok(response.content)
    }

    fn build_workflow_user_query(arguments: &serde_json::Value) -> String {
        arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Backward compatibility: if no explicit "query" exists, pass structured
                // arguments through as JSON so workflow actions can still consume fields.
                if let Some(obj) = arguments.as_object() {
                    if !obj.is_empty() {
                        return serde_json::to_string(arguments).unwrap_or_default();
                    }
                }
                String::new()
            })
    }

    fn collect_required_fields_from_schema(schema: &serde_json::Value) -> Vec<String> {
        schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn has_non_empty_argument(arguments: &serde_json::Value, key: &str) -> bool {
        let Some(value) = arguments.get(key) else {
            return false;
        };
        match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(s) => !s.trim().is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::Object(map) => !map.is_empty(),
            _ => true,
        }
    }

    fn collect_provided_argument_keys(arguments: &serde_json::Value) -> Vec<String> {
        let Some(obj) = arguments.as_object() else {
            return Vec::new();
        };
        obj.iter()
            .filter(|(_, v)| match v {
                serde_json::Value::Null => false,
                serde_json::Value::String(s) => !s.trim().is_empty(),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(map) => !map.is_empty(),
                _ => true,
            })
            .map(|(k, _)| k.to_string())
            .collect()
    }

    fn build_workflow_missing_inputs_marker(payload: &WorkflowMissingInputsPayload) -> String {
        let json = serde_json::to_string(payload).unwrap_or_else(|_| {
            let fallback = serde_json::json!({
                "action": payload.action,
                "missing": payload.missing,
                "required": payload.required,
                "provided": payload.provided,
                "query": payload.query
            });
            fallback.to_string()
        });
        format!("{}{}", WORKFLOW_MISSING_INPUTS_MARKER, json)
    }

    fn dedupe_non_empty<I>(items: I) -> Vec<String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            let cleaned = item
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('`')
                .trim_matches('"')
                .trim_matches('\'')
                .trim_end_matches(',')
                .to_string();
            if cleaned.is_empty() {
                continue;
            }
            let key = cleaned.to_ascii_lowercase();
            if seen.insert(key) {
                out.push(cleaned);
            }
        }
        out
    }

    fn parse_required_fields_from_frontmatter(frontmatter: &str) -> Vec<String> {
        let mut required = Vec::new();
        let lines: Vec<&str> = frontmatter.lines().collect();
        let mut i = 0usize;

        while i < lines.len() {
            let raw = lines[i];
            let line = raw.trim();
            let is_required_key = line.starts_with("required:")
                || line.starts_with("required_inputs:")
                || line.starts_with("requiredInputs:");
            if is_required_key {
                let rhs = line
                    .split_once(':')
                    .map(|(_, rhs)| rhs.trim())
                    .unwrap_or("");
                if rhs.starts_with('[') && rhs.ends_with(']') {
                    let inner = &rhs[1..rhs.len().saturating_sub(1)];
                    for part in inner.split(',') {
                        required.push(part.trim().trim_matches('"').trim_matches('\'').to_string());
                    }
                } else if !rhs.is_empty() {
                    required.push(rhs.trim_matches('"').trim_matches('\'').to_string());
                }

                let mut j = i + 1;
                while j < lines.len() {
                    let next_raw = lines[j];
                    let next_trim = next_raw.trim();
                    if next_trim.starts_with("- ") {
                        required.push(
                            next_trim
                                .trim_start_matches("- ")
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .to_string(),
                        );
                        j += 1;
                        continue;
                    }
                    if next_raw.starts_with(' ')
                        || next_raw.starts_with('\t')
                        || next_trim.is_empty()
                    {
                        j += 1;
                        continue;
                    }
                    break;
                }
                i = j;
                continue;
            }
            i += 1;
        }

        Self::dedupe_non_empty(required)
    }

    fn parse_required_fields_from_workflow(workflow: &str) -> Vec<String> {
        let mut required = Vec::new();
        let mut in_required_section = false;

        for raw_line in workflow.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('#') {
                let heading = line.trim_start_matches('#').trim().to_ascii_lowercase();
                in_required_section = heading.contains("required input")
                    || heading == "required"
                    || heading == "input contract";
                continue;
            }

            if line.to_ascii_lowercase().starts_with("required inputs:") {
                in_required_section = true;
                continue;
            }

            if in_required_section
                && !line.starts_with("- ")
                && !line.starts_with("* ")
                && line.ends_with(':')
            {
                in_required_section = false;
                continue;
            }

            if !in_required_section {
                continue;
            }

            let candidate = if line.starts_with("- ") {
                line.trim_start_matches("- ").trim()
            } else if let Some(rest) = line.strip_prefix("* ") {
                rest.trim()
            } else {
                continue;
            };

            let mut field =
                if let (Some(start), Some(end)) = (candidate.find('`'), candidate.rfind('`')) {
                    if end > start {
                        candidate[start + 1..end].trim().to_string()
                    } else {
                        candidate.to_string()
                    }
                } else {
                    candidate.to_string()
                };

            if let Some((left, _)) = field.split_once(':') {
                field = left.trim().to_string();
            }
            field = field
                .trim_matches('{')
                .trim_matches('}')
                .trim_matches('[')
                .trim_matches(']')
                .trim()
                .to_string();

            if field
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                required.push(field);
            }
        }

        Self::dedupe_non_empty(required)
    }

    fn build_workflow_input_schema(frontmatter: &str, workflow_content: &str) -> serde_json::Value {
        let mut required = Self::parse_required_fields_from_frontmatter(frontmatter);
        if required.is_empty() {
            required = Self::parse_required_fields_from_workflow(workflow_content);
        }
        let required = Self::dedupe_non_empty(required);

        let mut properties = serde_json::Map::new();
        properties.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional free-form input/context for the action"
            }),
        );

        for key in &required {
            if key.eq_ignore_ascii_case("query") {
                continue;
            }
            properties.insert(
                key.clone(),
                serde_json::json!({
                    "type": "string",
                    "description": format!("Required input: {}", key)
                }),
            );
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    /// Extract search queries from workflow content based on action type
    fn extract_search_queries(
        &self,
        workflow: &str,
        action_name: &str,
        user_query: &str,
    ) -> Vec<String> {
        let mut queries = Vec::new();
        let year = chrono::Utc::now().format("%Y");
        let month = chrono::Utc::now().format("%B");

        // Look for search queries in the workflow (lines starting with - "...")
        for line in workflow.lines() {
            let line = line.trim();
            if line.starts_with("- \"") && line.ends_with("\"") {
                let query = line.trim_start_matches("- \"").trim_end_matches("\"");
                // Replace placeholders
                let query = query
                    .replace("2026", &year.to_string())
                    .replace("February", &month.to_string());
                queries.push(query.to_string());
            }
        }

        // If the workflow does not declare explicit queries, fall back to a
        // generic topic-based set rather than hardcoding skill names here.
        if queries.is_empty() {
            let topic = if user_query.trim().is_empty() {
                action_name.replace('-', " ")
            } else {
                user_query.trim().to_string()
            };
            queries.push(format!("{} latest news {}", topic, year));
            queries.push(format!("{} trends analysis {}", topic, year));
            if !user_query.trim().is_empty() {
                queries.push(format!("{} {}", user_query.trim(), year));
            }
        }

        queries
    }

    /// Load markdown-defined actions from a directory
    /// Looks for SKILL.md files in subdirectories, with ACTION.md as a legacy fallback.
    /// These are registered as workflow actions for LLM-driven execution
    pub async fn load_markdown_actions(&self, dir: &Path, source: ActionSource) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        // Read directory entries
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Could not read skills directory {:?}: {}", dir, e);
                return Ok(());
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(md_file) = Self::resolve_skill_markdown_path(&path) else {
                continue;
            };

            match self.parse_action_md(&md_file, source.clone()).await {
                Ok((info, workflow_content, frontmatter)) => {
                    if source == ActionSource::Bundled {
                        let disabled = self.disabled_actions.read().await;
                        if disabled.contains(&info.name) {
                            tracing::info!(
                                "Loaded bundled action '{}' as disabled from {:?}",
                                info.name,
                                md_file
                            );
                        }
                    }

                    // Run security evaluation if action guard is set
                    if let Some(ref guard) = self.action_guard {
                        match guard
                            .evaluate_action(&path, &info.name, &workflow_content, &frontmatter)
                            .await
                        {
                            Ok(verdict) => {
                                if !verdict.allow_load {
                                    tracing::warn!(
                                        "Action '{}' BLOCKED by security guard: {:?}",
                                        info.name,
                                        verdict.warnings
                                    );
                                    continue; // skip registration
                                }
                                for w in &verdict.warnings {
                                    tracing::warn!("Action '{}': {}", info.name, w);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Security check failed for '{}': {} — loading anyway",
                                    info.name,
                                    e
                                );
                            }
                        }
                    }

                    tracing::info!("Loaded workflow action '{}' from {:?}", info.name, md_file);
                    self.register_workflow_action(info, workflow_content).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to load action from {:?}: {}", md_file, e);
                }
            }
        }

        Ok(())
    }

    /// Parse a SKILL.md file to extract action information and full content
    /// Legacy ACTION.md files are also accepted.
    /// Returns (ActionDef, full_workflow_content, frontmatter_text)
    async fn parse_action_md(
        &self,
        path: &Path,
        source: ActionSource,
    ) -> Result<(ActionDef, String, String)> {
        let content = tokio::fs::read_to_string(path).await?;

        // Parse YAML frontmatter (between --- markers)
        let mut name = String::new();
        let mut description = String::new();
        let mut version = "1.0.0".to_string();
        let mut frontmatter_text = String::new();

        if let Some(stripped) = content.strip_prefix("---") {
            if let Some(end_pos) = stripped.find("---") {
                let frontmatter = &stripped[..end_pos];
                frontmatter_text = frontmatter.to_string();
                for line in frontmatter.lines() {
                    let line = line.trim();
                    if let Some(val) = line.strip_prefix("name:") {
                        name = val.trim().trim_matches('"').to_string();
                    } else if let Some(val) = line.strip_prefix("description:") {
                        description = val.trim().trim_matches('"').to_string();
                    } else if let Some(val) = line.strip_prefix("version:") {
                        version = val.trim().trim_matches('"').to_string();
                    }
                }
            }
        }

        // Fallback: use directory name as action name
        if name.is_empty() {
            name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }

        // Extract first heading as description if not in frontmatter
        if description.is_empty() {
            for line in content.lines() {
                if let Some(stripped) = line.strip_prefix("# ") {
                    description = stripped.trim().to_string();
                    break;
                }
            }
        }
        if description.is_empty() {
            description = format!("Custom skill '{}'", name);
        }

        // Parse permissions from frontmatter
        let permissions = crate::security::ActionGuard::parse_permissions(&frontmatter_text);
        let mut capabilities: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
        if capabilities.is_empty() {
            capabilities.push("research".to_string());
        }

        let info = ActionDef {
            name,
            description,
            version,
            input_schema: Self::build_workflow_input_schema(&frontmatter_text, &content),
            capabilities,
            sandbox_mode: Some(SandboxMode::Native),
            source,
            file_path: Some(path.to_string_lossy().to_string()),
        };

        // Return the info, full content, and frontmatter for security evaluation
        Ok((info, content, frontmatter_text))
    }

    /// Execute a WASM module with given arguments
    async fn run_wasm_module(
        &self,
        wasm_bytes: &[u8],
        arguments: &serde_json::Value,
    ) -> Result<String> {
        use wasmtime::*;

        let engine = Engine::default();

        // Create a basic store without WASI for simple modules
        let mut store = Store::new(&engine, ());

        // Compile the module
        let module = Module::new(&engine, wasm_bytes)?;

        // Create a linker and instantiate
        let linker = Linker::new(&engine);
        let instance = linker.instantiate(&mut store, &module)?;

        // Try to find entry points
        let result = if let Ok(run_fn) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            run_fn.call(&mut store, ())?;
            format!(
                "WASM execution completed successfully. Args: {}",
                serde_json::to_string(arguments)?
            )
        } else if let Ok(run_fn) = instance.get_typed_func::<(), i32>(&mut store, "run") {
            let exit_code = run_fn.call(&mut store, ())?;
            format!("WASM execution completed with exit code: {}", exit_code)
        } else if let Ok(run_fn) = instance.get_typed_func::<i32, i32>(&mut store, "main") {
            let exit_code = run_fn.call(&mut store, 0)?;
            format!("WASM execution completed with exit code: {}", exit_code)
        } else {
            // List available exports for debugging
            let exports: Vec<String> = instance
                .exports(&mut store)
                .map(|e| e.name().to_string())
                .collect();
            return Err(anyhow::anyhow!(
                "WASM module has no _start, run, or main entry point. Available exports: {:?}",
                exports
            ));
        };

        Ok(result)
    }
}

/// Build search config: loads user settings from search.toml, injects API-backed
/// secrets, auto-detects Playwright for explicit opt-in use, and applies the
/// default Lightpanda -> DuckDuckGo -> none chain only when no chain is saved.
async fn build_search_config(config_dir: &Path) -> crate::actions::SearchConfig {
    // Load saved search config (from Settings UI)
    let mut config = match std::fs::read_to_string(config_dir.join("search.toml")) {
        Ok(content) => toml::from_str::<crate::actions::SearchConfig>(&content).unwrap_or_default(),
        Err(_) => crate::actions::SearchConfig::default(),
    };

    if let Ok(manager) = crate::core::config::SecureConfigManager::new(config_dir) {
        if let Ok(secrets) = manager.load_secrets() {
            if let Some(api_key) = secrets
                .custom
                .get("search_serper_key")
                .filter(|value| !value.trim().is_empty())
                .cloned()
            {
                config.serper = Some(crate::actions::search::SearchBackend::Serper { api_key });
            } else if matches!(
                &config.serper,
                Some(crate::actions::search::SearchBackend::Serper { api_key })
                    if api_key.trim().is_empty()
            ) {
                config.serper = None;
            }

            if let Some(api_key) = secrets
                .custom
                .get("search_brave_key")
                .filter(|value| !value.trim().is_empty())
                .cloned()
            {
                config.brave = Some(crate::actions::search::SearchBackend::Brave { api_key });
            } else if matches!(
                &config.brave,
                Some(crate::actions::search::SearchBackend::Brave { api_key })
                    if api_key.trim().is_empty()
            ) {
                config.brave = None;
            }
        }
    }

    // Auto-detect Playwright bridge if not already set so explicit browser-backed
    // search remains available on full deployments.
    if config.playwright.is_none() {
        let bridge_url = std::env::var("PLAYWRIGHT_BRIDGE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3100".to_string());

        let available = reqwest::Client::new()
            .get(format!("{}/health", bridge_url))
            .timeout(std::time::Duration::from_secs(1))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        if available {
            tracing::debug!("Playwright bridge available at {}", bridge_url);
            config.playwright =
                Some(crate::actions::search::SearchBackend::Playwright { bridge_url });
        } else {
            tracing::debug!("Playwright bridge unavailable; browser-backed search disabled");
        }
    }

    config.ensure_default_chain();

    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::integration_enabled_key;

    #[tokio::test]
    async fn app_management_schemas_avoid_top_level_combinators() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let actions = runtime.list_actions().await.unwrap();

        for action_name in ["app_deploy", "app_restart", "app_stop", "app_delete"] {
            let action = actions
                .iter()
                .find(|action| action.name == action_name)
                .unwrap_or_else(|| panic!("missing builtin action {}", action_name));
            let schema = &action.input_schema;
            assert_eq!(
                schema.get("type").and_then(|value| value.as_str()),
                Some("object"),
                "{} schema should stay a top-level object",
                action_name
            );
            for combinator in ["anyOf", "oneOf", "allOf", "not"] {
                assert!(
                    schema.get(combinator).is_none(),
                    "{} schema should not use top-level {}",
                    action_name,
                    combinator
                );
            }
        }
    }

    #[tokio::test]
    async fn list_enabled_actions_autoheals_connected_google_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let manager = crate::core::config::SecureConfigManager::new(temp.path()).unwrap();
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
                Some(
                    serde_json::json!({
                        "access_token": "access",
                        "refresh_token": "refresh",
                        "expires_at": chrono::Utc::now().timestamp() + 3600,
                        "granted_scopes": [
                            "https://www.googleapis.com/auth/gmail.readonly",
                            "https://www.googleapis.com/auth/gmail.send",
                            "https://www.googleapis.com/auth/calendar"
                        ],
                        "granted_bundles": ["gmail", "calendar"]
                    })
                    .to_string(),
                ),
            )
            .unwrap();
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
                Some(serde_json::json!(["gmail", "calendar"]).to_string()),
            )
            .unwrap();
        manager
            .set_custom_secret(
                &integration_enabled_key("google_workspace"),
                Some("false".to_string()),
            )
            .unwrap();

        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
        assert!(enabled
            .iter()
            .any(|action| action.name == "google_workspace_gws_command"));
        assert!(enabled
            .iter()
            .any(|action| action.name == "google_workspace_gws_skills"));
        assert_eq!(
            manager
                .get_custom_secret(&integration_enabled_key("google_workspace"))
                .unwrap()
                .as_deref(),
            Some("true")
        );
    }

    #[test]
    fn loopback_http_get_rejects_non_app_paths() {
        let url = reqwest::Url::parse("http://127.0.0.1:8990/api/secret").unwrap();
        let err = ActionRuntime::loopback_http_get_allowed(&url).unwrap_err();
        assert!(err.to_string().contains("/apps/"));
    }

    #[test]
    fn loopback_http_get_allows_local_app_paths() {
        let url = reqwest::Url::parse("http://localhost:8990/apps/demo/health").unwrap();
        assert!(ActionRuntime::loopback_http_get_allowed(&url).is_ok());
    }

    #[test]
    fn native_env_overrides_block_runtime_control_keys() {
        let args = serde_json::json!({
            "env": {
                "PATH": "/tmp/bin"
            }
        });
        let err = ActionRuntime::collect_native_env_overrides(&args).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn workspace_alias_paths_remap_to_current_dir() {
        let runtime = ActionRuntime {
            config: RuntimeConfig::default(),
            _sandbox: ActionSandbox::new(&RuntimeConfig::default()).unwrap(),
            transactions: tokio::sync::Mutex::new(TransactionManager::new(PathBuf::from(
                "snapshots",
            ))),
            actions: tokio::sync::RwLock::new(HashMap::new()),
            disabled_actions: tokio::sync::RwLock::new(HashSet::new()),
            disabled_actions_file: PathBuf::from("./disabled_actions.json"),
            actions_dir: PathBuf::from("./skills"),
            cli_skills_dir: PathBuf::from("./cli_skills"),
            config_dir: PathBuf::from("."),
            task_queue: None,
            action_guard: None,
            storage: None,
            mcp_registry: None,
            plugin_registry: None,
        };
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            runtime
                .absolutize_tool_path("/workspace/demo/index.html")
                .unwrap(),
            cwd.join("demo").join("index.html")
        );
    }

    #[test]
    fn private_ips_are_not_treated_as_public() {
        assert!(!ActionRuntime::ip_is_public(IpAddr::V4(Ipv4Addr::new(
            127, 0, 0, 1
        ))));
        assert!(!ActionRuntime::ip_is_public(IpAddr::V4(Ipv4Addr::new(
            169, 254, 1, 10
        ))));
        assert!(ActionRuntime::ip_is_public(IpAddr::V4(Ipv4Addr::new(
            1, 1, 1, 1
        ))));
    }

    #[tokio::test]
    async fn install_cli_skill_action_persists_and_reloads() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        let skill_markdown = r#"---
name: officecli
description: Office CLI
version: "1.2.3"
---
# officecli
"#;
        let manifest = InstalledCliSkillManifest {
            name: "officecli".to_string(),
            description: "Office CLI".to_string(),
            version: "1.2.3".to_string(),
            executable_path: temp.path().join("officecli").display().to_string(),
            verify_args: vec!["--version".to_string()],
            source_url: Some("https://officecli.ai/SKILL.md".to_string()),
        };

        runtime
            .install_cli_skill_action(manifest.clone(), skill_markdown)
            .await
            .unwrap();

        let actions = runtime.list_actions().await.unwrap();
        assert!(actions.iter().any(|action| action.name == "officecli"));

        let reloaded = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        reloaded.load_all_actions().await.unwrap();
        let reloaded_actions = reloaded.list_actions().await.unwrap();
        assert!(reloaded_actions
            .iter()
            .any(|action| action.name == "officecli"));
    }

    #[tokio::test]
    async fn cli_skill_action_executes_bound_command() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        let manifest = InstalledCliSkillManifest {
            name: "echo-cli".to_string(),
            description: "Echo CLI".to_string(),
            version: "1.0.0".to_string(),
            executable_path: if cfg!(windows) {
                "cmd".to_string()
            } else {
                "sh".to_string()
            },
            verify_args: vec![],
            source_url: None,
        };

        runtime
            .install_cli_skill_action(
                manifest,
                "---\nname: echo-cli\ndescription: Echo CLI\n---\n# echo-cli\n",
            )
            .await
            .unwrap();

        let args = if cfg!(windows) {
            serde_json::json!({ "args": ["/C", "echo", "ready"] })
        } else {
            serde_json::json!({ "args": ["-lc", "printf ready"] })
        };
        let output = runtime.execute_action("echo-cli", &args).await.unwrap();
        assert!(output.contains("ready"));
    }
}
