//! Action Runtime - WASM Sandbox + Docker + Transactional Execution
//! Based on arXiv:2512.12806 "Fault-Tolerant Sandboxing"
//!
//! Features:
//! - WASM sandbox for lightweight, fast action execution
//! - Docker sandbox for heavier/untrusted operations
//! - Transactional filesystem with rollback capability

mod action_runtime;
mod ark_inspect;
mod file_tools;
mod sandbox;
pub mod toolsets;
mod transaction;

pub use sandbox::SandboxMode;
pub use transaction::TransactionManager;

use anyhow::{Context, Result};
#[cfg(feature = "docker")]
use futures::TryStreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::actions::ActionCallerPrincipal;
use crate::actions::{
    ActionAuthorization, ActionAuthorizationContext, ActionAuthorizationDecision, ActionDef,
    ActionErrorDomain, ActionErrorReason, ActionExecutionSurface, ActionRiskLevel, ActionSource,
};
use crate::clients::{CodeExecuteFilePayload, ExecutorClient, ExecutorClientConfig};
use crate::core::runtime::config::{AgentConfig, SecureConfigManager};
use crate::core::runtime::runtime_image;

/// Runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub default_sandbox: SandboxMode,
    pub wasm_memory_limit: u64,
    pub docker_image: String,
    pub enable_rollback: bool,
    pub snapshot_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingSecretPlaceholderKind {
    Secret,
    Env,
}

impl MissingSecretPlaceholderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Env => "env",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingSecretPlaceholder {
    pub action_name: String,
    pub kind: MissingSecretPlaceholderKind,
    pub key: String,
}

impl MissingSecretPlaceholder {
    pub fn new(action_name: &str, kind: MissingSecretPlaceholderKind, key: &str) -> Self {
        Self {
            action_name: action_name.trim().to_string(),
            kind,
            key: key.trim().to_string(),
        }
    }

    pub fn prompt_storage_key(&self) -> String {
        match self.kind {
            MissingSecretPlaceholderKind::Secret => self.key.clone(),
            MissingSecretPlaceholderKind::Env => format!("env:{}", self.key),
        }
    }
}

impl std::fmt::Display for MissingSecretPlaceholder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Missing credential {}:{} for action '{}'",
            self.kind.as_str(),
            self.key,
            self.action_name
        )
    }
}

impl std::error::Error for MissingSecretPlaceholder {}

const TOOL_PAYLOAD_INLINE_BYTES: usize = 256 * 1024;
const TOOL_PAYLOAD_RESOURCE_DIR: &str = "tool-payloads";
const FILE_SEARCH_DEFAULT_MAX_FILES_SCANNED: usize = 2_500;
const FILE_SEARCH_DEFAULT_MAX_ENTRIES_VISITED: usize = 20_000;
const FILE_SEARCH_DEFAULT_SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackend {
    Docker,
    Native,
    RemoteExecutor,
    Wasm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendFallbackPolicy {
    AutoDegrade,
    RequireExact,
    AskUser,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendPreference {
    pub preferred: Vec<RuntimeBackend>,
    pub fallback_policy: BackendFallbackPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeResourceRef {
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub bytes: u64,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ToolPayload {
    Text(String),
    Structured(serde_json::Value),
    Bytes {
        mime: Option<String>,
        body: Vec<u8>,
        suggested_name: Option<String>,
    },
    Resource {
        resource: RuntimeResourceRef,
        metadata: Option<serde_json::Value>,
    },
    Empty,
}

#[derive(Debug, Clone, Default)]
pub struct PersistHints {
    pub mime: Option<String>,
    pub suggested_name: Option<String>,
    pub source_action: Option<String>,
    pub force_resource: bool,
}

pub trait DurableStore {
    fn put_payload<'a>(
        &'a self,
        payload: ToolPayload,
        hints: PersistHints,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolPayload>> + Send + 'a>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPathAccessError {
    OutsideAllowedRoots {
        attempted_path: PathBuf,
        allowed_roots: Vec<PathBuf>,
    },
}

impl std::fmt::Display for ToolPathAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideAllowedRoots {
                attempted_path,
                allowed_roots,
            } => {
                let roots = allowed_roots
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "Path '{}' is outside allowed roots: {}",
                    attempted_path.display(),
                    roots
                )
            }
        }
    }
}

impl std::error::Error for ToolPathAccessError {}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_sandbox: SandboxMode::Wasm,
            wasm_memory_limit: 256 * 1024 * 1024, // 256MB
            docker_image: runtime_image::default_runtime_image(),
            enable_rollback: true,
            snapshot_dir: PathBuf::from("snapshots"),
        }
    }
}

fn authorization_with_access(
    access: crate::actions::ActionAccessMetadata,
) -> crate::actions::ActionAuthorization {
    crate::actions::ActionAuthorization {
        access,
        ..crate::actions::ActionAuthorization::default()
    }
}

fn integration_authorization(integration_id: &str) -> crate::actions::ActionAuthorization {
    authorization_with_access(crate::actions::ActionAccessMetadata {
        integration_ids: vec![integration_id.to_string()],
        ..crate::actions::ActionAccessMetadata::default()
    })
}

fn read_only_authorization(
    mut authorization: crate::actions::ActionAuthorization,
) -> crate::actions::ActionAuthorization {
    authorization.outbound.read_only = true;
    authorization.outbound.outbound_write = false;
    authorization.outbound.public_publish = false;
    authorization
}

fn integration_authorization_with_features(
    integration_id: &str,
    features: &[&str],
) -> crate::actions::ActionAuthorization {
    let mut integration_features = BTreeMap::new();
    integration_features.insert(
        integration_id.to_string(),
        features.iter().map(|feature| feature.to_string()).collect(),
    );
    authorization_with_access(crate::actions::ActionAccessMetadata {
        integration_ids: vec![integration_id.to_string()],
        integration_features,
        ..crate::actions::ActionAccessMetadata::default()
    })
}

fn google_workspace_authorization() -> crate::actions::ActionAuthorization {
    integration_authorization("google_workspace")
}

fn google_workspace_bundle_authorization(bundle: &str) -> crate::actions::ActionAuthorization {
    integration_authorization_with_features("google_workspace", &[bundle])
}

fn google_workspace_bundle_read_authorization(bundle: &str) -> crate::actions::ActionAuthorization {
    read_only_authorization(google_workspace_bundle_authorization(bundle))
}

fn channel_target(argument_key: &str, default_target: &str) -> crate::actions::ActionChannelTarget {
    crate::actions::ActionChannelTarget {
        argument_key: argument_key.to_string(),
        default_target: default_target.to_string(),
    }
}

/// The action runtime that manages execution
pub struct ActionRuntime {
    config: RuntimeConfig,
    /// Transactions wrapped in Mutex for concurrent access
    transactions: tokio::sync::Mutex<TransactionManager>,
    /// Actions wrapped in RwLock for concurrent access
    actions: tokio::sync::RwLock<HashMap<String, LoadedAction>>,
    /// Bundled actions explicitly disabled by user (persisted on disk)
    disabled_actions: tokio::sync::RwLock<HashSet<String>>,
    disabled_actions_file: PathBuf,
    /// Persisted security/readiness state for all non-builtin actions.
    action_reviews: tokio::sync::RwLock<HashMap<String, ActionReviewRecord>>,
    action_reviews_file: PathBuf,
    /// In-memory capability observations scoped to active conversations/runs.
    capability_run_contexts: tokio::sync::RwLock<HashMap<String, CapabilityRunCorrelationRecord>>,
    /// Bundled actions deleted by user for this install (persisted on disk)
    removed_bundled_actions: tokio::sync::RwLock<HashSet<String>>,
    removed_bundled_actions_file: PathBuf,
    actions_dir: PathBuf,
    cli_skills_dir: PathBuf,
    config_dir: PathBuf,
    /// Shared task queue for list_tasks action
    task_queue: Option<std::sync::Arc<tokio::sync::RwLock<crate::core::TaskQueue>>>,
    /// Action security guard for integrity, static analysis, permissions, injection detection
    action_guard: Option<std::sync::Arc<crate::security::ActionGuard>>,
    /// Shared safety engine for dynamically registered integration actions.
    safety_engine: Option<std::sync::Arc<crate::safety::SafetyEngine>>,
    /// Actions explicitly auto-approved by the user in Settings > Advanced.
    auto_approved_actions: std::sync::RwLock<HashSet<String>>,
    /// Structural guard for outward URLs used by runtime-backed actions.
    tool_args_guard_config:
        std::sync::RwLock<crate::security::tool_args_guard::ToolArgsGuardConfig>,
    /// Shared storage for expense + entity operations
    storage: Option<crate::storage::Storage>,
    /// Embedding client for runtime actions that do bounded semantic retrieval.
    embedding_client: Option<std::sync::Arc<crate::core::EmbeddingClient>>,
    /// Stable identifier for the active user (DID), set by `Agent::init`. Used
    /// by per-user features such as ArkOrbit when no explicit user scope is
    /// supplied in tool arguments.
    current_user_id: Option<String>,
    /// MCP registry for external tools/resources
    mcp_registry: Option<std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>>,
    /// Plugin registry for third-party HTTP extensions
    plugin_registry:
        Option<std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>>,
    /// Generic extension-pack registry for integrations, channels, and user-installed packs
    extension_pack_registry:
        Option<std::sync::Arc<tokio::sync::RwLock<crate::extension_packs::ExtensionPackRegistry>>>,
    #[cfg(feature = "docker")]
    active_sandbox_containers: tokio::sync::RwLock<HashSet<String>>,
    #[cfg(feature = "docker")]
    container_reaper_status: tokio::sync::RwLock<ContainerReaperStatus>,
}

const LOCAL_APP_HTTP_PORT: u16 = 8990;
const HTTP_GET_TIMEOUT_SECS: u64 = 10;
const HTTP_GET_MAX_BODY_BYTES: usize = 1_000_000;
const VISION_IMAGE_INLINE_MAX_BODY_BYTES: usize = 20 * 1024 * 1024;
const VISION_DOCUMENT_INLINE_MAX_BODY_BYTES: usize = 50 * 1024 * 1024;
const MAX_NATIVE_ENV_OVERRIDES: usize = 32;
const ACTION_REVIEW_HISTORY_LIMIT: usize = 10;
const CAPABILITY_CONTEXT_TTL_SECS: i64 = 60 * 60;
const CAPABILITY_CONTEXT_LIMIT: usize = 128;
const CAPABILITY_CONTEXT_OBSERVATION_LIMIT: usize = 96;
#[cfg(feature = "docker")]
const AGENTARK_SANDBOX_LABEL_KEY: &str = "agentark.runtime";
#[cfg(feature = "docker")]
const AGENTARK_SANDBOX_LABEL_VALUE: &str = "sandbox";

const BACKGROUND_BLOCKED_ACTIONS: &[&str] = &[
    "service_manage",
    "work_manage",
    "app_delete",
    "app_stop",
    "app_restart",
    "app_deploy",
    "shell",
    "code_execute",
    "browser_auto",
    "browser_navigate",
    "browser_click",
    "browser_type",
    "browser_scroll",
    "browser_snapshot",
    "browser_screenshot",
    "browser_back",
    "browser_press",
    "browser_console",
    "gmail_reply",
    "calendar_create",
    "schedule_task",
    "watch",
];

#[derive(Debug, Clone)]
struct OpenAiChatVisionCandidate {
    api_key: String,
    model: String,
    base_url: Option<String>,
}

impl OpenAiChatVisionCandidate {
    fn provider_label(&self) -> &'static str {
        crate::core::model::llm_provider::openai_provider_label(self.base_url.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ContainerReaperStatus {
    pub last_run_at: Option<String>,
    pub last_removed_count: u64,
    pub total_removed_count: u64,
    pub last_error: Option<String>,
}

struct SandboxUploadFile {
    filename: String,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

struct FileWritePayload {
    bytes: Vec<u8>,
    mime: Option<String>,
    source_resource: Option<RuntimeResourceRef>,
}

#[derive(Debug, Clone, Serialize)]
struct IndexedDocumentArtifact {
    id: String,
    filename: String,
    content_type: String,
    chunk_count: usize,
    file_size: u64,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_url: Option<String>,
    duplicate_skipped: bool,
    content_fingerprint: String,
    metadata_only: bool,
    index_mode: String,
}

fn path_has_source_checkout_markers(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("src").is_dir()
}

fn data_dir_looks_like_source_checkout(data_dir: &Path) -> bool {
    if path_has_source_checkout_markers(data_dir) {
        return true;
    }

    let Ok(current_dir) = std::env::current_dir() else {
        return false;
    };
    if !path_has_source_checkout_markers(&current_dir) {
        return false;
    }

    let canonical_data = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let canonical_current =
        std::fs::canonicalize(&current_dir).unwrap_or_else(|_| current_dir.clone());
    canonical_data == canonical_current
}

fn managed_uploads_dir(data_dir: &Path) -> PathBuf {
    if !data_dir_looks_like_source_checkout(data_dir) {
        return data_dir.join("uploads");
    }

    if let Some(dirs) = crate::branding::project_dirs() {
        let fallback_data_dir = dirs.data_dir().to_path_buf();
        if !data_dir_looks_like_source_checkout(&fallback_data_dir) {
            return fallback_data_dir.join("uploads");
        }
    }

    std::env::temp_dir().join("agentark").join("uploads")
}

fn required_skill_name(arguments: &serde_json::Value) -> Result<String> {
    let raw = arguments
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Skill name is required"))?;
    let normalized = ActionRuntime::normalize_generated_action_name(raw);
    if normalized.is_empty() {
        anyhow::bail!("Skill name must contain at least one ASCII letter or number");
    }
    Ok(normalized)
}

async fn read_usage_json(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let Ok(bytes) = tokio::fs::read(path).await else {
        return serde_json::Map::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return serde_json::Map::new();
    };
    value
        .get("skills")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default()
}

async fn write_usage_json(
    path: &Path,
    skills: serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let body = serde_json::json!({
        "skills": skills,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });
    tokio::fs::write(path, serde_json::to_vec_pretty(&body)?).await?;
    Ok(())
}

/// A loaded action ready for execution
struct LoadedAction {
    info: ActionDef,
    builtin_handler: Option<BuiltinActionHandler>,
    supports_background: bool,
    wasm_module: Option<Vec<u8>>,
    /// Workflow content from SKILL.md
    workflow_content: Option<String>,
    /// Optional fixed local CLI binding backed by a verified host executable
    cli_binding: Option<CliToolBinding>,
    /// Optional MCP binding (external tool/resource)
    mcp_binding: Option<McpBinding>,
    /// Optional plugin binding (third-party HTTP extension)
    plugin_binding: Option<PluginBinding>,
    /// Optional imported custom API binding
    custom_api_binding: Option<CustomApiBinding>,
    /// Optional installed extension-pack feature binding
    extension_pack_binding: Option<ExtensionPackActionBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinActionHandler {
    Native,
    Wasm,
    Docker,
}

impl BuiltinActionHandler {
    fn for_action(info: &ActionDef, default_sandbox: SandboxMode) -> Option<Self> {
        if !matches!(info.source, ActionSource::System) {
            return None;
        }
        match info.sandbox_mode.clone().unwrap_or(default_sandbox) {
            SandboxMode::Native => Some(Self::Native),
            SandboxMode::Wasm => Some(Self::Wasm),
            SandboxMode::Docker => Some(Self::Docker),
        }
    }

    async fn execute(
        self,
        runtime: &ActionRuntime,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        match self {
            Self::Native => runtime.execute_native(action_name, arguments).await,
            Self::Wasm => {
                runtime
                    .execute_wasm(action_name, arguments, auth_context)
                    .await
            }
            Self::Docker => {
                runtime
                    .execute_docker(action_name, arguments, auth_context)
                    .await
            }
        }
    }
}

#[derive(Debug, Clone)]
struct CapabilityRunCorrelationRecord {
    updated_at: chrono::DateTime<chrono::Utc>,
    context: crate::security::capabilities::RunCapabilityContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionScopeHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_api_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integration_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extension_pack_ids: Vec<String>,
    #[serde(default)]
    pub requires_ssh_connection: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_targets: Vec<crate::actions::ActionChannelTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpBinding {
    pub server_id: String,
    pub server_name: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub auth_configured: bool,
    pub kind: McpBindingKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum McpBindingKind {
    Tool { name: String },
    Resource { uri: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginBinding {
    pub plugin_id: String,
    pub action_name: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub auth_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolBinding {
    pub executable_path: String,
    #[serde(default)]
    pub verify_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub auth_env_exports: BTreeMap<String, String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_body: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionPackActionBinding {
    pub pack_id: String,
    pub feature_id: String,
    pub action_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub binding_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionReviewStatus {
    Ready,
    NeedsSecrets,
    Blocked,
    Warning,
    Unreviewed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReviewSnapshot {
    pub action_name: String,
    pub source_kind: String,
    pub reviewed_at: String,
    pub fingerprint: String,
    pub status: ActionReviewStatus,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub allow_load: bool,
    #[serde(default)]
    pub allow_execute: bool,
    #[serde(default)]
    pub visible_in_catalog: bool,
    #[serde(default)]
    pub integrity_ok: bool,
    #[serde(default)]
    pub threat_level: String,
    #[serde(default)]
    pub total_severity: u32,
    #[serde(default)]
    pub total_findings: usize,
    #[serde(default)]
    pub risk_score_10: f32,
    #[serde(default)]
    pub risk_band: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub findings: Vec<crate::security::action_guard::AnalysisFinding>,
    #[serde(default)]
    pub required_env: Vec<String>,
    #[serde(default)]
    pub missing_env: Vec<String>,
    #[serde(default)]
    pub permissions_needed: Vec<String>,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub auth_configured: bool,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

impl Default for ActionReviewSnapshot {
    fn default() -> Self {
        Self {
            action_name: String::new(),
            source_kind: "unknown".to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint: String::new(),
            status: ActionReviewStatus::Unreviewed,
            ready: true,
            allow_load: true,
            allow_execute: true,
            visible_in_catalog: true,
            integrity_ok: true,
            threat_level: "Clean".to_string(),
            total_severity: 0,
            total_findings: 0,
            risk_score_10: 0.0,
            risk_band: "secure".to_string(),
            warnings: Vec::new(),
            findings: Vec::new(),
            required_env: Vec::new(),
            missing_env: Vec::new(),
            permissions_needed: Vec::new(),
            requires_auth: false,
            auth_configured: true,
            notes: Vec::new(),
            blocked_reason: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ActionReviewRecord {
    current: ActionReviewSnapshot,
    #[serde(default)]
    history: Vec<ActionReviewSnapshot>,
}

pub const WORKFLOW_ACTION_MARKER: &str = "__WORKFLOW_ACTION__:";
pub const WORKFLOW_MISSING_INPUTS_MARKER: &str = "__WORKFLOW_MISSING_INPUTS__:";
pub const TOOL_COMPLETION_MARKER: &str = "__TOOL_COMPLETION__:";

fn runtime_collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn runtime_truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let end = value
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len());
    format!("{}...", &value[..end])
}

fn runtime_response_body_is_probably_binary(_content_type: &str, body: &[u8]) -> bool {
    if body.is_empty() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return true;
    };
    let mut sampled_chars = 0usize;
    let mut control_chars = 0usize;
    for ch in text.chars().take(4096) {
        sampled_chars = sampled_chars.saturating_add(1);
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            control_chars = control_chars.saturating_add(1);
        }
    }
    sampled_chars > 0 && control_chars > sampled_chars / 20
}

fn runtime_content_type_mime(value: &str) -> Option<String> {
    let mime = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime.contains('/') {
        Some(mime)
    } else {
        None
    }
}

fn runtime_mime_is_textual(value: &str) -> bool {
    let Some(mime) = runtime_content_type_mime(value) else {
        return false;
    };
    let mut parts = mime.splitn(2, '/');
    let top_level = parts.next().unwrap_or("");
    let subtype = parts.next().unwrap_or("");
    top_level == "text"
        || subtype.ends_with("+json")
        || subtype.ends_with("+xml")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/ecmascript"
                | "application/x-javascript"
                | "application/x-www-form-urlencoded"
        )
}

fn runtime_mime_is_html(value: &str) -> bool {
    runtime_content_type_mime(value)
        .as_deref()
        .is_some_and(|mime| matches!(mime, "text/html" | "application/xhtml+xml"))
}

fn runtime_url_expected_mime(url: &reqwest::Url) -> Option<&'static str> {
    mime_guess::from_path(url.path()).first_raw()
}

fn runtime_url_expects_non_text_resource(url: &reqwest::Url) -> bool {
    runtime_url_expected_mime(url).is_some_and(|mime| !runtime_mime_is_textual(mime))
}

fn runtime_url_suggested_filename(url: &reqwest::Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn runtime_response_matches_expected_url_mime(
    expected_mime: Option<&str>,
    content_type: &str,
    body: &[u8],
) -> bool {
    let Some(expected_mime) = expected_mime else {
        return true;
    };
    let Some(expected) = runtime_content_type_mime(expected_mime) else {
        return true;
    };
    let actual = runtime_content_type_mime(content_type);
    let expected_textual = runtime_mime_is_textual(&expected);
    let body_is_binary = runtime_response_body_is_probably_binary(content_type, body);

    match actual.as_deref() {
        Some(actual) if actual == expected => true,
        Some("application/octet-stream") if !expected_textual && body_is_binary => true,
        Some(actual) if expected_textual && runtime_mime_is_textual(actual) => {
            !runtime_mime_is_html(actual) || runtime_mime_is_html(&expected)
        }
        Some(actual) if !expected_textual && runtime_mime_is_textual(actual) => false,
        Some(_) => false,
        None if expected_textual => !body_is_binary,
        None => body_is_binary,
    }
}

fn runtime_expected_mime_mismatch_message(
    action: &str,
    expected_mime: Option<&str>,
    content_type: &str,
) -> String {
    let expected = expected_mime.unwrap_or("the URL's declared resource type");
    let actual =
        runtime_content_type_mime(content_type).unwrap_or_else(|| "no Content-Type".to_string());
    format!(
        "{} expected a {} response from this URL, but the server returned {} instead.",
        action, expected, actual
    )
}

fn structured_tool_completion_output(
    tool: &str,
    status: &str,
    detail: impl Into<String>,
    data: serde_json::Value,
) -> String {
    format!(
        "{}{}",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({
            "tool": tool,
            "status": status,
            "detail": detail.into(),
            "data": data,
        })
    )
}

fn browse_completion_detail(url: &str, title: &str, extract: &str, content: &str) -> String {
    let mut lines = Vec::new();
    if title.trim().is_empty() {
        lines.push("Fetched page.".to_string());
    } else {
        lines.push(format!("Fetched page: {}", title.trim()));
    }
    lines.push(format!("URL: {}", url.trim()));
    lines.push(format!("Extract: {}", extract.trim()));
    let excerpt = runtime_truncate_chars(&runtime_collapse_whitespace(content), 1_200);
    if !excerpt.trim().is_empty() {
        lines.push(String::new());
        lines.push(format!("Excerpt: {}", excerpt));
    }
    lines.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMissingInputsPayload {
    pub action: String,
    pub missing: Vec<String>,
    #[serde(default)]
    pub sensitive_missing: Vec<String>,
    pub required: Vec<String>,
    pub provided: Vec<String>,
    pub query: String,
}

fn parse_json_object_output(output: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_json::from_str::<serde_json::Value>(output.trim()).ok()? {
        serde_json::Value::Object(object) => Some(object),
        _ => None,
    }
}

pub fn parse_workflow_action_marker(output: &str) -> Option<(String, String)> {
    if let Some(payload) = output.trim_start().strip_prefix(WORKFLOW_ACTION_MARKER) {
        let mut parts = payload.splitn(2, ':');
        let action = parts.next()?.trim();
        if action.is_empty() {
            return None;
        }
        let query = parts.next().unwrap_or("").to_string();
        return Some((action.to_string(), query));
    }

    let payload = parse_json_object_output(output)?;
    let action = payload
        .get("workflow_action")
        .or_else(|| payload.get("action"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let query = payload
        .get("query")
        .or_else(|| payload.get("user_query"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    Some((action.to_string(), query))
}

pub fn parse_workflow_missing_inputs_marker(output: &str) -> Option<WorkflowMissingInputsPayload> {
    if let Some(payload) = output
        .trim_start()
        .strip_prefix(WORKFLOW_MISSING_INPUTS_MARKER)
    {
        return serde_json::from_str::<WorkflowMissingInputsPayload>(payload).ok();
    }
    serde_json::from_str::<WorkflowMissingInputsPayload>(output.trim()).ok()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredToolCompletion {
    pub tool: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn parse_structured_tool_completion(output: &str) -> Option<StructuredToolCompletion> {
    if let Some(payload) = output.trim_start().strip_prefix(TOOL_COMPLETION_MARKER) {
        let payload = payload.lines().next().unwrap_or(payload).trim();
        return serde_json::from_str::<StructuredToolCompletion>(payload).ok();
    }
    serde_json::from_str::<StructuredToolCompletion>(output.trim()).ok()
}

pub fn parse_schedule_task_completion(output: &str) -> Option<StructuredToolCompletion> {
    parse_structured_tool_completion(output).filter(|completion| completion.tool == "schedule_task")
}

pub fn parse_watch_completion(output: &str) -> Option<StructuredToolCompletion> {
    parse_structured_tool_completion(output).filter(|completion| completion.tool == "watch")
}

#[cfg(test)]
pub fn parse_delegate_completion(output: &str) -> Option<StructuredToolCompletion> {
    parse_structured_tool_completion(output).filter(|completion| completion.tool == "delegate")
}

/// Isolation level for ephemeral Docker containers
#[cfg(feature = "docker")]
#[derive(Clone, Copy)]
enum ContainerIsolation {
    /// Strict: read-only root, no network, noexec /tmp. For shell commands.
    Strict,
    /// Standard: writable fs, no network egress by default. For code execution.
    Standard,
    /// Opt-in network-enabled profile for explicit user-approved sandbox execution.
    StandardWithNetwork,
}

#[cfg(feature = "docker")]
impl ContainerIsolation {
    fn label(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Standard => "standard",
            Self::StandardWithNetwork => "standard_with_network",
        }
    }

    fn network_access(self) -> bool {
        matches!(self, Self::StandardWithNetwork)
    }
}

#[cfg(feature = "docker")]
const CODE_EXECUTE_SANDBOX_DIR: &str = "/workspace";
#[cfg(feature = "docker")]
const CODE_EXECUTE_HOME_DIR: &str = "/workspace/home";
#[cfg(feature = "docker")]
const CODE_EXECUTE_TMP_DIR: &str = "/workspace/tmp";
#[cfg(feature = "docker")]
const CODE_EXECUTE_CACHE_DIR: &str = "/workspace/cache";
#[cfg(feature = "docker")]
const CODE_EXECUTE_PIP_CACHE_DIR: &str = "/workspace/pip-cache";
const CODE_EXECUTE_OUTPUT_RETENTION_SECS: u64 = 14 * 24 * 60 * 60;
const CODE_EXECUTE_NATIVE_TEMP_RETENTION_SECS: u64 = 14 * 24 * 60 * 60;

struct ActionReviewBuildInput<'a> {
    action_name: &'a str,
    source_kind: &'a str,
    fingerprint: String,
    verdict: &'a crate::security::action_guard::ActionSecurityVerdict,
    required_env: Vec<String>,
    missing_env: Vec<String>,
    requires_auth: bool,
    auth_configured: bool,
    notes: Vec<String>,
}

pub(crate) fn load_persisted_search_config(
    config_dir: &Path,
    data_dir: Option<&Path>,
) -> crate::actions::SearchConfig {
    if let Ok(manager) = SecureConfigManager::new_with_data_dir(config_dir, data_dir) {
        if manager.uses_storage_backend() {
            match manager.load_encrypted_json::<crate::actions::SearchConfig>(
                crate::core::runtime::config::SETTINGS_SEARCH_KEY,
            ) {
                Ok(Some(config)) => return config,
                Ok(None) => return crate::actions::SearchConfig::default(),
                Err(error) => {
                    tracing::warn!(
                        "Failed to load search config from settings storage: {}",
                        error
                    )
                }
            }
        }
    }

    std::fs::read_to_string(config_dir.join("search.toml"))
        .ok()
        .and_then(|content| toml::from_str::<crate::actions::SearchConfig>(&content).ok())
        .unwrap_or_default()
}

pub(crate) async fn load_persisted_search_config_async(
    config_dir: PathBuf,
    data_dir: Option<PathBuf>,
) -> crate::actions::SearchConfig {
    tokio::task::spawn_blocking(move || {
        load_persisted_search_config(&config_dir, data_dir.as_deref())
    })
    .await
    .unwrap_or_default()
}

pub(crate) fn save_persisted_search_config(
    config_dir: &Path,
    data_dir: Option<&Path>,
    config: &crate::actions::SearchConfig,
) -> Result<()> {
    if let Ok(manager) = SecureConfigManager::new_with_data_dir(config_dir, data_dir) {
        if manager.uses_storage_backend() {
            return manager
                .save_encrypted_json(crate::core::runtime::config::SETTINGS_SEARCH_KEY, config);
        }
    }

    let content = toml::to_string_pretty(config)?;
    std::fs::write(config_dir.join("search.toml"), content)?;
    Ok(())
}

pub(crate) async fn save_persisted_search_config_async(
    config_dir: PathBuf,
    data_dir: Option<PathBuf>,
    config: crate::actions::SearchConfig,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        save_persisted_search_config(&config_dir, data_dir.as_deref(), &config)
    })
    .await
    .map_err(|error| anyhow::anyhow!("search config persistence task failed: {}", error))?
}

/// Build search config: loads user settings from persistent settings storage,
/// injects API-backed secrets, auto-detects runtime-provided builtins such as
/// Lightpanda and the Playwright bridge, and applies the default free fallback
/// chain only when no chain is saved.
pub(crate) async fn build_search_config(
    config_dir: &Path,
    storage: Option<&crate::storage::Storage>,
) -> crate::actions::SearchConfig {
    let mut config = load_persisted_search_config_async(config_dir.to_path_buf(), None).await;

    if let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new(config_dir) {
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

            if let Some(api_key) = secrets
                .custom
                .get("search_exa_key")
                .filter(|value| !value.trim().is_empty())
                .cloned()
            {
                config.exa = Some(crate::actions::search::SearchBackend::Exa { api_key });
            } else if matches!(
                &config.exa,
                Some(crate::actions::search::SearchBackend::Exa { api_key })
                    if api_key.trim().is_empty()
            ) {
                config.exa = None;
            }

            if let Some(api_key) = secrets
                .custom
                .get("search_tavily_key")
                .filter(|value| !value.trim().is_empty())
                .cloned()
            {
                config.tavily = Some(crate::actions::search::SearchBackend::Tavily { api_key });
            } else if matches!(
                &config.tavily,
                Some(crate::actions::search::SearchBackend::Tavily { api_key })
                    if api_key.trim().is_empty()
            ) {
                config.tavily = None;
            }

            if let Some(api_key) = secrets
                .custom
                .get("search_perplexity_key")
                .filter(|value| !value.trim().is_empty())
                .cloned()
            {
                config.perplexity =
                    Some(crate::actions::search::SearchBackend::Perplexity { api_key });
            } else if matches!(
                &config.perplexity,
                Some(crate::actions::search::SearchBackend::Perplexity { api_key })
                    if api_key.trim().is_empty()
            ) {
                config.perplexity = None;
            }

            if let Some(api_key) = secrets
                .custom
                .get("search_firecrawl_key")
                .filter(|value| !value.trim().is_empty())
                .cloned()
            {
                config.firecrawl =
                    Some(crate::actions::search::SearchBackend::Firecrawl { api_key });
            } else if matches!(
                &config.firecrawl,
                Some(crate::actions::search::SearchBackend::Firecrawl { api_key })
                    if api_key.trim().is_empty()
            ) {
                config.firecrawl = None;
            }
        }
    }

    config.lightpanda_available = crate::integrations::lightpanda::is_available();
    if config.lightpanda_available {
        if let Some(path) = crate::integrations::lightpanda::binary_path() {
            tracing::debug!("Lightpanda available at {}", path.display());
        }
    } else {
        tracing::warn!(
            "Lightpanda binary not found in this runtime; free search fallback will skip it"
        );
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
    let health = crate::actions::search::SearchBackendHealthState::load(storage).await;

    config.with_health(health)
}

impl DurableStore for ActionRuntime {
    fn put_payload<'a>(
        &'a self,
        payload: ToolPayload,
        hints: PersistHints,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolPayload>> + Send + 'a>> {
        Box::pin(self.persist_tool_payload_if_needed(payload, hints))
    }
}

#[cfg(test)]
mod tests;
