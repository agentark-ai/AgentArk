//! Action Runtime - WASM Sandbox + Docker + Transactional Execution
//! Based on arXiv:2512.12806 "Fault-Tolerant Sandboxing"
//!
//! Features:
//! - WASM sandbox for lightweight, fast action execution
//! - Docker sandbox for heavier/untrusted operations
//! - Transactional filesystem with rollback capability

mod ark_inspect;
mod sandbox;
mod transaction;

pub use sandbox::{ActionSandbox, SandboxMode};
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
use crate::core::config::{AgentConfig, SecureConfigManager};
use crate::core::runtime_image;

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

#[derive(Debug, Clone, Serialize)]
pub struct SkillEvolutionApplyResult {
    pub skill_name: String,
    pub approved_ref: String,
    pub history_version: u32,
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

fn channel_target(argument_key: &str, default_target: &str) -> crate::actions::ActionChannelTarget {
    crate::actions::ActionChannelTarget {
        argument_key: argument_key.to_string(),
        default_target: default_target.to_string(),
    }
}

/// The action runtime that manages execution
pub struct ActionRuntime {
    config: RuntimeConfig,
    sandbox: ActionSandbox,
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
    "app_delete",
    "app_stop",
    "app_restart",
    "app_deploy",
    "shell",
    "code_execute",
    "browser_auto",
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
        crate::core::llm_provider::openai_provider_label(self.base_url.as_deref())
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

/// A loaded action ready for execution
struct LoadedAction {
    info: ActionDef,
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
#[cfg(feature = "docker")]
const CODE_EXECUTE_OUTPUT_RETENTION_SECS: u64 = 14 * 24 * 60 * 60;
#[cfg(feature = "docker")]
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

impl ActionRuntime {
    fn sanitize_upload_filename(raw: &str) -> String {
        let filename: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if filename.is_empty() {
            "file".to_string()
        } else {
            filename
        }
    }

    fn inline_code_execute_payloads(
        arguments: &serde_json::Value,
    ) -> Result<Vec<SandboxUploadFile>> {
        let Some(payloads) = arguments
            .get("file_payloads")
            .and_then(|value| value.as_array())
        else {
            return Ok(Vec::new());
        };
        let mut files = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let filename = payload
                .get("filename")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Each file_payload must include a filename"))?;
            let bytes_b64 = payload
                .get("bytes_b64")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Each file_payload must include bytes_b64"))?;
            let bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, bytes_b64)
                    .map_err(|e| {
                        anyhow::anyhow!("Invalid base64 file payload for '{}': {}", filename, e)
                    })?;
            files.push(SandboxUploadFile {
                filename: Self::sanitize_upload_filename(filename),
                content_type: payload
                    .get("content_type")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                bytes,
            });
        }
        Ok(files)
    }

    async fn collect_code_execute_files(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<Vec<SandboxUploadFile>> {
        let inline = Self::inline_code_execute_payloads(arguments)?;
        if !inline.is_empty() {
            return Ok(inline);
        }
        let mut files = Vec::new();
        if let Some(files_arr) = arguments.get("files").and_then(|v| v.as_array()) {
            for file_val in files_arr {
                let upload_id = file_val.as_str().ok_or_else(|| {
                    anyhow::anyhow!("Each code_execute file reference must be a string upload ID")
                })?;
                files.push(self.resolve_upload_for_sandbox(upload_id).await?);
            }
        }
        Ok(files)
    }

    fn upload_signature(
        filename: &str,
        content_type: Option<&str>,
        bytes: &[u8],
    ) -> serde_json::Value {
        let lower_name = filename.to_ascii_lowercase();
        let lower_ct = content_type.unwrap_or("").to_ascii_lowercase();
        let ext = lower_name
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .unwrap_or("");

        let mut detected = if bytes.starts_with(b"OggS") {
            if bytes
                .windows(b"OpusHead".len())
                .any(|win| win == b"OpusHead")
            {
                serde_json::json!({
                    "input_type": "audio",
                    "media_kind": "audio",
                    "mime": "audio/ogg; codecs=opus",
                    "extension": "opus",
                    "confidence": "high",
                    "source": "magic_bytes",
                })
            } else {
                serde_json::json!({
                    "input_type": "audio",
                    "media_kind": "audio",
                    "mime": "audio/ogg",
                    "extension": "ogg",
                    "confidence": "high",
                    "source": "magic_bytes",
                })
            }
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            serde_json::json!({
                "input_type": "audio",
                "media_kind": "audio",
                "mime": "audio/wav",
                "extension": "wav",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"ID3") || bytes.starts_with(&[0xFF, 0xFB]) {
            serde_json::json!({
                "input_type": "audio",
                "media_kind": "audio",
                "mime": "audio/mpeg",
                "extension": "mp3",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"fLaC") {
            serde_json::json!({
                "input_type": "audio",
                "media_kind": "audio",
                "mime": "audio/flac",
                "extension": "flac",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            let brand =
                String::from_utf8_lossy(&bytes[8..bytes.len().min(24)]).to_ascii_lowercase();
            let audio_brand =
                brand.contains("m4a") || brand.contains("m4b") || brand.contains("mp42");
            serde_json::json!({
                "input_type": if audio_brand { "audio" } else { "audio_video" },
                "media_kind": if audio_brand { "audio" } else { "audio_or_video" },
                "mime": if audio_brand { "audio/mp4" } else { "video/mp4" },
                "extension": if audio_brand { "m4a" } else { "mp4" },
                "confidence": "medium",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            serde_json::json!({
                "input_type": "audio_video",
                "media_kind": "audio_or_video",
                "mime": "video/webm",
                "extension": "webm",
                "confidence": "medium",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/png",
                "extension": "png",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/jpeg",
                "extension": "jpg",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/gif",
                "extension": "gif",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/webp",
                "extension": "webp",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"%PDF-") {
            serde_json::json!({
                "input_type": "document",
                "media_kind": "document",
                "mime": "application/pdf",
                "extension": "pdf",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"PK\x03\x04") {
            serde_json::json!({
                "input_type": "archive",
                "media_kind": "archive",
                "mime": "application/zip",
                "extension": "zip",
                "confidence": "medium",
                "source": "magic_bytes",
            })
        } else {
            serde_json::json!({
                "input_type": "unknown",
                "media_kind": "unknown",
                "mime": serde_json::Value::Null,
                "extension": ext,
                "confidence": "low",
                "source": "unresolved",
                "needs_deeper_inspection": true,
            })
        };

        if let Some(obj) = detected.as_object_mut() {
            obj.insert("filename".to_string(), serde_json::json!(filename));
            obj.insert("size_bytes".to_string(), serde_json::json!(bytes.len()));
            if let Some(content_type) = content_type {
                obj.insert(
                    "provided_content_type".to_string(),
                    serde_json::json!(content_type),
                );
            }
            if !lower_ct.is_empty() {
                obj.insert(
                    "provided_content_type_hint".to_string(),
                    serde_json::json!(lower_ct),
                );
            }
        }
        detected
    }

    fn sanitize_missing_binary_candidate(raw: &str) -> Option<String> {
        let candidate = raw
            .trim()
            .trim_matches(|ch: char| {
                !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '.' && ch != '+'
            })
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .trim();
        if candidate.is_empty()
            || candidate.len() > 80
            || !candidate
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'))
        {
            return None;
        }
        Some(candidate.to_string())
    }

    fn quoted_missing_binary_candidate(line: &str) -> Option<String> {
        for quote in ['\'', '"'] {
            let mut parts = line.split(quote);
            while let Some(_) = parts.next() {
                let Some(candidate) = parts.next() else {
                    break;
                };
                if let Some(cleaned) = Self::sanitize_missing_binary_candidate(candidate) {
                    return Some(cleaned);
                }
            }
        }
        None
    }

    fn detect_missing_binary_from_output(output: &str) -> Option<String> {
        let lower = output.to_ascii_lowercase();
        if let Some(idx) = lower.find("agentark_missing_binary:") {
            let raw = output[idx + "AGENTARK_MISSING_BINARY:".len()..]
                .lines()
                .next()
                .unwrap_or("")
                .trim();
            if let Some(candidate) = Self::sanitize_missing_binary_candidate(
                raw.split_whitespace().next().unwrap_or(raw),
            ) {
                return Some(candidate);
            }
        }

        for line in output.lines() {
            let lower_line = line.to_ascii_lowercase();
            for pattern in [": command not found", ": not found"] {
                if let Some(idx) = lower_line.find(pattern) {
                    let prefix = line[..idx].trim();
                    let after_shell_prefix = prefix.rsplit(':').next().unwrap_or(prefix);
                    let candidate = after_shell_prefix
                        .split_whitespace()
                        .last()
                        .unwrap_or(after_shell_prefix);
                    if let Some(cleaned) = Self::sanitize_missing_binary_candidate(candidate) {
                        return Some(cleaned);
                    }
                }
            }

            if lower_line.contains("no such file or directory")
                || lower_line.contains("is not recognized")
            {
                if let Some(candidate) = Self::quoted_missing_binary_candidate(line) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn build_sandbox_transcription_code() -> &'static str {
        r#"import json
import pathlib
import shutil
import sys

data_dir = pathlib.Path("/data")
files = [p for p in data_dir.iterdir() if p.is_file()]
if not files:
    raise SystemExit("No uploaded audio file was injected into /data.")

input_path = files[0]
if shutil.which("ffmpeg") is None:
    print("AGENTARK_MISSING_BINARY: ffmpeg")
    raise SystemExit(127)

import whisper

model = whisper.load_model("base")
result = model.transcribe(str(input_path))
print(json.dumps({
    "input_file": input_path.name,
    "text": (result.get("text") or "").strip()
}, ensure_ascii=False))
"#
    }

    fn control_plane_executor_client() -> Option<ExecutorClient> {
        let role = std::env::var("AGENTARK_STACK_ROLE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase());
        if !matches!(role.as_deref(), Some("control-plane" | "control")) {
            return None;
        }
        let client = ExecutorClient::new(ExecutorClientConfig::from_env()).ok()?;
        client.bearer_token()?;
        Some(client)
    }

    async fn execute_code_remote(
        &self,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_string();
        let code = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?
            .to_string();
        let env = arguments
            .get("env")
            .and_then(|value| value.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect::<BTreeMap<String, String>>()
            })
            .unwrap_or_default();
        let network_access = arguments
            .get("network_access")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let execution_contract = arguments.get("execution_contract").cloned();
        let file_payloads = self
            .collect_code_execute_files(arguments)
            .await?
            .into_iter()
            .map(|file| CodeExecuteFilePayload {
                filename: file.filename,
                bytes_b64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    file.bytes,
                ),
            })
            .collect::<Vec<_>>();
        let executor = Self::control_plane_executor_client()
            .ok_or_else(|| anyhow::anyhow!("Executor service is not configured"))?;
        let response = executor
            .execute_code(&crate::clients::CodeExecuteRequest {
                language,
                code,
                files: Vec::new(),
                file_payloads,
                env,
                network_access,
                execution_contract,
                auth_context: Some(auth_context.clone()),
            })
            .await?;
        if response.status.eq_ignore_ascii_case("ok") {
            if response.raw.is_object() {
                return Ok(serde_json::to_string(&response.raw)?);
            }
            return Ok(serde_json::to_string(&serde_json::json!({
                "output": response.output_text.unwrap_or_default(),
                "error": serde_json::Value::Null,
                "exit_code": 0,
                "files": response.output_files,
            }))?);
        }
        if response.raw.is_object() {
            let error = response
                .raw
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or(response.message.as_str());
            anyhow::bail!("{}", error);
        }
        anyhow::bail!("{}", response.message);
    }

    fn remap_workspace_alias_path(&self, raw: &str) -> Option<PathBuf> {
        let trimmed = raw.trim();
        const PREFIXES: &[&str] = &["/workspace", "/repo", "/project"];
        let matched = PREFIXES.iter().find(|prefix| {
            trimmed == **prefix
                || trimmed
                    .strip_prefix(**prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })?;
        let workspace_root = self.workspace_root();
        let suffix = trimmed.strip_prefix(matched).unwrap_or("");
        let relative = suffix.trim_start_matches('/');
        if relative.is_empty() {
            Some(workspace_root)
        } else {
            Some(workspace_root.join(relative))
        }
    }

    fn allowed_file_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            self.data_dir().to_path_buf(),
            self.actions_dir.clone(),
            self.config_dir.clone(),
            self.workspace_root(),
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
        Err(ToolPathAccessError::OutsideAllowedRoots {
            attempted_path: candidate.to_path_buf(),
            allowed_roots,
        }
        .into())
    }

    fn tool_path_looks_sensitive_file(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        let lower = name.trim().to_ascii_lowercase();
        lower == ".agentark_runtime_env"
            || lower == ".env"
            || lower.starts_with(".env.")
            || lower.ends_with(".pem")
            || lower.ends_with(".key")
            || lower.ends_with(".p12")
            || lower.ends_with(".pfx")
            || lower == "secrets.json"
            || lower == "credentials.json"
    }

    fn resolve_tool_read_path(&self, raw: &str) -> Result<PathBuf> {
        let candidate = self.absolutize_tool_path(raw)?;
        let resolved = candidate.canonicalize()?;
        self.ensure_tool_path_allowed(&resolved)?;
        if Self::tool_path_looks_sensitive_file(&resolved) {
            anyhow::bail!(
                "Refusing to read sensitive credential file '{}'. Use the secure credential store or app required_inputs flow instead.",
                resolved.display()
            );
        }
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
        let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
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

    fn parse_http_get_url(raw_url: &str) -> Result<reqwest::Url> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Missing URL");
        }
        let candidate = if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("https://{}", trimmed.trim_start_matches("//"))
        };
        let parsed = reqwest::Url::parse(&candidate)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("http_get only supports http:// and https:// URLs");
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("URL must include a host");
        }
        Ok(parsed)
    }

    fn http_get_url_is_privateish(url: &reqwest::Url) -> bool {
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        Self::host_is_explicitly_local(&host)
            || host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".home")
            || host.ends_with(".lan")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|ip| !Self::ip_is_public(ip))
    }

    async fn validate_http_get_url(&self, raw_url: &str) -> Result<reqwest::Url> {
        let parsed = Self::parse_http_get_url(raw_url)?;
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

    async fn resolve_http_get_url_for_context(
        &self,
        raw_url: &str,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<reqwest::Url> {
        if Self::direct_trusted_chat_tool_override(auth_context) {
            return Self::parse_http_get_url(raw_url);
        }
        self.validate_http_get_url(raw_url).await
    }

    async fn validate_connector_request_url(&self, raw_url: &str) -> Result<reqwest::Url> {
        let parsed = reqwest::Url::parse(raw_url)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("connector_request only supports http:// and https:// URLs");
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("connector_request requires a URL host");
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!("Embedded credentials are not allowed in connector_request URLs");
        }

        let host = parsed
            .host_str()
            .unwrap_or_default()
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if Self::host_is_explicitly_local(&host) {
            anyhow::bail!("connector_request cannot target localhost or loopback addresses");
        }
        if host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".home")
            || host.ends_with(".lan")
        {
            anyhow::bail!("connector_request cannot target local network hostnames");
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            if !Self::ip_is_public(ip) {
                anyhow::bail!("connector_request cannot target private or link-local IP addresses");
            }
            return Ok(parsed);
        }

        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut resolved_any = false;
        for addr in tokio::net::lookup_host((host.as_str(), port)).await? {
            resolved_any = true;
            if !Self::ip_is_public(addr.ip()) {
                anyhow::bail!(
                    "connector_request cannot target internal address {} resolved from {}",
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

    async fn resolve_upload_for_sandbox(&self, upload_id: &str) -> Result<SandboxUploadFile> {
        let normalized_id = uuid::Uuid::parse_str(upload_id.trim())
            .map_err(|_| anyhow::anyhow!("Invalid upload ID '{}'", upload_id))?
            .to_string();
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Upload-backed code execution requires storage"))?;
        let manifest = storage
            .load_upload_manifest(&normalized_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Upload '{}' was not found", normalized_id))?;
        let uploads_dir = managed_uploads_dir(self.data_dir());
        let uploads_root = tokio::fs::canonicalize(&uploads_dir)
            .await
            .with_context(|| {
                format!(
                    "Upload directory '{}' is not available",
                    uploads_dir.display()
                )
            })?;
        let resolved = tokio::fs::canonicalize(uploads_root.join(&manifest.stored_name))
            .await
            .with_context(|| {
                format!("Upload payload for '{}' is missing on disk", normalized_id)
            })?;
        if !resolved.starts_with(&uploads_root) {
            anyhow::bail!(
                "Upload '{}' resolved outside the managed upload directory",
                normalized_id
            );
        }
        let bytes = tokio::fs::read(&resolved)
            .await
            .with_context(|| format!("Failed to read upload payload '{}'", normalized_id))?;
        let filename: String = manifest.original_name.chars().collect::<String>();
        let filename = Self::sanitize_upload_filename(&filename);
        Ok(SandboxUploadFile {
            filename,
            content_type: manifest.content_type,
            bytes,
        })
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

    fn load_disabled_actions(
        path: &Path,
        settings: Option<&SecureConfigManager>,
    ) -> HashSet<String> {
        if let Some(manager) = settings.filter(|manager| manager.uses_storage_backend()) {
            match manager.load_encrypted_json::<Vec<String>>(
                crate::core::config::SETTINGS_DISABLED_ACTIONS_KEY,
            ) {
                Ok(Some(entries)) => {
                    return entries
                        .into_iter()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                Ok(None) => return HashSet::new(),
                Err(error) => {
                    tracing::warn!(
                        "Failed to load disabled actions from settings storage: {}",
                        error
                    )
                }
            }
        }

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
        let manager = self.settings_manager()?;
        if manager.uses_storage_backend() {
            manager
                .save_encrypted_json(crate::core::config::SETTINGS_DISABLED_ACTIONS_KEY, &list)?;
        } else {
            let raw = serde_json::to_vec_pretty(&list)?;
            tokio::fs::write(&self.disabled_actions_file, raw).await?;
        }
        Ok(())
    }

    fn load_action_reviews(
        path: &Path,
        settings: Option<&SecureConfigManager>,
    ) -> HashMap<String, ActionReviewRecord> {
        if let Some(manager) = settings.filter(|manager| manager.uses_storage_backend()) {
            match manager.load_encrypted_json::<HashMap<String, ActionReviewRecord>>(
                crate::core::config::SETTINGS_ACTION_REVIEWS_KEY,
            ) {
                Ok(Some(reviews)) => return reviews,
                Ok(None) => return HashMap::new(),
                Err(error) => {
                    tracing::warn!(
                        "Failed to load action reviews from settings storage: {}",
                        error
                    )
                }
            }
        }

        let raw = match std::fs::read(path) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };
        serde_json::from_slice::<HashMap<String, ActionReviewRecord>>(&raw).unwrap_or_default()
    }

    async fn save_action_reviews(&self) -> Result<()> {
        let reviews = self.action_reviews.read().await.clone();
        let manager = self.settings_manager()?;
        if manager.uses_storage_backend() {
            manager
                .save_encrypted_json(crate::core::config::SETTINGS_ACTION_REVIEWS_KEY, &reviews)?;
        } else {
            let raw = serde_json::to_vec_pretty(&reviews)?;
            tokio::fs::write(&self.action_reviews_file, raw).await?;
        }
        Ok(())
    }

    async fn upsert_action_review(&self, review: ActionReviewSnapshot) -> Result<()> {
        let mut reviews = self.action_reviews.write().await;
        let entry = reviews
            .entry(review.action_name.clone())
            .or_insert_with(ActionReviewRecord::default);
        let changed = entry.current.action_name.is_empty()
            || entry.current.fingerprint != review.fingerprint
            || entry.current.status != review.status
            || entry.current.blocked_reason != review.blocked_reason
            || entry.current.missing_env != review.missing_env
            || entry.current.auth_configured != review.auth_configured
            || entry.current.allow_execute != review.allow_execute
            || entry.current.permissions_needed != review.permissions_needed
            || entry.current.warnings != review.warnings
            || (entry.current.risk_score_10 - review.risk_score_10).abs() > f32::EPSILON;
        if changed && !entry.current.action_name.is_empty() {
            entry.history.push(entry.current.clone());
            if entry.history.len() > ACTION_REVIEW_HISTORY_LIMIT {
                let drop_count = entry.history.len() - ACTION_REVIEW_HISTORY_LIMIT;
                entry.history.drain(0..drop_count);
            }
        }
        entry.current = review;
        drop(reviews);
        self.save_action_reviews().await?;
        if changed {
            self.record_cross_layer_capability_correlation().await;
        }
        Ok(())
    }

    async fn remove_action_review(&self, name: &str) -> Result<()> {
        let mut reviews = self.action_reviews.write().await;
        reviews.remove(name);
        drop(reviews);
        self.save_action_reviews().await
    }

    async fn clear_action_secret_bindings(&self, action_name: &str) -> Result<()> {
        let manager =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let prefix = format!("action_envmap:{}:", action_name);
        manager.update_custom_secrets(|custom| {
            custom.retain(|key, _| !key.starts_with(&prefix));
            Ok(())
        })?;
        Ok(())
    }

    async fn remove_action_reviews<F>(&self, mut predicate: F) -> Result<usize>
    where
        F: FnMut(&str) -> bool,
    {
        let mut reviews = self.action_reviews.write().await;
        let before = reviews.len();
        reviews.retain(|name, _| !predicate(name));
        let removed = before.saturating_sub(reviews.len());
        drop(reviews);
        if removed > 0 {
            self.save_action_reviews().await?;
        }
        Ok(removed)
    }

    pub async fn get_action_review(&self, name: &str) -> Option<ActionReviewSnapshot> {
        self.action_reviews
            .read()
            .await
            .get(name)
            .map(|record| record.current.clone())
    }

    fn load_removed_bundled_actions(
        path: &Path,
        settings: Option<&SecureConfigManager>,
    ) -> HashSet<String> {
        if let Some(manager) = settings.filter(|manager| manager.uses_storage_backend()) {
            match manager.load_encrypted_json::<Vec<String>>(
                crate::core::config::SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
            ) {
                Ok(Some(entries)) => {
                    return entries
                        .into_iter()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                Ok(None) => return HashSet::new(),
                Err(error) => tracing::warn!(
                    "Failed to load removed bundled actions from settings storage: {}",
                    error
                ),
            }
        }

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

    async fn save_removed_bundled_actions(&self) -> Result<()> {
        let mut list: Vec<String> = self
            .removed_bundled_actions
            .read()
            .await
            .iter()
            .cloned()
            .collect();
        list.sort();
        let manager = self.settings_manager()?;
        if manager.uses_storage_backend() {
            manager.save_encrypted_json(
                crate::core::config::SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
                &list,
            )?;
        } else {
            let raw = serde_json::to_vec_pretty(&list)?;
            tokio::fs::write(&self.removed_bundled_actions_file, raw).await?;
        }
        Ok(())
    }

    /// Get the data directory (parent of actions_dir)
    fn data_dir(&self) -> &Path {
        self.actions_dir.parent().unwrap_or(&self.actions_dir)
    }

    fn settings_manager(&self) -> Result<SecureConfigManager> {
        SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))
    }

    fn workspace_root(&self) -> PathBuf {
        let configured = std::env::var("AGENTARK_WORKSPACE_ROOT")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let fallback = std::env::current_dir()
            .ok()
            .unwrap_or_else(|| self.data_dir().to_path_buf());
        match configured {
            Some(path) if path.is_absolute() => path,
            Some(path) => fallback.join(path),
            None => fallback,
        }
    }

    fn action_source_label(source: &ActionSource) -> &'static str {
        match source {
            ActionSource::System => "system",
            ActionSource::Bundled => "bundled",
            ActionSource::Custom => "custom",
        }
    }

    fn is_contextual_review_finding(
        finding: &crate::security::action_guard::AnalysisFinding,
    ) -> bool {
        let placeholder_like = finding.matched_text.contains('$')
            || finding.matched_text.contains("${")
            || finding.matched_text.contains("{{");
        match finding.category {
            crate::security::action_guard::FindingCategory::NetworkAccess
            | crate::security::action_guard::FindingCategory::EnvironmentAccess => true,
            crate::security::action_guard::FindingCategory::CredentialPattern => placeholder_like,
            _ => false,
        }
    }

    fn compute_review_risk_summary(
        static_analysis: &crate::security::action_guard::StaticAnalysisResult,
        blocked: bool,
    ) -> (f32, String, usize, usize) {
        let total_findings = static_analysis.findings.len();
        let contextual_findings = static_analysis
            .findings
            .iter()
            .filter(|f| Self::is_contextual_review_finding(f))
            .count();
        let mut score = ((static_analysis.total_severity as f32) / 4.0).min(10.0);
        let contextual_ratio = if total_findings > 0 {
            (contextual_findings as f32) / (total_findings as f32)
        } else {
            0.0
        };
        if contextual_ratio >= 0.75 {
            score *= 0.65;
        } else if contextual_ratio >= 0.5 {
            score *= 0.8;
        }
        match static_analysis.threat_level {
            crate::security::action_guard::ThreatLevel::Malicious => {
                if contextual_ratio >= 0.8 {
                    score = score.max(4.0);
                } else {
                    score = score.max(8.5);
                }
            }
            crate::security::action_guard::ThreatLevel::Suspicious => {
                score = score.max(5.0);
            }
            crate::security::action_guard::ThreatLevel::Clean => {}
        }
        if blocked && contextual_ratio < 0.8 {
            score = score.max(8.5);
        } else if blocked {
            score = score.max(5.0);
        }
        let score_10 = ((score.clamp(0.0, 10.0)) * 10.0).round() / 10.0;
        let band = if score_10 < 5.0 {
            "secure"
        } else if score_10 < 8.0 {
            "review"
        } else {
            "risky"
        };
        (
            score_10,
            band.to_string(),
            total_findings,
            contextual_findings,
        )
    }

    fn fingerprint_text(parts: &[impl AsRef<str>]) -> String {
        let mut hasher = Sha256::new();
        for part in parts {
            hasher.update(part.as_ref().as_bytes());
            hasher.update(b"\n---\n");
        }
        hex::encode(hasher.finalize())
    }

    fn is_env_var_style_key(key: &str) -> bool {
        !key.is_empty()
            && key.len() <= 128
            && key
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }

    fn builtin_env_from_agent_config(cfg: &AgentConfig, env: &str) -> bool {
        let mut providers: Vec<&crate::core::LlmProvider> = vec![&cfg.llm];
        if let Some(fallback) = cfg.llm_fallback.as_ref() {
            providers.push(fallback);
        }
        for slot in &cfg.model_pool.slots {
            if slot.enabled {
                providers.push(&slot.provider);
            }
        }
        match env {
            "OPENAI_API_KEY" => providers.into_iter().any(|provider| {
                matches!(
                    provider,
                    crate::core::LlmProvider::OpenAI { api_key, .. } if !api_key.is_empty()
                )
            }),
            "OPENROUTER_API_KEY" => providers.into_iter().any(|provider| {
                matches!(
                    provider,
                    crate::core::LlmProvider::OpenAI {
                        api_key,
                        base_url,
                        ..
                    } if !api_key.is_empty()
                        && base_url
                            .as_deref()
                            .unwrap_or("")
                            .contains("openrouter")
                )
            }),
            "ANTHROPIC_API_KEY" => providers.into_iter().any(|provider| {
                matches!(
                    provider,
                    crate::core::LlmProvider::Anthropic { api_key, .. } if !api_key.is_empty()
                )
            }),
            _ => false,
        }
    }

    fn extract_required_envs_from_frontmatter(frontmatter: &str) -> Vec<String> {
        let mut envs: Vec<String> = Vec::new();
        let unique_push = |out: &mut Vec<String>, value: String| {
            if !out.iter().any(|existing| existing == &value) {
                out.push(value);
            }
        };
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
            Self::collect_required_envs_from_yaml(&value, &mut envs, &unique_push);
        }

        envs
    }

    fn collect_required_envs_from_yaml<F>(
        value: &serde_yaml::Value,
        envs: &mut Vec<String>,
        unique_push: &F,
    ) where
        F: Fn(&mut Vec<String>, String),
    {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for value in map.values() {
                    Self::collect_required_envs_from_yaml(value, envs, unique_push);
                }
            }
            serde_yaml::Value::Sequence(items) => {
                for item in items {
                    Self::collect_required_envs_from_yaml(item, envs, unique_push);
                }
            }
            serde_yaml::Value::String(text) => {
                for item in Self::split_env_candidate_text(text) {
                    if item.contains('_') && Self::is_env_var_style_key(&item) {
                        unique_push(envs, item);
                    }
                }
            }
            _ => {}
        }
    }

    fn split_env_candidate_text(text: &str) -> Vec<String> {
        text.split(|ch: char| ch == ',' || ch.is_whitespace() || ch == '[' || ch == ']')
            .map(|item| item.trim().trim_matches('"').trim_matches('\''))
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn split_frontmatter_block(content: &str) -> Option<(&str, &str)> {
        let body = content
            .strip_prefix("---\r\n")
            .or_else(|| content.strip_prefix("---\n"))?;
        let mut consumed = 0usize;
        for segment in body.split_inclusive('\n') {
            let line = segment.trim_end_matches(&['\r', '\n'][..]);
            if line == "---" {
                let rest_start = consumed + segment.len();
                return Some((&body[..consumed], &body[rest_start..]));
            }
            consumed += segment.len();
        }
        None
    }

    fn parse_frontmatter_yaml(frontmatter: &str) -> Option<serde_yaml::Value> {
        let trimmed = frontmatter.trim();
        if trimmed.is_empty() {
            return None;
        }
        serde_yaml::from_str::<serde_yaml::Value>(trimmed).ok()
    }

    fn extract_auth_profile_id_from_frontmatter(frontmatter: &str) -> Option<String> {
        let yaml = Self::parse_frontmatter_yaml(frontmatter)?;
        let root = yaml.as_mapping()?;
        let direct_keys = ["auth_profile", "auth_profile_id"];
        for key in direct_keys {
            if let Some(value) = root
                .get(serde_yaml::Value::String(key.to_string()))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(value.to_string());
            }
        }
        let auth = root.get(serde_yaml::Value::String("auth".to_string()))?;
        let auth_map = auth.as_mapping()?;
        for key in ["profile", "profile_id", "id"] {
            if let Some(value) = auth_map
                .get(serde_yaml::Value::String(key.to_string()))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(value.to_string());
            }
        }
        None
    }

    fn extract_auth_env_exports_from_frontmatter(frontmatter: &str) -> BTreeMap<String, String> {
        let mut exports = BTreeMap::new();
        let Some(yaml) = Self::parse_frontmatter_yaml(frontmatter) else {
            return exports;
        };
        let Some(root) = yaml.as_mapping() else {
            return exports;
        };
        let Some(auth) = root.get(serde_yaml::Value::String("auth".to_string())) else {
            return exports;
        };
        let Some(auth_map) = auth.as_mapping() else {
            return exports;
        };
        let mapping = auth_map
            .get(serde_yaml::Value::String("env_exports".to_string()))
            .or_else(|| auth_map.get(serde_yaml::Value::String("exports".to_string())));
        let Some(mapping) = mapping.and_then(|value| value.as_mapping()) else {
            return exports;
        };
        for (key, value) in mapping {
            let Some(env_name) = key.as_str().map(str::trim).filter(|item| !item.is_empty()) else {
                continue;
            };
            let Some(source) = value
                .as_str()
                .map(str::trim)
                .filter(|item| !item.is_empty())
            else {
                continue;
            };
            exports.insert(env_name.to_string(), source.to_string());
        }
        exports
    }

    fn env_is_configured_for_action(
        cfg: &AgentConfig,
        custom: &std::collections::HashMap<String, String>,
        action_name: &str,
        env: &str,
    ) -> bool {
        let binding_key = format!("action_envmap:{}:{}", action_name, env);
        let target = custom.get(&binding_key).map(|s| s.as_str()).unwrap_or(env);
        if target == "builtin" {
            return Self::builtin_env_from_agent_config(cfg, env);
        }
        crate::core::secrets::has_user_secret(custom, target)
            || Self::builtin_env_from_agent_config(cfg, env)
    }

    fn plugin_secret_key(plugin_id: &str) -> String {
        format!("plugin_sdk_secret:{}", plugin_id.trim())
    }

    fn build_blocked_review(
        action_name: &str,
        source_kind: &str,
        fingerprint: String,
        reason: impl Into<String>,
    ) -> ActionReviewSnapshot {
        ActionReviewSnapshot {
            action_name: action_name.to_string(),
            source_kind: source_kind.to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status: ActionReviewStatus::Blocked,
            ready: false,
            allow_load: false,
            allow_execute: false,
            visible_in_catalog: false,
            integrity_ok: false,
            threat_level: "Unknown".to_string(),
            risk_band: "risky".to_string(),
            warnings: Vec::new(),
            findings: Vec::new(),
            required_env: Vec::new(),
            missing_env: Vec::new(),
            permissions_needed: Vec::new(),
            requires_auth: false,
            auth_configured: false,
            notes: Vec::new(),
            blocked_reason: Some(reason.into()),
            ..ActionReviewSnapshot::default()
        }
    }

    fn build_review_from_verdict(input: ActionReviewBuildInput<'_>) -> ActionReviewSnapshot {
        let ActionReviewBuildInput {
            action_name,
            source_kind,
            fingerprint,
            verdict,
            required_env,
            missing_env,
            requires_auth,
            auth_configured,
            notes,
        } = input;
        let blocked = !verdict.allow_load;
        let (risk_score_10, risk_band, total_findings, _contextual_findings) =
            Self::compute_review_risk_summary(&verdict.static_analysis, blocked);
        let mut warnings = verdict.warnings.clone();
        warnings.extend(notes.iter().cloned());
        let permissions_needed = verdict
            .permissions_needed
            .iter()
            .map(|perm| perm.to_string())
            .collect::<Vec<_>>();
        let blocked_reason = if blocked {
            verdict
                .warnings
                .first()
                .cloned()
                .or_else(|| Some("Blocked by security review".to_string()))
        } else if !auth_configured && requires_auth {
            Some("Required authentication is not configured.".to_string())
        } else if !missing_env.is_empty() {
            Some(format!(
                "Required secrets missing: {}",
                missing_env.join(", ")
            ))
        } else {
            None
        };
        let status = if blocked {
            ActionReviewStatus::Blocked
        } else if !auth_configured && requires_auth || !missing_env.is_empty() {
            ActionReviewStatus::NeedsSecrets
        } else if !warnings.is_empty() || !permissions_needed.is_empty() || risk_band == "review" {
            ActionReviewStatus::Warning
        } else {
            ActionReviewStatus::Ready
        };
        let allow_execute = matches!(
            status,
            ActionReviewStatus::Ready | ActionReviewStatus::Warning
        );
        ActionReviewSnapshot {
            action_name: action_name.to_string(),
            source_kind: source_kind.to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status,
            ready: allow_execute,
            allow_load: verdict.allow_load,
            allow_execute,
            visible_in_catalog: allow_execute,
            integrity_ok: verdict.integrity_ok,
            threat_level: format!("{:?}", verdict.static_analysis.threat_level),
            total_severity: verdict.static_analysis.total_severity,
            total_findings,
            risk_score_10,
            risk_band,
            warnings,
            findings: verdict.static_analysis.findings.clone(),
            required_env,
            missing_env,
            permissions_needed,
            requires_auth,
            auth_configured,
            notes,
            blocked_reason,
        }
    }

    fn apply_capability_report_to_review(
        review: &mut ActionReviewSnapshot,
        report: crate::security::capabilities::CapabilityLayerReport,
    ) {
        for observation in &report.observations {
            let selector = observation.selector();
            if !review
                .permissions_needed
                .iter()
                .any(|existing| existing == &selector)
            {
                review.permissions_needed.push(selector);
            }
        }
        for warning in &report.warnings {
            if !review.warnings.iter().any(|existing| existing == warning) {
                review.warnings.push(warning.clone());
            }
            if !review.notes.iter().any(|existing| existing == warning) {
                review.notes.push(warning.clone());
            }
        }
        for rule in &report.matched_rules {
            let note = format!("Capability policy rule '{}': {}", rule.id, rule.message);
            if !review.notes.iter().any(|existing| existing == &note) {
                review.notes.push(note);
            }
        }
        review.findings.extend(report.findings);
        review.total_findings = review.findings.len();
        review.total_severity = review.total_severity.saturating_add(report.total_severity);
        if report.risk_score_10 > review.risk_score_10 {
            review.risk_score_10 = report.risk_score_10;
            review.risk_band = report.risk_band.clone();
        }
        if matches!(
            report.threat_level,
            crate::security::action_guard::ThreatLevel::Malicious
        ) {
            review.threat_level = "Malicious".to_string();
        } else if matches!(
            report.threat_level,
            crate::security::action_guard::ThreatLevel::Suspicious
        ) && review.threat_level != "Malicious"
        {
            review.threat_level = "Suspicious".to_string();
        }

        if report.blocked {
            review.status = ActionReviewStatus::Blocked;
            review.ready = false;
            review.allow_load = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            review.blocked_reason = report
                .warnings
                .first()
                .cloned()
                .or_else(|| Some("Blocked by capability security policy.".to_string()));
            return;
        }

        if matches!(review.status, ActionReviewStatus::Ready)
            && (!report.warnings.is_empty()
                || report.risk_band == "review"
                || report.risk_band == "risky")
        {
            review.status = ActionReviewStatus::Warning;
            review.ready = true;
            review.allow_execute = true;
            review.visible_in_catalog = true;
        }
    }

    async fn record_security_event(
        &self,
        event_type: &str,
        severity: &str,
        message: String,
        source: Option<String>,
    ) {
        let Some(storage) = self.storage.as_ref() else {
            tracing::info!(
                event_type = event_type,
                severity = severity,
                source = source.as_deref().unwrap_or("runtime"),
                "{}",
                message
            );
            return;
        };
        let log = crate::storage::entities::security_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            severity: severity.to_string(),
            message: crate::security::redact_pii(&message),
            source,
            count: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(error) = storage.insert_security_log(&log).await {
            tracing::debug!("Failed to persist action security event: {}", error);
        }
    }

    async fn record_custom_messaging_channel_upsert_event(
        &self,
        channel: &crate::custom_messaging_channels::CustomMessagingChannelView,
        operation: &'static str,
    ) {
        self.record_security_event(
            if operation == "update" {
                "custom_messaging_channel_update"
            } else {
                "custom_messaging_channel_create"
            },
            "medium",
            format!(
                "Custom messaging channel {} by runtime action. channel_id={}",
                operation, channel.id
            ),
            Some(format!(
                "actor=runtime_action;source_kind=custom_channel;channel_id={}",
                channel.id
            )),
        )
        .await;

        let mut capabilities = vec![
            "calls-network".to_string(),
            "sends-message".to_string(),
            "sends-external".to_string(),
        ];
        if channel.requires_auth {
            capabilities.push("requests-secrets".to_string());
            capabilities.push("uses-auth-profile".to_string());
        }
        let report = crate::security::capabilities::evaluate_declared_capabilities(
            "custom_channel",
            &channel.id,
            &capabilities,
        );
        let severity = if report.blocked || report.risk_score_10 >= 8.0 {
            "high"
        } else if report.risk_score_10 >= 5.0 || !report.warnings.is_empty() {
            "medium"
        } else {
            "low"
        };
        let rules = report
            .matched_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        self.record_security_event(
            "capability_review",
            severity,
            format!(
                "Custom messaging channel capability review: channel_id={}, risk_score={}, capabilities=[{}], rules=[{}]",
                channel.id,
                report.risk_score_10,
                capabilities.join(", "),
                rules
            ),
            Some(format!(
                "actor=runtime_action;source_kind=custom_channel;channel_id={}",
                channel.id
            )),
        )
        .await;
    }

    fn review_event_severity(review: &ActionReviewSnapshot) -> &'static str {
        if matches!(review.status, ActionReviewStatus::Blocked) || review.risk_score_10 >= 8.0 {
            "high"
        } else if review.risk_score_10 >= 5.0 || !review.warnings.is_empty() {
            "medium"
        } else {
            "low"
        }
    }

    fn has_semantic_skill_review_marker(review: &ActionReviewSnapshot) -> bool {
        review.notes.iter().any(|note| {
            note.starts_with("Semantic capability review used configured model ")
                || note.starts_with("Semantic capability review used configured model '")
        })
    }

    async fn record_action_review_event(&self, review: &ActionReviewSnapshot) {
        if review.permissions_needed.is_empty()
            && review.warnings.is_empty()
            && !matches!(review.status, ActionReviewStatus::Blocked)
        {
            return;
        }
        let message = format!(
            "Action capability review: action='{}', source='{}', status='{:?}', risk_score={}, capabilities=[{}], warnings={}",
            review.action_name,
            review.source_kind,
            review.status,
            review.risk_score_10,
            review.permissions_needed.join(", "),
            review.warnings.len()
        );
        self.record_security_event(
            "capability_review",
            Self::review_event_severity(review),
            message,
            Some(format!(
                "source_kind={};action={}",
                review.source_kind, review.action_name
            )),
        )
        .await;
    }

    async fn record_cross_layer_capability_correlation(&self) {
        let observations = {
            let reviews = self.action_reviews.read().await;
            let mut observations = Vec::new();
            for record in reviews.values() {
                let review = &record.current;
                if matches!(review.status, ActionReviewStatus::Blocked)
                    || !review.allow_load
                    || review.permissions_needed.is_empty()
                {
                    continue;
                }
                observations.extend(
                    crate::security::capabilities::observations_from_declared_capabilities(
                        &review.source_kind,
                        &review.action_name,
                        &review.permissions_needed,
                    ),
                );
            }
            observations
        };
        let Some(report) =
            crate::security::capabilities::evaluate_cross_layer_capabilities(observations)
        else {
            return;
        };
        let rules = report
            .matched_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let subjects = report
            .observations
            .iter()
            .map(|observation| format!("{}:{}", observation.layer, observation.entity_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        self.record_security_event(
            "capability_correlation",
            "high",
            format!(
                "Cross-layer capability policy match: rules=[{}], subjects=[{}]",
                rules, subjects
            ),
            Some("scope=runtime".to_string()),
        )
        .await;
    }

    fn prune_cli_auth_exported_envs(review: &mut ActionReviewSnapshot, binding: &CliToolBinding) {
        if binding.auth_env_exports.is_empty() {
            return;
        }
        review
            .missing_env
            .retain(|env| !binding.auth_env_exports.contains_key(env));
    }

    fn reconcile_dynamic_review_state(review: &mut ActionReviewSnapshot) {
        if matches!(review.status, ActionReviewStatus::Blocked) {
            review.ready = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            return;
        }

        if !review.missing_env.is_empty() || (review.requires_auth && !review.auth_configured) {
            review.status = ActionReviewStatus::NeedsSecrets;
            review.ready = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            if review.blocked_reason.is_none() {
                review.blocked_reason = if !review.missing_env.is_empty() {
                    Some(format!(
                        "Required secrets missing: {}",
                        review.missing_env.join(", ")
                    ))
                } else {
                    Some("Required authentication is not configured.".to_string())
                };
            }
        } else {
            review.ready = review.allow_load;
            review.allow_execute = review.allow_load;
            review.visible_in_catalog = review.allow_load;
            review.blocked_reason = None;
            if matches!(
                review.status,
                ActionReviewStatus::NeedsSecrets | ActionReviewStatus::Unreviewed
            ) {
                review.status = if review.warnings.is_empty() {
                    ActionReviewStatus::Ready
                } else {
                    ActionReviewStatus::Warning
                };
            }
        }
    }

    async fn compute_missing_required_envs(
        &self,
        action_name: &str,
        required_env: &[String],
    ) -> Result<Vec<String>> {
        if required_env.is_empty() {
            return Ok(Vec::new());
        }
        let manager =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let config = manager.load()?;
        let secrets = manager.load_secrets()?;
        let custom = &secrets.custom;
        let mut missing = Vec::new();
        for env in required_env {
            if !Self::env_is_configured_for_action(&config, custom, action_name, env) {
                missing.push(env.clone());
            }
        }
        Ok(missing)
    }

    async fn auth_profile_status(&self, auth_profile_id: &str) -> Result<(bool, Vec<String>)> {
        let storage = self
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage is unavailable for auth profile lookups"))?;
        let view =
            crate::core::auth_profiles::AuthProfileControlPlane::get(&storage, auth_profile_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("Auth profile '{}' was not found", auth_profile_id)
                })?;
        let mut notes = Vec::new();
        if let Some(reason) = view.blocked_reason {
            notes.push(reason);
        }
        Ok((view.ready, notes))
    }

    async fn resolve_auth_profile_http(
        &self,
        auth_profile_id: &str,
    ) -> Result<crate::core::auth_profiles::AuthProfileResolution> {
        let storage = self
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage is unavailable for auth profile lookups"))?;
        crate::core::auth_profiles::AuthProfileControlPlane::resolve_http(&storage, auth_profile_id)
            .await
    }

    fn capabilities_frontmatter(capabilities: &[String]) -> String {
        if capabilities.is_empty() {
            String::new()
        } else {
            let mut mapping = serde_yaml::Mapping::new();
            mapping.insert(
                serde_yaml::Value::String("permissions".to_string()),
                serde_yaml::Value::Sequence(
                    capabilities
                        .iter()
                        .map(|capability| serde_yaml::Value::String(capability.clone()))
                        .collect(),
                ),
            );
            serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
                .unwrap_or_else(|_| "permissions: []".to_string())
                .trim_end()
                .to_string()
        }
    }

    async fn review_markdown_action(
        &self,
        action_dir: &Path,
        info: &ActionDef,
        workflow_content: &str,
        frontmatter: &str,
    ) -> Result<ActionReviewSnapshot> {
        let Some(guard) = self.action_guard.as_ref() else {
            let fingerprint = crate::security::ActionGuard::compute_bundle_hash(action_dir)
                .unwrap_or_else(|_| Self::fingerprint_text(&[workflow_content]));
            return Ok(Self::build_blocked_review(
                &info.name,
                Self::action_source_label(&info.source),
                fingerprint,
                "Action security is unavailable, so user-added skills are not loadable.",
            ));
        };
        let verdict = guard
            .evaluate_action(action_dir, &info.name, workflow_content, frontmatter)
            .await?;
        let required_env = Self::extract_required_envs_from_frontmatter(frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let auth_profile_id = Self::extract_auth_profile_id_from_frontmatter(frontmatter);
        let (requires_auth, auth_configured, mut notes) =
            if let Some(auth_profile_id) = auth_profile_id.as_deref() {
                let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                (true, ready, notes)
            } else {
                (false, true, Vec::new())
            };
        if let Some(auth_profile_id) = auth_profile_id.as_deref() {
            notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
        }
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[workflow_content, frontmatter]));
        Ok(Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: Self::action_source_label(&info.source),
            fingerprint,
            verdict: &verdict,
            required_env,
            missing_env,
            requires_auth,
            auth_configured,
            notes,
        }))
    }

    async fn review_cli_action(
        &self,
        action_dir: &Path,
        info: &ActionDef,
        skill_markdown: &str,
        frontmatter: &str,
        binding: &CliToolBinding,
    ) -> Result<ActionReviewSnapshot> {
        let mut review = self
            .review_markdown_action(action_dir, info, skill_markdown, frontmatter)
            .await?;
        if binding.auth_profile_id.is_some() {
            Self::prune_cli_auth_exported_envs(&mut review, binding);
            if binding.auth_env_exports.is_empty() {
                review.auth_configured = false;
                review.blocked_reason = Some(
                    "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string(),
                );
                let note = "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string();
                if !review.notes.iter().any(|existing| existing == &note) {
                    review.notes.push(note);
                }
            } else {
                let mut exported_envs =
                    binding.auth_env_exports.keys().cloned().collect::<Vec<_>>();
                exported_envs.sort();
                let note = format!("CLI auth exports: {}.", exported_envs.join(", "));
                if !review.notes.iter().any(|existing| existing == &note) {
                    review.notes.push(note);
                }
            }
            Self::reconcile_dynamic_review_state(&mut review);
        }
        let executable_ok = std::path::Path::new(&binding.executable_path).is_file();
        if executable_ok {
            return Ok(review);
        }
        review.status = ActionReviewStatus::NeedsSecrets;
        review.ready = false;
        review.allow_execute = false;
        review.visible_in_catalog = false;
        review.blocked_reason = Some(format!(
            "CLI executable '{}' is not present on this machine.",
            binding.executable_path
        ));
        let note =
            "CLI skills are machine-specific and must be revalidated after reload.".to_string();
        if !review.notes.iter().any(|existing| existing == &note) {
            review.notes.push(note);
        }
        Ok(review)
    }

    fn url_review_notes(url_str: &str) -> Vec<String> {
        let mut notes = Vec::new();
        if let Ok(url) = reqwest::Url::parse(url_str) {
            if url.scheme() != "https" {
                notes.push(format!("Remote endpoint '{}' does not use HTTPS.", url_str));
            }
            if let Some(host) = url.host_str() {
                let is_private = if host.eq_ignore_ascii_case("localhost") {
                    true
                } else if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                    match ip {
                        std::net::IpAddr::V4(v4) => {
                            v4.is_private() || v4.is_loopback() || v4.is_link_local()
                        }
                        std::net::IpAddr::V6(v6) => {
                            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
                        }
                    }
                } else {
                    false
                };
                if is_private {
                    notes.push(format!(
                        "Remote endpoint '{}' resolves to a private or loopback host.",
                        url_str
                    ));
                }
            }
        }
        notes
    }

    async fn review_plugin_action(
        &self,
        info: &ActionDef,
        binding: &PluginBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.base_url,
            &serde_json::to_string(&info.input_schema).unwrap_or_default(),
            &info.capabilities.join(","),
        ]);
        let Some(guard) = self.action_guard.as_ref() else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "plugin",
                fingerprint,
                "Action security is unavailable, so plugin actions are not loadable.",
            ));
        };
        let mut notes = Self::url_review_notes(&binding.base_url);
        let auth_configured = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
            let (ready, auth_notes) = self.auth_profile_status(auth_profile_id).await?;
            notes.extend(auth_notes);
            ready
        } else {
            binding.auth_configured
        };
        let frontmatter = Self::capabilities_frontmatter(&info.capabilities);
        let content = format!(
            "name: {}\nsource: plugin\naction: {}\nbase_url: {}\ndescription: {}\ncapabilities: {}\ninput_schema: {}",
            info.name,
            binding.action_name,
            binding.base_url,
            info.description,
            info.capabilities.join(", "),
            serde_json::to_string_pretty(&info.input_schema).unwrap_or_default()
        );
        let verdict = guard
            .evaluate_inline_action(&info.name, &content, &frontmatter, notes.clone())
            .await?;
        let mut review = Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: "plugin",
            fingerprint,
            verdict: &verdict,
            required_env: Vec::new(),
            missing_env: Vec::new(),
            requires_auth: binding.auth_required,
            auth_configured,
            notes,
        });
        let capability_report = crate::security::capabilities::evaluate_declared_capabilities(
            "plugin",
            &info.name,
            &info.capabilities,
        );
        Self::apply_capability_report_to_review(&mut review, capability_report);
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    async fn review_custom_api_action(
        &self,
        info: &ActionDef,
        binding: &CustomApiBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.base_url,
            &binding.path,
            &binding.method,
            &serde_json::to_string(&info.input_schema).unwrap_or_default(),
            &info.capabilities.join(","),
        ]);
        let Some(guard) = self.action_guard.as_ref() else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "custom_api",
                fingerprint,
                "Action security is unavailable, so custom API actions are not loadable.",
            ));
        };
        if matches!(
            binding.auth_mode,
            crate::custom_apis::CustomApiAuthMode::OAuth2
        ) && binding.auth_profile_id.is_none()
        {
            return Ok(Self::build_blocked_review(
                &info.name,
                "custom_api",
                fingerprint,
                "OAuth2 custom API actions require a bound auth profile.",
            ));
        }
        let mut notes = Self::url_review_notes(&binding.base_url);
        if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
        }
        let frontmatter = Self::capabilities_frontmatter(&info.capabilities);
        let content = format!(
            "name: {}\nsource: custom_api\napi: {}\noperation: {}\nmethod: {}\nbase_url: {}\npath: {}\nauth_mode: {:?}\ncapabilities: {}\ninput_schema: {}",
            info.name,
            binding.api_name,
            binding.operation_name,
            binding.method,
            binding.base_url,
            binding.path,
            binding.auth_mode,
            info.capabilities.join(", "),
            serde_json::to_string_pretty(&info.input_schema).unwrap_or_default()
        );
        let verdict = guard
            .evaluate_inline_action(&info.name, &content, &frontmatter, notes.clone())
            .await?;
        let requires_auth = binding.auth_profile_id.is_some()
            || !matches!(
                binding.auth_mode,
                crate::custom_apis::CustomApiAuthMode::None
            );
        let auth_configured = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            let (ready, auth_notes) = self.auth_profile_status(auth_profile_id).await?;
            notes.extend(auth_notes);
            ready
        } else if requires_auth {
            let manager =
                SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
            manager
                .get_custom_secret(&binding.secret_key)?
                .is_some_and(|value| !value.trim().is_empty())
        } else {
            true
        };
        let mut review = Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: "custom_api",
            fingerprint,
            verdict: &verdict,
            required_env: Vec::new(),
            missing_env: Vec::new(),
            requires_auth,
            auth_configured,
            notes,
        });
        let capability_report = crate::security::capabilities::evaluate_declared_capabilities(
            "custom_api",
            &info.name,
            &info.capabilities,
        );
        Self::apply_capability_report_to_review(&mut review, capability_report);
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    async fn review_extension_pack_action(
        &self,
        info: &ActionDef,
        binding: &ExtensionPackActionBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.pack_id,
            &binding.feature_id,
            &binding.action_name,
            &binding.binding_kind,
            &serde_json::to_string(&info.input_schema).unwrap_or_default(),
            &info.capabilities.join(","),
        ]);
        let Some(action_guard) = self.action_guard.as_ref() else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "extension_pack",
                fingerprint,
                "Action security is unavailable, so extension-pack actions are not loadable.",
            ));
        };
        let Some(registry) = self.extension_pack_registry.as_ref() else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "extension_pack",
                fingerprint,
                "Extension-pack registry is unavailable in this runtime.",
            ));
        };

        let pack = {
            let guard = registry.read().await;
            guard.get_pack(&binding.pack_id).await?
        };
        let Some(pack) = pack else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "extension_pack",
                fingerprint,
                format!("Extension pack '{}' was not found.", binding.pack_id),
            ));
        };

        let mut notes = Vec::new();
        notes.push(format!(
            "Uses extension pack '{}' ({}).",
            pack.manifest.name, pack.manifest.id
        ));
        notes.push(format!("Feature '{}'.", binding.feature_id));
        notes.push(format!("Binding kind: {}.", binding.binding_kind));
        if let Some(connection_id) = binding.connection_id.as_deref() {
            notes.push(format!("Connection '{}'.", connection_id));
        }
        if matches!(
            pack.trust_level,
            crate::extension_packs::ExtensionPackTrustLevel::Unverified
        ) {
            notes.push("Pack is installed as unverified.".to_string());
        }

        let frontmatter = Self::capabilities_frontmatter(&info.capabilities);
        let content = format!(
            "name: {}\nsource: extension_pack\npack_id: {}\npack_name: {}\nfeature_id: {}\nbinding_kind: {}\ndescription: {}\ncapabilities: {}\ninput_schema: {}",
            info.name,
            binding.pack_id,
            pack.manifest.name,
            binding.feature_id,
            binding.binding_kind,
            info.description,
            info.capabilities.join(", "),
            serde_json::to_string_pretty(&info.input_schema).unwrap_or_default()
        );
        let verdict = action_guard
            .evaluate_inline_action(&info.name, &content, &frontmatter, notes.clone())
            .await?;
        let mut review = Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: "extension_pack",
            fingerprint,
            verdict: &verdict,
            required_env: Vec::new(),
            missing_env: Vec::new(),
            requires_auth: false,
            auth_configured: true,
            notes,
        });
        if matches!(
            pack.trust_level,
            crate::extension_packs::ExtensionPackTrustLevel::Unverified
        ) && (!binding.read_only || binding.binding_kind.eq_ignore_ascii_case("local_cli"))
        {
            review.status = ActionReviewStatus::Blocked;
            review.ready = false;
            review.allow_load = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            review.blocked_reason = Some(
                "Unverified extension packs may not expose host CLI or write-capable actions."
                    .to_string(),
            );
        }
        let capability_report = crate::security::capabilities::evaluate_declared_capabilities(
            "extension_pack",
            &info.name,
            &info.capabilities,
        );
        Self::apply_capability_report_to_review(&mut review, capability_report);
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    async fn review_mcp_action(
        &self,
        info: &ActionDef,
        binding: &McpBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.server_id,
            &binding.server_name,
            &info.capabilities.join(","),
        ]);
        let Some(guard) = self.action_guard.as_ref() else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "mcp",
                fingerprint,
                "Action security is unavailable, so MCP actions are not loadable.",
            ));
        };
        let frontmatter = Self::capabilities_frontmatter(&info.capabilities);
        let content = format!(
            "name: {}\nsource: mcp\nserver_id: {}\nserver_name: {}\nkind: {:?}\ndescription: {}\ncapabilities: {}\ninput_schema: {}",
            info.name,
            binding.server_id,
            binding.server_name,
            binding.kind,
            info.description,
            info.capabilities.join(", "),
            serde_json::to_string_pretty(&info.input_schema).unwrap_or_default()
        );
        let mut warnings = binding.warnings.clone();
        let auth_configured = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            warnings.push(format!("Uses auth profile '{}'.", auth_profile_id));
            let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
            warnings.extend(notes);
            ready
        } else {
            binding.auth_configured
        };
        let verdict = guard
            .evaluate_inline_action(&info.name, &content, &frontmatter, warnings.clone())
            .await?;
        let mut review = Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: "mcp",
            fingerprint,
            verdict: &verdict,
            required_env: Vec::new(),
            missing_env: Vec::new(),
            requires_auth: binding.auth_required,
            auth_configured,
            notes: warnings,
        });
        let capability_report = crate::security::capabilities::evaluate_declared_capabilities(
            "mcp",
            &info.name,
            &info.capabilities,
        );
        Self::apply_capability_report_to_review(&mut review, capability_report);
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    pub async fn refresh_action_review_state(
        &self,
        action_name: &str,
    ) -> Result<Option<ActionReviewSnapshot>> {
        let loaded = {
            let actions = self.actions.read().await;
            actions.get(action_name).map(|action| {
                (
                    action.info.clone(),
                    action.cli_binding.clone(),
                    action.plugin_binding.clone(),
                    action.custom_api_binding.clone(),
                    action.mcp_binding.clone(),
                    action.extension_pack_binding.clone(),
                )
            })
        };
        let Some((
            info,
            cli_binding,
            plugin_binding,
            custom_api_binding,
            mcp_binding,
            extension_pack_binding,
        )) = loaded
        else {
            return Ok(None);
        };
        let mut review = match self.get_action_review(action_name).await {
            Some(review) => review,
            None => return Ok(None),
        };

        if info.source != ActionSource::System {
            if let Some(action_dir) = info
                .file_path
                .as_deref()
                .and_then(|file_path| Path::new(file_path).parent().map(Path::to_path_buf))
            {
                match crate::security::ActionGuard::compute_bundle_hash(&action_dir) {
                    Ok(current_fingerprint)
                        if !review.fingerprint.is_empty()
                            && current_fingerprint != review.fingerprint =>
                    {
                        let note = "Skill files changed on disk outside the reviewed API path; re-import or update the skill to run semantic review again.".to_string();
                        review.status = ActionReviewStatus::Blocked;
                        review.ready = false;
                        review.allow_load = false;
                        review.allow_execute = false;
                        review.visible_in_catalog = false;
                        review.integrity_ok = false;
                        review.threat_level = "Malicious".to_string();
                        review.risk_band = "risky".to_string();
                        review.risk_score_10 = review.risk_score_10.max(8.5);
                        review.total_severity = review.total_severity.saturating_add(10);
                        review.blocked_reason = Some(note.clone());
                        if !review.warnings.iter().any(|existing| existing == &note) {
                            review.warnings.push(note.clone());
                        }
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note.clone());
                        }
                        review
                            .findings
                            .push(crate::security::action_guard::AnalysisFinding {
                                category:
                                    crate::security::action_guard::FindingCategory::BundleShape,
                                description:
                                    "Reviewed skill fingerprint no longer matches disk content."
                                        .to_string(),
                                matched_text: "disk-content-changed-after-review".to_string(),
                                line_number: 1,
                                severity: 10,
                                file_path: info.file_path.clone(),
                            });
                        review.total_findings = review.findings.len();
                        self.upsert_action_review(review.clone()).await?;
                        {
                            let mut disabled = self.disabled_actions.write().await;
                            if disabled.insert(action_name.to_string()) {
                                drop(disabled);
                                self.save_disabled_actions().await?;
                            }
                        }
                        self.record_action_review_event(&review).await;
                        return Ok(Some(review));
                    }
                    Err(error) => {
                        let note = format!(
                            "Unable to re-check reviewed skill bundle fingerprint: {}",
                            error
                        );
                        review.status = ActionReviewStatus::Blocked;
                        review.ready = false;
                        review.allow_load = false;
                        review.allow_execute = false;
                        review.visible_in_catalog = false;
                        review.integrity_ok = false;
                        review.threat_level = "Malicious".to_string();
                        review.risk_band = "risky".to_string();
                        review.risk_score_10 = review.risk_score_10.max(8.5);
                        review.total_severity = review.total_severity.saturating_add(10);
                        review.blocked_reason = Some(note.clone());
                        if !review.warnings.iter().any(|existing| existing == &note) {
                            review.warnings.push(note.clone());
                        }
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note.clone());
                        }
                        self.upsert_action_review(review.clone()).await?;
                        {
                            let mut disabled = self.disabled_actions.write().await;
                            if disabled.insert(action_name.to_string()) {
                                drop(disabled);
                                self.save_disabled_actions().await?;
                            }
                        }
                        self.record_action_review_event(&review).await;
                        return Ok(Some(review));
                    }
                    _ => {}
                }
            }
        }

        if matches!(review.status, ActionReviewStatus::Blocked) {
            return Ok(Some(review));
        }

        if !review.required_env.is_empty() {
            review.missing_env = self
                .compute_missing_required_envs(action_name, &review.required_env)
                .await?;
        }

        let mut cli_executable_missing = None::<String>;
        if let Some(binding) = cli_binding {
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                review.requires_auth = true;
                Self::prune_cli_auth_exported_envs(&mut review, &binding);
                if binding.auth_env_exports.is_empty() {
                    review.auth_configured = false;
                    review.blocked_reason = Some(
                        "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string(),
                    );
                    let note = "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string();
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                } else {
                    let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                    review.auth_configured = ready;
                    for note in notes {
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note);
                        }
                    }
                    let mut exported_envs =
                        binding.auth_env_exports.keys().cloned().collect::<Vec<_>>();
                    exported_envs.sort();
                    let note = format!("CLI auth exports: {}.", exported_envs.join(", "));
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                }
            }
            if !std::path::Path::new(&binding.executable_path).is_file() {
                cli_executable_missing = Some(binding.executable_path.clone());
            }
        }

        if let Some(binding) = plugin_binding {
            review.requires_auth = binding.auth_required;
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                review.auth_configured = ready;
                for note in notes {
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                }
            } else if binding.auth_required {
                let manager = SecureConfigManager::new_with_data_dir(
                    &self.config_dir,
                    Some(self.data_dir()),
                )?;
                review.auth_configured = manager
                    .get_custom_secret(&Self::plugin_secret_key(&binding.plugin_id))?
                    .is_some_and(|value| !value.trim().is_empty());
            }
        }

        if let Some(binding) = custom_api_binding {
            let requires_auth = binding.auth_profile_id.is_some()
                || !matches!(
                    binding.auth_mode,
                    crate::custom_apis::CustomApiAuthMode::None
                );
            review.requires_auth = requires_auth;
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                review.auth_configured = ready;
                for note in notes {
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                }
            } else if requires_auth {
                let manager = SecureConfigManager::new_with_data_dir(
                    &self.config_dir,
                    Some(self.data_dir()),
                )?;
                review.auth_configured = manager
                    .get_custom_secret(&binding.secret_key)?
                    .is_some_and(|value| !value.trim().is_empty());
            }
        }

        if let Some(binding) = mcp_binding {
            review.requires_auth = binding.auth_required;
            review.auth_configured =
                if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                    let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                    for note in notes {
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note);
                        }
                    }
                    ready
                } else {
                    binding.auth_configured
                };
        }

        if let Some(binding) = extension_pack_binding {
            let note = format!(
                "Uses extension pack '{}' feature '{}'.",
                binding.pack_id, binding.feature_id
            );
            if !review.notes.iter().any(|existing| existing == &note) {
                review.notes.push(note);
            }
        }

        Self::reconcile_dynamic_review_state(&mut review);

        if let Some(executable_path) = cli_executable_missing {
            review.status = ActionReviewStatus::NeedsSecrets;
            review.ready = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            review.blocked_reason = Some(format!(
                "CLI executable '{}' is not present on this machine.",
                executable_path
            ));
            let note =
                "CLI skills are machine-specific and must be revalidated after reload.".to_string();
            if !review.notes.iter().any(|existing| existing == &note) {
                review.notes.push(note);
            }
        }

        review.source_kind = if info.source == ActionSource::System {
            review.source_kind
        } else {
            Self::action_source_label(&info.source).to_string()
        };
        self.upsert_action_review(review.clone()).await?;
        Ok(Some(review))
    }

    #[cfg(test)]
    fn find_project_root_from_path(start: &Path) -> Option<PathBuf> {
        let mut dir = if start.is_file() {
            start.parent()?
        } else {
            start
        };
        loop {
            if dir.join("Cargo.toml").exists() {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
    }

    fn bundled_skill_dirs(&self) -> Vec<PathBuf> {
        // Repo-local bundled markdown skills are disabled for this install.
        Vec::new()
    }

    fn is_runtime_owned_bundled_dir(path: &Path) -> bool {
        let _ = path;
        false
    }

    async fn delete_runtime_owned_bundled_skill_dir(&self, name: &str) -> Result<()> {
        for bundled_dir in self.bundled_skill_dirs() {
            if !Self::is_runtime_owned_bundled_dir(&bundled_dir) {
                continue;
            }
            let action_dir = bundled_dir.join(name);
            if action_dir.exists() {
                tokio::fs::remove_dir_all(&action_dir).await?;
            }
        }
        Ok(())
    }

    pub async fn new(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        let settings = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir)).ok();
        let config_path = config_dir.join("runtime.toml");
        let config: RuntimeConfig = if let Some(manager) = settings
            .as_ref()
            .filter(|manager| manager.uses_storage_backend())
        {
            match manager
                .load_encrypted_json::<RuntimeConfig>(crate::core::config::SETTINGS_RUNTIME_KEY)
            {
                Ok(Some(config)) => config,
                Ok(None) => {
                    let default = RuntimeConfig::default();
                    manager
                        .save_encrypted_json(crate::core::config::SETTINGS_RUNTIME_KEY, &default)?;
                    default
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to load runtime config from settings storage: {}",
                        error
                    );
                    RuntimeConfig::default()
                }
            }
        } else {
            if config_path.exists() {
                let content = std::fs::read_to_string(&config_path)?;
                toml::from_str(&content)?
            } else {
                let default = RuntimeConfig::default();
                let content = toml::to_string_pretty(&default)?;
                std::fs::write(&config_path, content)?;
                default
            }
        };

        // User-owned skills go in the data dir and survive release updates.
        let actions_dir = data_dir.join("skills");
        std::fs::create_dir_all(&actions_dir)?;
        let cli_skills_dir = data_dir.join("cli_skills");
        std::fs::create_dir_all(&cli_skills_dir)?;
        let disabled_actions_file = data_dir.join("disabled_actions.json");
        let disabled_actions =
            Self::load_disabled_actions(&disabled_actions_file, settings.as_ref());
        let action_reviews_file = data_dir.join("action_reviews.json");
        let action_reviews = Self::load_action_reviews(&action_reviews_file, settings.as_ref());
        let removed_bundled_actions_file = data_dir.join("removed_bundled_actions.json");
        let removed_bundled_actions =
            Self::load_removed_bundled_actions(&removed_bundled_actions_file, settings.as_ref());

        let snapshot_dir = data_dir.join(&config.snapshot_dir);
        std::fs::create_dir_all(&snapshot_dir)?;

        let sandbox = ActionSandbox::new(&config)?;
        let transactions = TransactionManager::new(snapshot_dir);

        let runtime = Self {
            config,
            sandbox,
            transactions: tokio::sync::Mutex::new(transactions),
            actions: tokio::sync::RwLock::new(HashMap::new()),
            disabled_actions: tokio::sync::RwLock::new(disabled_actions),
            disabled_actions_file,
            action_reviews: tokio::sync::RwLock::new(action_reviews),
            action_reviews_file,
            capability_run_contexts: tokio::sync::RwLock::new(HashMap::new()),
            removed_bundled_actions: tokio::sync::RwLock::new(removed_bundled_actions),
            removed_bundled_actions_file,
            actions_dir: actions_dir.clone(),
            cli_skills_dir,
            config_dir: config_dir.to_path_buf(),
            task_queue: None,
            action_guard: None,
            auto_approved_actions: std::sync::RwLock::new(HashSet::new()),
            tool_args_guard_config: std::sync::RwLock::new(Default::default()),
            storage: None,
            embedding_client: None,
            current_user_id: None,
            mcp_registry: None,
            plugin_registry: None,
            extension_pack_registry: None,
            #[cfg(feature = "docker")]
            active_sandbox_containers: tokio::sync::RwLock::new(HashSet::new()),
            #[cfg(feature = "docker")]
            container_reaper_status: tokio::sync::RwLock::new(ContainerReaperStatus::default()),
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

    /// Update the effective action-name overrides that can skip approval prompts.
    pub fn set_auto_approved_actions(&self, actions: &[String]) {
        let approved = crate::core::config::sanitize_auto_approve_actions(actions)
            .into_iter()
            .collect::<HashSet<_>>();
        if let Ok(mut set) = self.auto_approved_actions.write() {
            *set = approved;
        }
    }

    pub fn set_tool_args_guard_config(
        &self,
        config: crate::security::tool_args_guard::ToolArgsGuardConfig,
    ) {
        if let Ok(mut current) = self.tool_args_guard_config.write() {
            *current = config;
        }
    }

    fn tool_args_guard_config(&self) -> crate::security::tool_args_guard::ToolArgsGuardConfig {
        self.tool_args_guard_config
            .read()
            .map(|config| config.clone())
            .unwrap_or_default()
    }

    /// Set shared storage reference for expense/entity operations (called from Agent::init)
    pub fn set_storage(&mut self, storage: crate::storage::Storage) {
        self.storage = Some(storage);
    }

    pub fn set_embedding_client(
        &mut self,
        embedding_client: Option<std::sync::Arc<crate::core::EmbeddingClient>>,
    ) {
        self.embedding_client = embedding_client;
    }

    pub fn storage(&self) -> Option<crate::storage::Storage> {
        self.storage.clone()
    }

    /// Set the active user identifier (DID). Called from `Agent::init` after
    /// the identity is loaded so per-user actions (e.g. ArkOrbit) can resolve
    /// scope without it being threaded through every tool argument.
    pub fn set_current_user_id(&mut self, user_id: impl Into<String>) {
        let value = user_id.into();
        self.current_user_id = if value.trim().is_empty() {
            None
        } else {
            Some(value)
        };
    }

    fn current_user_id(&self) -> Result<&str> {
        self.current_user_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Active user identity is not configured for the runtime")
        })
    }

    fn arkorbit_service(&self) -> Result<crate::core::arkorbit::ArkOrbitService> {
        let storage = self
            .storage
            .clone()
            .ok_or_else(|| anyhow::anyhow!("ArkOrbit requires storage to be configured"))?;
        Ok(crate::core::arkorbit::ArkOrbitService::with_filesystem(
            storage,
            self.data_dir(),
        ))
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

    /// Set extension-pack registry (called from Agent::init)
    pub fn set_extension_pack_registry(
        &mut self,
        registry: std::sync::Arc<
            tokio::sync::RwLock<crate::extension_packs::ExtensionPackRegistry>,
        >,
    ) {
        self.extension_pack_registry = Some(registry);
    }

    fn action_is_auto_approved(&self, action_name: &str) -> bool {
        self.auto_approved_actions
            .read()
            .map(|set| set.contains(action_name))
            .unwrap_or(false)
    }

    async fn unapproved_permissions_for_action(
        &self,
        action: &ActionDef,
        auth_context: &ActionAuthorizationContext,
    ) -> Vec<crate::security::action_guard::Permission> {
        let action_name = action.name.as_str();
        if self.action_is_auto_approved(action_name) {
            return Vec::new();
        }
        if auth_context.direct_user_intent
            && auth_context
                .principal
                .as_ref()
                .is_some_and(|principal| principal.trusted)
            && (matches!(
                auth_context.surface,
                ActionExecutionSurface::Chat | ActionExecutionSurface::Api
            ) || Self::is_background_surface(&auth_context.surface))
        {
            return Vec::new();
        }
        let Some(guard) = self.action_guard.as_ref() else {
            return Vec::new();
        };
        let mut requested = Self::builtin_dangerous_permissions(action);
        requested.extend(
            action
                .authorization
                .access
                .permission_ids
                .iter()
                .map(|permission| {
                    crate::security::action_guard::Permission::Custom(
                        permission.trim().to_ascii_lowercase(),
                    )
                }),
        );
        if !action.authorization.access.channel_targets.is_empty() {
            requested.push(crate::security::action_guard::Permission::Custom(
                "messaging_send".to_string(),
            ));
        }
        if Self::action_demands_broad_network_consent(action) {
            requested.push(crate::security::action_guard::Permission::Custom(
                "broad_network".to_string(),
            ));
        }
        requested.sort_by_key(|permission| permission.to_string());
        requested.dedup_by(|left, right| left.to_string() == right.to_string());
        if let Some(scope) = auth_context.agent_access_scope.as_ref() {
            requested.retain(|permission| {
                !scope
                    .approved_permission_ids
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(permission.to_string().as_str()))
            });
        }
        if requested.is_empty() {
            return Vec::new();
        }
        guard.check_permissions(action_name, &requested).await
    }

    fn build_permission_requirement_error(
        action_name: &str,
        permissions: &[crate::security::action_guard::Permission],
    ) -> String {
        let perm_names = permissions
            .iter()
            .map(|perm| perm.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let guidance = if crate::core::config::AUTO_APPROVE_BLOCKED.contains(&action_name) {
            "This action is always approval-gated."
        } else {
            "If this action is part of your trusted workflow, add its name to Settings > Advanced > Auto-Approve Skills."
        };
        format!(
            "Action '{}' requires approval before execution because it needs unapproved permissions: {}. {}",
            action_name, perm_names, guidance
        )
    }

    /// Load all actions (builtin + user). Call AFTER set_action_guard.
    pub async fn load_all_actions(&self) -> Result<()> {
        // Load built-in actions
        self.load_builtin_actions().await?;

        // Load user-added skills from data dir
        let has_actions_dir = tokio::fs::metadata(&self.actions_dir)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if has_actions_dir {
            tracing::info!("Loading user skills from {:?}", self.actions_dir);
            self.load_markdown_actions(&self.actions_dir, ActionSource::Custom)
                .await?;
        }

        let has_cli_skills_dir = tokio::fs::metadata(&self.cli_skills_dir)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if has_cli_skills_dir {
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
            description: "Read the full text contents of a single file from the workspace and return them. Use when an action depends on what is already inside a file the user has authored, downloaded, or generated previously: source code, notes, JSON state, generated artifacts. The path must resolve inside the configured workspace and data directories; absolute paths outside those roots are rejected. Credential-looking files such as runtime env files, .env files, private keys, and credential JSON files are refused; use the secure credential store instead. Returns the file's UTF-8 text body.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read" }
                },
                "required": ["path"]
            }),
            capabilities: vec!["capability_inventory".to_string(), "file_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_write".to_string(),
            description: "Author or overwrite a single file in the workspace with the provided text content. Suitable for tangible authored artifacts that the user wants persisted on disk: HTML pages, JSON or YAML configuration, CSV/TSV data tables, Markdown notes, analytical models, source code modules, and generated reports. The path must resolve inside the configured workspace and data directories; both the path and the full content body are required for any useful write. Parent directories are created if they do not already exist. For generated multi-file apps, write each app file individually under one workspace directory, then call app_deploy with source_dir and source_paths so the hosted app is assembled from those staged files.".to_string(),
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
            authorization: Default::default(),
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
                    "include_semantic": { "type": "boolean", "description": "Include learned semantic facts and constraints from durable memory (default: true)" },
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
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "document_lookup".to_string(),
            description: "Search indexed documents and uploaded attachments on demand. Use when a question depends on document contents beyond the small excerpts already visible in the prompt, or when the user references uploaded files, attachments, or explicit doc ids like `doc:<id>`.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The question or search query to run against indexed documents" },
                    "limit": { "type": "integer", "description": "Maximum number of excerpts to return (default: 6)" },
                    "doc_ids": {
                        "type": "array",
                        "description": "Optional document ids to prioritize, for example [\"abcd1234\", \"efgh5678\"]",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            capabilities: vec!["documents".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "agentark_capability_lookup".to_string(),
            description: "Search the live AgentArk capability registry with curated AgentArk manual context. Use when the user asks what AgentArk can do, how a feature works, where it is configured, or whether a built-in/plugin/MCP capability exists. The live registry is authoritative; manual text is supplemental explanation. This is read-only; current run logs and object state still require state-inspection actions.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Question or topic to search in the AgentArk capability registry and manual" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 8, "description": "Maximum registry entries and supplemental manual entries to return per source (default: 4)" },
                    "doc_ids": {
                        "type": "array",
                        "description": "Optional AgentArk knowledge document IDs that scope supplemental manual retrieval.",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            capabilities: vec![
                "agentark_capabilities".to_string(),
                "agentark_manual".to_string(),
                "capability_inventory".to_string(),
                "documentation".to_string(),
                "database_readonly".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "session_search".to_string(),
            description: "Search prior conversations, persisted messages, and execution traces in AgentArk's existing history.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query or topic. Leave empty to return recent sessions." },
                    "scope": {
                        "type": "string",
                        "enum": ["all", "conversations", "messages", "traces"],
                        "description": "History area to search"
                    },
                    "conversation_id": { "type": "string", "description": "Optional conversation id to inspect directly" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 25, "description": "Maximum results to return" }
                }
            }),
            capabilities: vec!["session_history".to_string(), "database_readonly".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "vision_ocr".to_string(),
            description: "Analyze an uploaded image/PDF or image/PDF URL. Use for OCR, screenshot understanding, visual document extraction, and image questions in chat or tool flows.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "upload_id": { "type": "string", "description": "Optional uploaded image or PDF id from AgentArk uploads" },
                    "image_url": { "type": "string", "description": "Optional public image or PDF URL" },
                    "file_url": { "type": "string", "description": "Optional public image or PDF URL alias" },
                    "task": {
                        "type": "string",
                        "enum": ["extract_text", "describe", "answer_question", "analyze_document"],
                        "description": "Vision task"
                    },
                    "question": { "type": "string", "description": "Question for answer_question or extra analysis instructions" },
                    "provider": {
                        "type": "string",
                        "enum": ["openai", "google_gemini"],
                        "description": "Optional provider override"
                    },
                    "model": { "type": "string", "description": "Optional provider model override" },
                    "detail": {
                        "type": "string",
                        "enum": ["auto", "low", "high"],
                        "description": "OpenAI image detail level"
                    }
                }
            }),
            capabilities: vec!["vision_ocr".to_string(), "network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        // HTTP requests
        self.register_builtin_action(ActionDef {
            name: "http_get".to_string(),
            description: "Perform an HTTP GET request against a publicly reachable URL and return the response body. Suitable for fetching small JSON, HTML, or text resources from the open web for inspection, summarization, or follow-up reasoning. Returns the response body alongside the status code. Prefer a dedicated integration action when the user has connected the relevant service rather than treating an authenticated endpoint as plain HTTP.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "headers": { "type": "object", "description": "Optional headers" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string(), "search".to_string()],
            sandbox_mode: Some(SandboxMode::Wasm),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "lan_discover".to_string(),
            description: "Discover devices and host-local apps on the user's own LAN through a dedicated local-network discovery path. Use for requests like finding Sonos, lights, local devices, or localhost apps. In Docker installs this prefers the authenticated host LAN helper and falls back to degraded container-visible discovery. Discovery is read-only inventory; ask before any device-control action.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Optional discovery target such as sonos, lights, localhost_apps, apps, devices, or all."
                    },
                    "cidr": {
                        "type": "string",
                        "description": "Optional private IPv4 CIDR scope such as 192.168.1.0/24. Public and broad ranges are rejected."
                    },
                    "max_hosts": {
                        "type": "integer",
                        "description": "Maximum bounded host scope for any CIDR hint. Default 64, hard cap 512."
                    },
                    "include_host_local": {
                        "type": "boolean",
                        "description": "Whether to include host-local app probes. Default true."
                    },
                    "include_http_metadata": {
                        "type": "boolean",
                        "description": "Whether to run light HTTP metadata probes for discovered candidates. Default true."
                    }
                }
            }),
            capabilities: vec![
                "local_network_discovery".to_string(),
                "network".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                human_approval: crate::actions::ActionHumanApproval { required: true },
                ..Default::default()
            },
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_restart".to_string(),
            description: "Restart an existing deployed app from its saved metadata. Use after file_write edits to /app/data/apps/<id>/..., when a deployed app needs reload, or when the user asks to restart or re-run an existing app. Prefer app_id from ark_inspect app-registry results when available; otherwise use query to match an app.".to_string(),
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
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
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
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
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
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        })
        .await;

        // Shell commands (requires approval by default)
        self.register_builtin_action(ActionDef {
            name: "shell".to_string(),
            description: "Run a single shell command on the host machine and return its combined stdout/stderr and exit status. Suitable for read-only diagnostics such as listing files, inspecting process state, querying versions, and for small scripted manipulations that do not have a dedicated action. The command is forwarded verbatim to the configured shell; quoting and escaping are the caller's responsibility. By default this action requires approval, since arbitrary shell access can mutate host state.".to_string(),
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
            authorization: Default::default(),
        })
        .await;

        // Clipboard
        self.register_builtin_action(ActionDef {
            name: "clipboard_read".to_string(),
            description: "Read the current text on the host system clipboard and return it as a UTF-8 string. Useful when the user has just copied something such as a snippet, a URL, or a structured payload, and wants the assistant to operate on that exact content without retyping it. Returns the clipboard's text contents, or an empty string when the clipboard does not currently hold text.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["clipboard_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "clipboard_write".to_string(),
            description: "Replace the host system clipboard's text contents with the provided string so the user can paste it elsewhere immediately. Useful when the assistant has produced a snippet, command, address, or structured value the user wants to use outside the conversation. The full text body is required; existing clipboard content is overwritten.".to_string(),
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
            authorization: Default::default(),
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
        authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "notify_user".to_string(),
            description: format!(
                "Return a notification message for internal reminder/scheduler delivery. Use for reminders and nudges that should be delivered through {}'s delivery routing instead of an external data source.",
                crate::branding::PRODUCT_NAME
            ),
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
                    },
                    "delivery_channel": {
                        "type": "string",
                        "description": "Optional delivery route for direct chat/API notification requests. Use preferred for the runtime fallback chain, in_app for local-only delivery, or a requested channel such as telegram or whatsapp. Scheduled tasks should use schedule_task.report_to instead."
                    }
                },
                "required": ["message"]
            }),
            capabilities: vec!["notify".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        })
        .await;

        // Scheduler
        self.register_builtin_action(ActionDef {
            name: "schedule_task".to_string(),
            description: "Schedule or update durable recurring/one-time AgentArk task records whose execution is intentionally deferred until specified times or recurrences. The result is asynchronous task record(s) that run later and report through the selected delivery route; it is not a substitute for an immediate action that returns the requested result during the current turn. Do not use this for cadence that belongs inside a generated app, dashboard, page, or tool, such as its own refresh, polling, auto-update, or live-data display behavior; keep that behavior in the app/deploy artifact unless the user wants AgentArk to run or notify independently outside the artifact. Create the task directly from the task body, cadence, selected action, validation policy, and reporting route. When the user asks for multiple independent future notifications, reminders, appointments, or scheduled outcomes in one request, use `items` with one item per outcome instead of collapsing them into one task. Use `task_id` when changing an existing task from `list_tasks`; otherwise matching tasks are updated/reused unless allow_duplicate=true. Use cron for recurring schedules with minute granularity or at for one-time ISO timestamps. The cron/at value is the run or notification time; for reminders before an event, schedule the notification at the lead-time offset before the event and keep the event details in task/action_arguments. A recurring cron has no expiry unless the user gives an end policy. If the exact schedule needed to honor the user's requested reminder cannot be inferred, ask for the missing timing detail instead of creating a guess. Use `watch` instead when notification should happen only after a condition, material change, or trigger match.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                      "task": { "type": "string", "description": "Task description - what to do" },
                      "task_id": { "type": "string", "description": "Optional existing task ID to update. Use this after `list_tasks` or when the user explicitly references an existing routine/task." },
                      "cron": { "type": "string", "description": "Cron expression for recurring tasks. Minute granularity only for schedule_task. Format: 'minute hour day month weekday'. This is the time AgentArk runs or notifies, not necessarily the event start time. For advance reminders, schedule the reminder at the offset time before the event. Recurring cron schedules continue until the user cancels or changes them unless the task itself encodes a different policy. Examples: '0 9 * * *' = daily at 9am, '45 8 * * 1' = every Monday at 8:45am, '*/30 * * * *' = every 30 minutes" },
                      "at": { "type": "string", "description": "ISO 8601 timestamp for one-time task. This is the time AgentArk runs or notifies. For advance reminders, use the offset timestamp before the event. Example: '2026-02-06T09:00:00+05:30'" },
                      "items": {
                          "type": "array",
                          "description": "Batch of independent scheduled outcomes. Use one item for each distinct future task/reminder/appointment requested in the same turn. Top-level report_to, action, action_arguments, validation, automation_policy, max_attempts, stall_timeout_secs, retry_backoff_secs, and allow_duplicate are inherited by items unless overridden.",
                          "items": {
                              "type": "object",
                              "properties": {
                                  "task": { "type": "string", "description": "Task description for this scheduled outcome" },
                                  "task_id": { "type": "string", "description": "Optional existing task ID to update for this item" },
                                  "cron": { "type": "string", "description": "Cron expression for this recurring task, at the run/notification time" },
                                  "at": { "type": "string", "description": "ISO 8601 timestamp for this one-time task, at the run/notification time" },
                                  "action": { "type": "string", "description": "Optional explicit action name for this item" },
                                  "action_arguments": { "type": "object", "description": "Optional explicit arguments for this item's action" },
                                  "report_to": { "type": "string", "description": "Optional notification route override for this item" },
                                  "allow_duplicate": { "type": "boolean", "description": "Create this item separately even if a matching task already exists" },
                                  "validation": { "type": "object" },
                                  "max_attempts": { "type": "integer" },
                                  "stall_timeout_secs": { "type": "integer" },
                                  "retry_backoff_secs": { "type": "integer" },
                                  "automation_policy": { "type": "object" }
                              },
                              "oneOf": [
                                  { "required": ["task", "cron"] },
                                  { "required": ["task", "at"] },
                                  { "required": ["task_id", "cron"] },
                                  { "required": ["task_id", "at"] }
                              ]
                          }
                      },
                      "action": { "type": "string", "description": "Optional explicit action name to run for each task occurrence" },
                      "action_arguments": { "type": "object", "description": "Optional explicit arguments for the selected action" },
                    "report_to": { "type": "string", "description": "Notification route for results. Use 'preferred' unless the user explicitly requests a connected delivery channel; do not guess Telegram, WhatsApp, or another messenger." },
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
                  "oneOf": [
                      { "required": ["task", "cron"] },
                      { "required": ["task", "at"] },
                      { "required": ["task_id", "cron"] },
                      { "required": ["task_id", "at"] },
                      { "required": ["items"] }
                  ]
              }),
            capabilities: vec!["scheduler".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                channel_targets: vec![channel_target("report_to", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // Background watcher - poll an action until a condition is met, then act
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
            capabilities: vec!["network".to_string(), "search".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;
        self.register_builtin_action(ActionDef {
            name: "watch".to_string(),
            description: "Spawn or update durable background watcher(s) that poll an action at regular intervals until structured conditions are met, then execute follow-up instructions. Use this when the requested outcome is conditional monitoring, trigger-on-change detection, sub-minute polling, or a long-running watch that should notify later. Do not use this for polling or refresh cadence that belongs inside a generated app, dashboard, page, or tool's own UI/data flow; implement that in the artifact unless the user wants AgentArk to monitor or notify independently outside the artifact. Use schedule_task instead when the trigger is purely a known date/time or recurrence and no external condition needs polling. Create watcher records directly from the target, poll action, condition, cadence, timeout, and notification policy; do not run read/data-source actions first just to establish a baseline unless a required watcher argument cannot be inferred. When the user asks for multiple independent watches in one request, use `items` with one item per watcher so item-specific targets, conditions, timeouts, cadences, and notification routes are preserved. Use `watcher_id` when changing an existing watcher from `list_watchers`; otherwise matching watchers are updated/reused unless allow_duplicate=true. The watcher runs autonomously and notifies the user when triggered or timed out. Default duration is 24 hours; use until_stopped=true for watches with no expiry or when the user says to keep watching until told otherwise. If the poll target, condition, cadence, or required delivery route is too vague to infer, ask for that missing item-specific detail instead of creating a guess.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                      "description": { "type": "string", "description": "What this watcher does (shown in UI)" },
                      "watcher_id": { "type": "string", "description": "Optional existing watcher ID to update. Use this after `list_watchers` or when the user explicitly references an existing watcher." },
                      "poll_action": { "type": "string", "description": "Action to poll (e.g. 'gmail_scan', 'web_search', 'http_get')" },
                      "poll_arguments": { "type": "object", "description": "Arguments for the poll action" },
                      "items": {
                          "type": "array",
                          "description": "Batch of independent watcher outcomes. Use one item for each distinct watch requested in the same turn. Top-level poll_action, poll_arguments, condition, interval_secs, timeout fields, notify_channel, on_trigger, validation, automation_policy, max_attempts, stall_timeout_secs, retry_backoff_secs, and allow_duplicate are inherited by items unless overridden.",
                          "items": {
                              "type": "object",
                              "properties": {
                                  "description": { "type": "string" },
                                  "watcher_id": { "type": "string" },
                                  "poll_action": { "type": "string" },
                                  "poll_arguments": { "type": "object" },
                                  "condition": { "type": "object" },
                                  "on_trigger": { "type": "string" },
                                  "interval_secs": { "type": "integer" },
                                  "timeout_secs": { "type": "integer" },
                                  "timeout_hours": { "type": "integer" },
                                  "timeout_days": { "type": "integer" },
                                  "until_stopped": { "type": "boolean" },
                                  "notify_channel": { "type": "string" },
                                  "allow_duplicate": { "type": "boolean" },
                                  "validation": { "type": "object" },
                                  "max_attempts": { "type": "integer" },
                                  "stall_timeout_secs": { "type": "integer" },
                                  "retry_backoff_secs": { "type": "integer" },
                                  "automation_policy": { "type": "object" }
                              },
                              "oneOf": [
                                  { "required": ["description", "poll_action", "condition", "on_trigger"] },
                                  { "required": ["watcher_id"] }
                              ]
                          }
                      },
                      "condition": {
                        "type": "object",
                        "description": "Structured trigger condition authored by the model. Include a human-readable `description` and an explicit matcher. Prefer `json_predicate` or `json_logic` for structured poll outputs; use `llm` only when the trigger cannot be expressed safely as a deterministic contract.",
                        "properties": {
                            "description": { "type": "string", "description": "Human-readable summary of what counts as a match. For change-detection watchers, state the material difference to compare against the previous successful poll." },
                            "type": { "type": "string", "enum": ["not_empty", "text_contains", "regex", "json_predicate", "json_logic", "llm"] },
                            "text": { "type": "string", "description": "Used by `text_contains`" },
                            "case_sensitive": { "type": "boolean", "description": "Optional flag for `text_contains`" },
                            "pattern": { "type": "string", "description": "Used by `regex`" },
                            "path": { "type": "string", "description": "Dot-path into the structured poll result. Use `$` or empty for the root object." },
                            "operator": { "type": "string", "enum": ["exists", "not_exists", "eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "non_empty", "empty", "true", "false", "regex"] },
                            "value": { "description": "Comparison value for operators that require one" },
                            "logic": { "type": "string", "enum": ["all", "any"], "description": "Used by `json_logic` to combine rules" },
                            "rules": {
                                "type": "array",
                                "description": "Used by `json_logic`",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "operator": { "type": "string", "enum": ["exists", "not_exists", "eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "non_empty", "empty", "true", "false", "regex"] },
                                        "value": {}
                                    },
                                    "required": ["path", "operator"]
                                }
                            }
                        },
                        "required": ["description", "type"]
                    },
                    "on_trigger": { "type": "string", "description": "What to do when condition is met - natural language instructions for the agent" },
                    "interval_secs": { "type": "integer", "description": "Seconds between polls, including sub-minute monitoring intervals (default: 60)" },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to watch before giving up (default: 86400 = 24 hours)" },
                    "timeout_hours": { "type": "integer", "description": "Convenience timeout override in hours. Supports very large values." },
                    "timeout_days": { "type": "integer", "description": "Convenience timeout override in days. Supports very large values." },
                    "until_stopped": { "type": "boolean", "description": "Keep watching until the user stops it. Internally stored as a very large timeout." },
                    "notify_channel": { "type": "string", "description": "Notification route. Use 'preferred' by default so AgentArk can use any connected messaging channel; use a named channel only when the user explicitly requested it and it is connected. Use 'in_app' for web-only notifications." },
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
                  "oneOf": [
                      { "required": ["description", "poll_action", "condition", "on_trigger"] },
                      { "required": ["watcher_id"] },
                      { "required": ["items"] }
                  ]
              }),
            capabilities: vec!["watcher".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["watcher".to_string()],
                channel_targets: vec![channel_target("notify_channel", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "delegate".to_string(),
            description: "Coordinate a request across multiple specialized agent workstreams and synthesize one final answer. Use for work whose desired outcome benefits from independent research, implementation analysis, validation, risk review, or other parallel specialist perspectives before consolidation.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "The complete user request to delegate, including all required sub-questions, constraints, and desired final output." },
                    "context": { "type": "string", "description": "Optional conversation or business context that delegated agents should use." },
                    "final_output": { "type": "string", "description": "Optional shape of the consolidated final answer, such as operator-ready plan, recommendation, launch plan, or risk report." }
                },
                "required": ["task"]
            }),
            capabilities: vec![
                "swarm".to_string(),
                "delegate".to_string(),
                "multi_agent".to_string(),
                "agent_orchestration".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["swarm".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "capability_acquire".to_string(),
            description: "Scaffold a reusable capability when the needed capability does not already exist. HTTP/API capabilities are saved as custom API integrations so they appear in Settings > Integrations and register generated API actions. Do not create user skills for API integrations; use skill import/create only when the user is explicitly working with a skill source. Do not use this for extension-pack integrations or connector installs; use the extension_pack_* actions for those.".to_string(),
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
                    "force": { "type": "boolean", "description": "Request installation after non-blocking warnings. Blocking security findings still prevent loading." },
                    "allow_duplicate": { "type": "boolean", "description": "Create another matching capability scaffold instead of updating/reusing an existing one. Default false." }
                },
                "required": ["name", "description"]
            }),
            capabilities: vec!["integration_builder".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["capability_acquire".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "capability_resolve".to_string(),
            description: "Inspect a user goal, attached files, and prior tool failures to choose the safest next capability path. Use before giving up when a request needs missing packages, binaries, codecs, file-type detection, media conversion/transcription, app/repo repair, connector scaffolding, downloads, or another acquired capability. Returns structured JSON with detected inputs, missing capabilities, a sandbox-first acquisition route, optional approval metadata, and suggested next tool calls; it does not run host installers itself.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": { "type": "string", "description": "The user's actual goal or the blocked subgoal to resolve." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Upload IDs returned by /api/upload. The resolver validates them and sniffs file bytes rather than trusting filename/content type." },
                    "file_payloads": {
                        "type": "array",
                        "description": "Inline file payloads for executor/control-plane callers.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "filename": { "type": "string" },
                                "content_type": { "type": "string" },
                                "bytes_b64": { "type": "string" }
                            },
                            "required": ["filename", "bytes_b64"]
                        }
                    },
                    "failure_output": { "type": "string", "description": "Raw stderr/stdout or tool output from a failed attempt. Use this to detect missing binaries/packages and choose the next route." },
                    "selected_action": { "type": "string", "description": "Optional exact action name already selected from the action catalog. This is a catalog signal, not a natural-language intent label." },
                    "requested_capability": { "type": "string", "description": "Optional opaque capability label from the model/action selector or a concrete missing binary/package name. The resolver records this as context but does not classify natural-language intent from it." }
                },
                "required": ["goal"]
            }),
            capabilities: vec![
                "file_read".to_string(),
                "capability_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
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
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["browser_auto".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
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
        authorization: Default::default(),
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
        authorization: Default::default(),
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
        authorization: Default::default(),
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
            authorization: integration_authorization("gmail"),
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
                    "thread_id": { "type": "string", "description": "Gmail thread ID to reply to (from gmail_scan results)" },
                    "html_body": { "type": "string", "description": "Optional HTML body for multipart email delivery" },
                    "from": { "type": "string", "description": "Optional sender mailbox address. Defaults to the connected Gmail profile." },
                    "delivery_source": { "type": "string", "enum": ["auto", "gmail", "google_workspace"], "description": "Choose which connected Gmail backend to send through. Leave as auto unless a specific backend is required." }
                },
                "required": ["to", "subject", "body"]
            }),
            capabilities: vec!["gmail".to_string(), "google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: integration_authorization("gmail"),
        }).await;

        // Web search
        self.register_builtin_action(ActionDef {
            name: "web_search".to_string(),
            description: "Search the web for external information needed in the current answer or as a required input to another action. Use the semantic temporal scope to distinguish current/recent information from historical or timeless lookup. Do not use this as a prerequisite baseline for durable scheduled work or watchers when the durable object can perform its own later poll.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query. Preserve the user's topic and any explicit date or range. For current/recent scope, include the runtime date or year when it improves freshness; for historical scope, preserve the historical period." },
                    "num_results": { "type": "integer", "description": "Number of results (default 5)" },
                    "backend": { "type": "string", "description": "Search backend override: serper, brave, brave_api, exa, tavily, perplexity, firecrawl, searxng, playwright, lightpanda, duckduckgo, bing_rss" },
                    "time_scope": { "type": "string", "enum": ["current", "recent", "historical", "timeless"], "description": "Semantic temporal intent of the lookup. Use current/recent when the answer depends on now, latest state, news, or recent changes; historical when the user gives or implies a past period; timeless for stable background/reference lookup." }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Research
        self.register_builtin_action(ActionDef {
            name: "research".to_string(),
            description: "Conduct deep research on a topic by gathering diverse source sets, fetching and comparing evidence, surfacing contradictions and open questions, and returning a citation-backed synthesis. Use for complex current-answer questions that need thorough investigation beyond a simple web search. Do not use this as a prerequisite baseline for durable scheduled work or watchers when the durable object can perform its own later poll.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Research topic or question. For current or recent questions, anchor the query to the runtime date/current year. For explicit historical periods, preserve the user's date or range instead of making it current." },
                    "max_sources": { "type": "integer", "description": "Maximum sources to examine (default 5, or 12 when depth='deep')" },
                    "backend": { "type": "string", "description": "Optional search backend override: serper, brave, brave_api, exa, tavily, perplexity, firecrawl, searxng, playwright, lightpanda, duckduckgo, bing_rss" },
                    "depth": { "type": "string", "description": "Research depth: quick, standard, deep" },
                    "include_sources": { "type": "boolean", "description": "Include source URLs" },
                    "min_primary_sources": { "type": "integer", "description": "Minimum number of primary-source-like results to include when available. Deep research defaults to 2." },
                    "freshness_window_days": { "type": "integer", "description": "Optional freshness window in days for preferring dated, recent evidence." },
                    "followup_rounds": { "type": "integer", "description": "Extra follow-up search rounds to close evidence gaps, fetch primary sources, and investigate contradictions. Deep research defaults to 2." }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Code execution sandbox
        self.register_builtin_action(ActionDef {
            name: "code_execute".to_string(),
            description: "Execute supplied source code in an isolated Docker sandbox for computational work, runtime validation, dependency bootstrap, scripted checks, and executable notebooks. The sandbox returns stdout, stderr, exit status, generated output files, and execution metadata; it is transient and should not be treated as a durable authoring or deployment surface when a direct filesystem, app-hosting, workspace, or orchestration action can produce the requested persistent result. Uploaded files can be mounted at /data/<filename>; network egress remains disabled unless execution metadata requires live target connectivity.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": {
                        "type": "string",
                        "enum": ["python", "javascript", "typescript", "bash", "ruby", "php", "perl", "lua", "r", "java", "c", "cpp", "go", "rust", "swift", "kotlin", "jupyter"],
                        "description": "Programming language. Use 'jupyter' for EDA/ML notebooks with visualizations."
                    },
                    "code": { "type": "string", "description": "Code to execute. For jupyter: provide valid .ipynb JSON content (notebook format). For other languages: plain code. Can include dependency installation. When files are provided, access them at /data/<filename>." },
                    "network_access": { "type": "boolean", "description": "Whether this sandbox execution may use outbound network access. Default: false. Leave disabled unless the code genuinely needs egress." },
                    "timeout_secs": { "type": "integer", "description": "Optional execution timeout in seconds. Defaults are chosen by runtime: 60s for scripts, 120s for compiled builds, 600s for notebooks, and longer for dependency bootstrap. Max 600s for scripts/builds and 900s for notebooks." },
                    "execution_contract": {
                        "type": "object",
                        "description": "Optional structured execution contract for multi-step automations. Use exact phase values `bootstrap`, `validate`, or `poll`. For validation/polling steps that prove the monitor is ready, set `target_validated_when_successful=true` and `ready_for_watch_when_successful=true` so AgentArk can chain follow-up actions without guessing from source text.",
                        "properties": {
                            "phase": { "type": "string", "enum": ["bootstrap", "validate", "poll"] },
                            "target_validated_when_successful": { "type": "boolean" },
                            "ready_for_watch_when_successful": { "type": "boolean" },
                            "target_connectivity_required": { "type": "boolean", "description": "Set true when this step must reach a live target such as a URL, LAN device, network stream, API, or other device endpoint. AgentArk will enable sandbox network access for the step." }
                        }
                    },
                    "env": { "type": "object", "description": "Optional environment variables (values may include {{secret:...}} / {{env:...}} placeholders).", "additionalProperties": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Upload IDs returned by /api/upload for user-attached files. Each file is validated before being injected into the sandbox at /data/<filename>." }
                },
                "required": ["language", "code"]
            }),
            capabilities: vec!["code_execute".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // List tasks/goals/routines
        self.register_builtin_action(ActionDef {
            name: "list_tasks".to_string(),
            description: "List pending tasks, goals, routines, and scheduled items, including IDs that can be passed back to schedule_task.task_id for updates. Use when the user asks about their pending goals, tasks, agenda, or what's scheduled.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Filter: 'all', 'pending', 'goals', 'routines', 'completed', 'failed'. Default: 'pending'" }
                }
            }),
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "list_watchers".to_string(),
            description: "List background watchers and their live status, IDs, poll counts, conditions, and next poll timing. Use watcher IDs with watch.watcher_id when updating an existing watcher. Use when the user asks what the agent is watching, which watchers are active, or whether a watcher has triggered/paused/failed.".to_string(),
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
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "background_session_manage".to_string(),
            description: "Inspect or modify a durable AgentArk background session and its linked tasks/watchers. Use when the user refers to an existing background work/session and wants status, pause, resume, stop/cancel, deletion, or a delivery-channel change for that session as a whole. Resolve by background_session_id when available; otherwise provide the user's reference in reference_text so AgentArk can resolve against recent session context. This action is for AgentArk-owned background work, not app-internal refresh/poll cadence.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["status", "list", "pause", "resume", "stop", "cancel", "delete", "update_delivery"],
                        "description": "Session-level operation. stop and cancel close the session and cancel linked pending work; delete removes the session and linked work records."
                    },
                    "background_session_id": {
                        "type": "string",
                        "description": "Optional exact background session id. Omit only when the current conversation context clearly identifies one session."
                    },
                    "reference_text": {
                        "type": "string",
                        "description": "User's semantic reference to the target background work when no id is supplied."
                    },
                    "delivery_channel": {
                        "type": "string",
                        "description": "Required for update_delivery. Use preferred, in_app, telegram, whatsapp, or another configured channel only when requested."
                    },
                    "include_closed": {
                        "type": "boolean",
                        "description": "Include completed/cancelled/failed sessions when listing or resolving. Default false except status by exact id."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec![
                "background_session".to_string(),
                "scheduler".to_string(),
                "watcher".to_string(),
                "notification".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec![
                    "background_session".to_string(),
                    "watcher".to_string(),
                    "scheduler".to_string(),
                ],
                channel_targets: vec![channel_target("delivery_channel", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ark_inspect::action_def())
            .await;

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
        authorization: Default::default(),
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
        authorization: Default::default(),
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
        authorization: integration_authorization("media_gen"),
        }).await;

        // Action management - create/update/delete/list custom actions via chat
        // Home Assistant read-only state access.
        self.register_builtin_action(ActionDef {
            name: "home_assistant".to_string(),
            description: "Read Home Assistant state, services, and entities from the configured Home Assistant instance.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["list_entities", "search_entities", "get_state", "get_services"],
                        "description": "Read operation to run"
                    },
                    "entity_id": { "type": "string", "description": "Entity id for get_state, such as light.kitchen" },
                    "domain": { "type": "string", "description": "Optional entity domain filter such as light, sensor, switch, climate" },
                    "query": { "type": "string", "description": "Optional entity search query" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum entities to return" }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["home_assistant".to_string(), "local_network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: integration_authorization("home_assistant"),
        }).await;

        let mut home_assistant_control_auth = integration_authorization("home_assistant");
        home_assistant_control_auth.risk_level = ActionRiskLevel::High;
        home_assistant_control_auth.human_approval =
            crate::actions::ActionHumanApproval { required: true };
        home_assistant_control_auth.outbound.outbound_write = true;
        self.register_builtin_action(ActionDef {
            name: "home_assistant_call_service".to_string(),
            description: "Call a Home Assistant service on configured devices. Requires explicit user approval because it can change the physical environment.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "domain": { "type": "string", "description": "Home Assistant service domain, such as light, switch, climate, media_player" },
                    "service": { "type": "string", "description": "Service name in the selected domain, such as turn_on, turn_off, set_temperature" },
                    "entity_id": { "type": "string", "description": "Optional target entity id" },
                    "target": { "type": "object", "description": "Optional Home Assistant target object" },
                    "service_data": { "type": "object", "description": "Optional Home Assistant service data" }
                },
                "required": ["domain", "service"]
            }),
            capabilities: vec!["home_assistant_control".to_string(), "local_network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: home_assistant_control_auth,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "manage_actions".to_string(),
            description: "Create, update, delete, or list user-added actions/skills/workflows. Use when the user wants to inspect their installed skills, add a new action, or modify the action library.".to_string(),
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
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "list_integrations".to_string(),
            description: "Return a compact inventory of every AgentArk external surface: built-in integrations, messaging channels, notification channels, custom APIs, webhooks, companion devices, extension packs, plugins, and MCP servers. Use for overview or lightweight connected/authenticated checks; use inspect_integration for one detailed surface record.".to_string(),
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
                    },
                    "include_details": {
                        "type": "boolean",
                        "description": "Include full per-surface records. Default false; prefer inspect_integration for detail."
                    }
                }
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "inspect_integration".to_string(),
            description: "Inspect one AgentArk external surface by structured surface id and item id from list_integrations. Supports companion devices, built-in integrations, messaging/notification channels, custom APIs, webhooks, extension packs, plugins, and MCP servers. Returns detailed status without broad catalog output.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "surface": {
                        "type": "string",
                        "description": "Surface id from list_integrations, such as companion_devices, integrations, messaging_channels, notification_channels, custom_apis, webhook_sources, extension_packs, plugins, or mcp_servers."
                    },
                    "id": {
                        "type": "string",
                        "description": "Item id from list_integrations."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional generic fallback search across ids and display names when id is not known."
                    },
                    "run_check": {
                        "type": "boolean",
                        "description": "Run a safe live/readiness check when the surface supports one. Default false."
                    }
                }
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // PDF generation - creates PDF documents from content
        // Generic extension-pack control plane
        self.register_builtin_action(ActionDef {
            name: "postgres_schema_inspect".to_string(),
            description: "Inspect the live AgentArk Postgres public schema and return valid table and column names for follow-up diagnostics. Use before structured database reads or when a DB-backed internal question needs schema discovery.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table_filter": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter for table names."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum tables to return (default: 25)."
                    }
                }
            }),
            capabilities: vec!["database_readonly".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "postgres_query_readonly".to_string(),
            description: "Run a structured, read-only table query against the live AgentArk Postgres database. Supply a public table name, optional columns, filters, sorting, and limit. Do not pass raw SQL. If a table or column is rejected, inspect the schema and retry.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table": {
                        "type": "string",
                        "description": "Public AgentArk table name from postgres_schema_inspect."
                    },
                    "columns": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of columns to return. Default: all readable columns."
                    },
                    "filters": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string" },
                                "op": {
                                    "type": "string",
                                    "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "contains", "starts_with", "ends_with", "in", "is_null", "not_null"]
                                },
                                "value": {}
                            },
                            "required": ["column", "op"]
                        }
                    },
                    "order_by": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string" },
                                "direction": {
                                    "type": "string",
                                    "enum": ["asc", "desc"]
                                }
                            },
                            "required": ["column"]
                        }
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum rows to return (default: 50, max: 200)."
                    }
                },
                "required": ["table"]
            }),
            capabilities: vec!["database_readonly".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_list".to_string(),
            description: "List installed and catalog extension packs. Use when the user asks what generic integrations, messaging channels, or other packs are available.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Optional pack kind filter such as integration or messaging_channel." },
                    "query": { "type": "string", "description": "Optional search query." }
                }
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_search".to_string(),
            description: "Search installed and catalog packs, including integrations, messaging channels, and future user-added extensions. If nothing is found, use this before asking the user for a pack link or docs URL.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Pack search query." },
                    "kind": { "type": "string", "description": "Optional kind filter such as integration or messaging_channel." }
                },
                "required": ["query"]
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_install".to_string(),
            description: "Install a bundled, linked, or inline-manifest extension pack. Use for install requests that can apply to integrations, messaging channels, or other pack types.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "source_url": { "type": "string" },
                    "source_path": { "type": "string" },
                    "manifest_text": { "type": "string" },
                    "manifest": { "type": "object" },
                    "trust_unverified": { "type": "boolean" }
                }
            }),
            capabilities: vec!["integration_admin".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_scaffold".to_string(),
            description: "Scaffold a draft local extension pack from chat intent. Use when the needed integration or channel pack does not exist yet.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "type": "string" },
                    "description": { "type": "string" },
                    "docs_url": { "type": "string" },
                    "openapi_url": { "type": "string" },
                    "openapi_text": { "type": "string" },
                    "curl_text": { "type": "string" },
                    "auth_mode": { "type": "string", "enum": ["none", "api_key", "basic", "oauth2_external"] },
                    "desired_features": { "type": "array", "items": { "type": "string" } },
                    "read_only": { "type": "boolean" },
                    "binding_kind": { "type": "string" },
                    "publisher": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["name"]
            }),
            capabilities: vec!["integration_admin".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_connect".to_string(),
            description: "Create or update a pack connection. For OAuth-style packs this returns a browser connect URL when supported. For secret-based packs, omit the secret when AgentArk should collect credentials securely through the UI; do not ask users to paste raw secrets into normal chat.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "connection_id": { "type": "string" },
                    "name": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "metadata": { "type": "object" },
                    "secret": {},
                    "clear_secret": { "type": "boolean" },
                    "redirect_uri": { "type": "string", "description": "Optional explicit redirect URI for OAuth connect URL generation." }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec!["integration_admin".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "custom_messaging_channel_upsert".to_string(),
            description: "Create or update a reusable custom messaging channel for outbound AgentArk notifications using a declared HTTP send spec. Use when the user wants AgentArk to deliver messages through a non-bundled channel, webhook, internal notification service, or provider-specific messaging API. Declare credential fields and {{secret:KEY}} placeholders only; never include raw credential values in this action.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Optional stable slug. If omitted, derived from name." },
                    "name": { "type": "string", "description": "User-facing channel name." },
                    "description": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "docs_url": { "type": "string" },
                    "auth_profile_id": { "type": "string", "description": "Optional reusable auth profile id for OAuth or advanced auth handled outside direct secret fields." },
                    "credential_fields": {
                        "type": "array",
                        "description": "Credential fields to collect securely. Do not include values.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": { "type": "string" },
                                "label": { "type": "string" },
                                "placeholder": { "type": "string" },
                                "help": { "type": "string" },
                                "input_type": { "type": "string", "enum": ["password", "text", "textarea"] },
                                "required": { "type": "boolean" }
                            },
                            "required": ["key"]
                        }
                    },
                    "auth_manifest": {
                        "type": "object",
                        "description": "Optional advanced IntegrationAuthManifest for multi-field, OAuth2 code, device code, or hybrid auth. Storage targets are normalized by AgentArk."
                    },
                    "send": {
                        "type": "object",
                        "description": "HTTP send template. Supported placeholders are {{text}}, {{subject}}, {{to}}, {{conversation_id}}, and {{secret:KEY}}.",
                        "properties": {
                            "method": { "type": "string", "enum": ["post", "put", "patch", "get", "delete"] },
                            "url_template": { "type": "string" },
                            "headers": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "value_template": { "type": "string" }
                                    },
                                    "required": ["name", "value_template"]
                                }
                            },
                            "body_template": { "type": "string" },
                            "content_type": { "type": "string" },
                            "auth": {
                                "type": "object",
                                "description": "Auth transport binding. Examples: {kind:'none'}, {kind:'bearer', secret_key:'token'}, {kind:'custom_header', name:'X-Api-Key', value_template:'{{secret:api_key}}'}, {kind:'basic', username_key:'username', password_key:'password'}, {kind:'query_param', name:'key', value_template:'{{secret:api_key}}'}."
                            },
                            "expect_status": { "type": "array", "items": { "type": "integer" } }
                        },
                        "required": ["url_template"]
                    }
                },
                "required": ["name", "send"]
            }),
            capabilities: vec!["integration_admin".to_string(), "notify".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                rate_limit: Some(crate::actions::ActionRateLimit {
                    max_calls: 5,
                    window_seconds: 300,
                }),
                human_approval: crate::actions::ActionHumanApproval { required: true },
                outbound: crate::actions::ActionEgressPolicy {
                    outbound_write: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_set_enabled".to_string(),
            description: "Enable or disable an installed extension pack so its registered actions can be used by the agent.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["pack_id", "enabled"]
            }),
            capabilities: vec!["integration_admin".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        for (name, description) in [
            (
                "extension_pack_runtime_install",
                "Install or verify the local runtime declared by an installed extension pack.",
            ),
            (
                "extension_pack_runtime_verify",
                "Verify the local runtime declared by an installed extension pack and refresh its recorded status.",
            ),
            (
                "extension_pack_runtime_update",
                "Update the local runtime declared by an installed extension pack when update commands are available.",
            ),
            (
                "extension_pack_runtime_uninstall",
                "Uninstall the local runtime declared by an installed extension pack and mark the runtime as missing.",
            ),
        ] {
            self.register_builtin_action(ActionDef {
                name: name.to_string(),
                description: description.to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pack_id": { "type": "string" }
                    },
                    "required": ["pack_id"]
                }),
                capabilities: vec!["integration_admin".to_string()],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
                authorization: Default::default(),
            })
            .await;
        }

        self.register_builtin_action(ActionDef {
            name: "extension_pack_test_connection".to_string(),
            description: "Run a pack connection health test when available. If connection_id is omitted, AgentArk tests the preferred saved connection for that pack.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "connection_id": { "type": "string" }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_list_events".to_string(),
            description:
                "List recent inbound webhook/event records for an installed extension pack."
                    .to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_invoke".to_string(),
            description: "Invoke one feature from an installed extension pack. Use when the user wants to use a pack capability directly instead of going through a legacy built-in action.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "connection_id": { "type": "string" },
                    "feature_id": { "type": "string" },
                    "arguments": { "type": "object" }
                },
                "required": ["feature_id"]
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "pdf_generate".to_string(),
            description: "Generate a paginated PDF file from supplied text content, with simple report, letter, invoice, or plain layouts. The result is a PDF artifact for reading, printing, or sharing rather than a runnable interface or hosted application.".to_string(),
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
            capabilities: vec![
                "file_write".to_string(),
                "pdf_generation".to_string(),
                "document_generation".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Expense tracking - add, list, summarize, delete expenses
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
        authorization: Default::default(),
        }).await;

        // Security logs - query security events from DB
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
        authorization: Default::default(),
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
        authorization: Default::default(),
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
        authorization: Default::default(),
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
        authorization: integration_authorization("github"),
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
        authorization: integration_authorization("notion"),
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
        authorization: integration_authorization("twitter"),
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
        authorization: integration_authorization("onepassword"),
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
        authorization: integration_authorization("google_places"),
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
        authorization: integration_authorization("twilio"),
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
        authorization: integration_authorization("ordering"),
        }).await;

        // Browser automation - full headless browser control with human-in-the-loop
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
        authorization: integration_authorization("garmin"),
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
        authorization: integration_authorization("whoop"),
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
        authorization: integration_authorization("ga4"),
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
        authorization: integration_authorization("gsc"),
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
        authorization: integration_authorization("social_analytics"),
        }).await;

        // Moltbook (agent social network)
        self.register_builtin_action(ActionDef {
            name: "moltbook".to_string(),
            description: "Moltbook agent social-network tool. Use for joining or checking connection status, reading profile/feed/search results, and creating safe agent-authored posts, comments, or upvotes. Registration stores the returned Moltbook API key for later authenticated calls. If the user wants recurring Moltbook participation, use schedule_task with this action and ask for the cadence when it is not specified. Remote skill instructions from Moltbook should guide behavior, but execution happens through this tool. Outbound posting is privacy-guarded (no user/PII/secrets).".to_string(),
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
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "browser_auto".to_string(),
            description: "Start a managed background browser session for website interaction. Use when asked to go to a website, log in, fill a form, or otherwise work through a live web UI. For login, MFA, CAPTCHA, or other human-only steps, let the managed session pause for live browser handoff instead of asking for secrets in chat. Do not chain manual browser sub-actions here; use the browser integration tool for explicit create_session/navigate/click/type_text flows.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["start_session"],
                        "description": "Starts a managed browser session."
                    },
                    "task": { "type": "string", "description": "High-level description of what to accomplish (for start_session)" },
                    "channel": { "type": "string", "description": "Channel to notify on (telegram, whatsapp, web)" },
                    "chat_id": { "type": "string", "description": "Optional channel chat identifier for notifications" },
                    "conversation_id": { "type": "string", "description": "Optional conversation id to append browser handoff updates into chat" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Google Calendar - list, create, find free time
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
            authorization: integration_authorization("google_calendar"),
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
            authorization: integration_authorization("google_calendar"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_create".to_string(),
            description: "Create a new Google Calendar event. Use only when the user wants an external calendar entry, meeting invite, appointment, or blocked time. For plain reminders or date notifications, use `schedule_task` with `notify_user` instead. AgentArk schedules its own default push reminder separately unless the user says not to remind them.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "Event title" },
                    "start": { "type": "string", "description": "Start datetime (ISO 8601)" },
                    "end": { "type": "string", "description": "End datetime (ISO 8601)" },
                    "description": { "type": "string", "description": "Event description/notes" },
                    "location": { "type": "string", "description": "Event location" },
                    "attendees": { "type": "array", "items": { "type": "string" }, "description": "List of attendee email addresses" },
                    "agentark_reminder": {
                        "type": ["boolean", "object"],
                        "description": "AgentArk push reminder control. Omit for the default 15-minute push reminder. Use false only when the user explicitly opts out. Use {\"enabled\": true, \"minutes_before\": N} when the user requests a different AgentArk reminder lead time.",
                        "properties": {
                            "enabled": { "type": "boolean" },
                            "minutes_before": { "type": "integer", "minimum": 1, "maximum": 1440 }
                        }
                    }
                },
                "required": ["summary", "start", "end"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["calendar_write".to_string()],
                integration_ids: vec!["google_calendar".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
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
            authorization: integration_authorization("google_calendar"),
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
            authorization: google_workspace_bundle_authorization("drive"),
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
            authorization: google_workspace_bundle_authorization("docs"),
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
            authorization: google_workspace_bundle_authorization("sheets"),
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
            authorization: google_workspace_bundle_authorization("chat"),
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
            authorization: google_workspace_bundle_authorization("admin"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_help".to_string(),
            description: "Show Google Workspace CLI help output for the currently connected Workspace integration. Use when you need to inspect gws syntax for a granted service or discover generic commands. Pass argv as the command parts after `gws`, for example [\"gmail\",\"users\",\"messages\",\"list\",\"--help\"] or [\"drive\",\"--help\"].".to_string(),
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
            authorization: google_workspace_authorization(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_schema".to_string(),
            description: "Inspect the request and response schema for a Google Workspace CLI method. Use when you need the exact shape for a gws command within the currently granted Workspace bundles before executing it. Example target: gmail.users.messages.list or drive.files.list.".to_string(),
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
            authorization: google_workspace_authorization(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_skills".to_string(),
            description: "List or read the generated Google Workspace CLI skill docs available for the currently granted Workspace bundles. Use this when you want exact gws examples before calling google_workspace_gws_schema or google_workspace_gws_command. If name is provided, returns the full SKILL.md content for that visible skill.".to_string(),
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
            authorization: google_workspace_authorization(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_command".to_string(),
            description: "Execute a non-auth Google Workspace CLI command against the connected Google Workspace account, but only within the currently granted Workspace bundles. Use this when you need broader coverage than the built-in Gmail, Calendar, or granted bundle helpers provide. Prefer google_workspace_gws_skills for granted examples and google_workspace_gws_schema for exact method shapes before executing unfamiliar commands. Provide argv as the command parts after `gws`, for example [\"gmail\",\"users\",\"messages\",\"list\",\"--params\",\"{\\\"maxResults\\\":5,\\\"labelIds\\\":[\\\"INBOX\\\"]}\"] , [\"calendar\",\"+agenda\"], or [\"drive\",\"files\",\"list\",\"--params\",\"{\\\"pageSize\\\":5}\"] . Set required_bundles when you know which Workspace bundles this command needs, such as [\"drive\"] or [\"gmail\",\"calendar\"].".to_string(),
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
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["google_workspace_command".to_string()],
                integration_ids: vec!["google_workspace".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // SSH - remote server execution (behind feature flag)
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
                authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                    permission_ids: vec!["ssh".to_string()],
                    requires_ssh_connection: true,
                    ..crate::actions::ActionAccessMetadata::default()
                }),
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
            authorization: Default::default(),
            }).await;
        }

        // App deployment - write files, start servers, return live URL
        self.register_builtin_action(ActionDef {
            name: "app_deploy".to_string(),
            description: format!(
                "Deploy a web app or server and return a live URL. Supports generated files, files staged in the workspace, line-level patches to an existing app, explicit file deletes, OR a repository source. Use when the intended outcome is a managed browser-usable or hosted artifact, such as building a dashboard, creating a tool, making a website, building an app, or deploying/running a repo locally for the user. External publishing is explicit: deploy_target defaults to local; set deploy_target=\"vercel_direct\" only when the selected app deployment layer is Vercel direct API publishing, or deploy_target=\"vercel_git\" only when the selected layer is Git-backed Vercel. If the requested timing/cadence describes how the generated artifact refreshes, polls, auto-updates, backfills, or presents live data, implement that behavior inside the artifact rather than creating an AgentArk schedule or watcher. Build the smallest working app that satisfies the requested workflow, with polished responsive UI, clear controls, and useful loading/empty/error states. Keep generated bundles lean: avoid unrelated routes, auth, databases, admin areas, test suites, generated boilerplate, package manifests, server files, or lifecycle commands unless the user's intent semantically requires them. Prefer a standalone static/browser bundle when the requested behavior can run with browser APIs, timers, client-side state, and public same-origin/app-scoped fetch. Use a dynamic backend/runtime only for server-only needs: secret credentials, authenticated server-side API access, durable jobs that must continue with no browser open, durable server-side state/databases, filesystem/process access, webhooks, private-network access, non-HTTP protocols, or APIs that the browser/app proxy cannot safely call. {inline_report_boundary} For generated multi-file apps, prefer staging each file with `file_write` under one workspace directory, then call app_deploy with `source_dir` and `source_paths`; this gives the user per-file progress and avoids one giant deploy payload. For edits to a known existing app, prefer `mode=\"patch\"` with `app_id` and `file_patches` unified diffs for small line changes, plus `delete_paths` for removed files; use full `files` only for files that must be replaced completely. For small file-based apps, you may instead provide a `files` object containing every local file needed by the page: if HTML/CSS references a local stylesheet, script, image, font, manifest, or media asset, include that file too. The delivered app must implement the requested workflow and controls; do not substitute a placeholder, mock-only screen, or decorative shell when the user asked for working behavior. Static browser apps should omit package manifests, server files, `entry_command`, and `start_command` unless a real runtime is needed. Local asset paths must be app-relative, not root-relative. For generated static apps that read public APIs, prefer app-relative {} helpers over third-party CORS proxy services. The app-scoped `__agentark/http/fetch?url=...` helper performs same-origin public GET/HEAD requests for public hosts referenced by the deployed app source; it is not for private networks or secrets. Authenticated API apps are supported, but do not embed credentials in browser JavaScript or static files. Build a dynamic backend/proxy when an API needs secret headers/tokens, declare the needed keys in `required_inputs`/`required_secrets`, read them from process env at runtime, and use `config` only for non-sensitive values such as base URLs. AgentArk's own model/provider credentials are not inherited by generated apps; app credentials must be supplied intentionally through the secure credential store. When modifying a known deployed app, provide its stable `app_id`; otherwise a new deployment is created unless duplicate detection finds a matching app to reuse or replace. For repo-based apps, provide `repo_url` (and optionally `repo_ref`, `repo_subdir`, `service_mode`) so {} can clone the repo, inspect the README/manifests, stand up the detected frontend/backend services, and return managed endpoints. For generated file bundles, provide `entry_command` or `start_command` only when the app needs a long-lived server/runtime; a start command makes the app dynamic unless `runtime_required=false` is explicitly supplied. Generated dynamic bundles may be Python, Node/TypeScript, Rust, or another direct-command stack when the files include complete project configuration plus appropriate lifecycle commands. Dynamic app runtimes persist their app directory and lifecycle commands, can install dependencies with network access before startup (`pip`, `npm`, `cargo`, etc.), can run an optional `stop_command` as a graceful stop hook, and restart from saved metadata. Repo-based deploys default to container runtime unless overridden. Dynamic app containers default to the installed {} image unless `runtime_image` or a runner-image env override is provided; use `runtime_image` for specialized toolchains not present in the default runner. Deployment is local by default. Content visibility or audience requirements inside the app are not the same as external network exposure; set expose_public only when the deployment target itself is external/public internet exposure. Local app deployments stay local and access guard defaults to off unless the user explicitly enables local App Guard or supplies a local access password. Public exposure does not change the local URL or local guard setting; the public app surface is protected by App Guard and AgentArk generates a public access password if one is not supplied. After deployment, direct the user to the Apps page for start, stop, restart, logs, App Guard, public exposure, and delete controls. Declare required inputs via required_inputs and mark each item sensitive=true/false.",
                crate::branding::PRODUCT_NAME,
                crate::branding::PRODUCT_NAME,
                crate::branding::PRODUCT_NAME,
                inline_report_boundary =
                    crate::core::inline_artifacts::app_deploy_inline_report_boundary()
            ),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Optional stable deployed app id to update in place. Use when modifying a known existing generated app; omit when creating a new app."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "patch"],
                        "default": "replace",
                        "description": "replace creates or replaces the declared app bundle and removes stale managed files. patch requires app_id and applies only file_patches, complete changed files in files/source_paths, and delete_paths while preserving all other managed files."
                    },
                    "files": {
                        "type": "object",
                        "description": "Object mapping filename to file content. Include every locally referenced asset; use relative paths such as \"style.css\", \"app.js\", or \"assets/logo.svg\", not \"/style.css\". For generated static pages, prefer {\"index.html\":\"<html>...<link rel=\\\"stylesheet\\\" href=\\\"style.css\\\">...\", \"style.css\":\"body{...}\", \"app.js\":\"...\"} over large inline style/script blocks. Each value must be the complete file body."
                    },
                    "file_patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string", "description": "App-relative file path to patch." },
                                "patch": { "type": "string", "description": "Unified diff hunks for this file. Include context lines so the patch can be verified against the current file." }
                            },
                            "required": ["path", "patch"]
                        },
                        "description": "Line-level unified diffs to apply when mode='patch'. This lets small edits avoid re-emitting whole files."
                    },
                    "delete_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "App-relative files to remove from the existing app bundle. Use with mode='patch' for deletions; replace mode also removes stale managed files not declared in files/source_paths."
                    },
                    "source_dir": {
                        "type": "string",
                        "description": "Optional workspace/data directory already populated with app files via file_write. When supplied with source_paths, app_deploy reads those staged files instead of receiving a large files object."
                    },
                    "source_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "App-relative file paths under source_dir to include in the bundle, such as [\"index.html\", \"style.css\", \"src/App.tsx\"]. Required when using source_dir without files."
                    },
                    "repo_url": {
                        "type": "string",
                        "description": format!(
                            "Public Git repository URL to clone and deploy, e.g. https://github.com/org/repo. Use this instead of `files` when the user wants {} to run an existing repo locally.",
                            crate::branding::PRODUCT_NAME
                        )
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
                    "deploy_target": {
                        "type": "string",
                        "enum": ["local", "vercel_direct", "vercel_git"],
                        "default": "local",
                        "description": "Explicit app deployment layer. local creates the standard AgentArk /apps/{id} deployment. vercel_direct also publishes the resulting app bundle to Vercel through the REST API using the configured Vercel token. vercel_git records the Git-backed Vercel intent and returns a structured nudge when Git or Vercel project configuration is missing."
                    },
                    "production": {
                        "type": "boolean",
                        "description": "For external Vercel publishing, deploy to production when true; otherwise create a preview deployment. Production publishing should be explicit."
                    },
                    "vercel_project_mode": {
                        "type": "string",
                        "enum": ["auto", "existing", "create"],
                        "default": "auto",
                        "description": "Project handling for external Vercel publishing. auto uses the saved or generated project name with the deployment API. existing requires a saved or supplied project id/name. create calls the Vercel Projects API before deploying and then deploys into that project."
                    },
                    "vercel_project_id": {
                        "type": "string",
                        "description": "Optional Vercel project id/name for external Vercel publishing. If omitted, the saved Vercel project setting or a generated project name is used."
                    },
                    "vercel_team_id": {
                        "type": "string",
                        "description": "Optional Vercel team id used as the teamId API query parameter for external Vercel publishing."
                    },
                    "build_command": {
                        "type": "string",
                        "description": "Optional Vercel projectSettings.buildCommand for source-based deployments such as Next.js apps."
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "Optional Vercel projectSettings.outputDirectory for source-based deployments."
                    },
                    "title": { "type": "string", "description": "App name/title (default: App)" },
                    "entry_command": {
                        "type": "string",
                        "description": "Command to start the server process (omit for static HTML apps). Supplying this makes the app a persistent dynamic runtime unless runtime_required=false is explicitly set. Use {PORT} placeholder or PORT env var for the port. Python apps auto-activate their venv. Examples: 'python3 app.py', 'node server.js', 'npm run start', 'uvicorn app:app --host 0.0.0.0 --port {PORT}', 'cargo run'"
                    },
                    "start_command": {
                        "type": "string",
                        "description": "Alias for entry_command. Use when the generated app or repo naturally describes its lifecycle as start/stop commands. It is persisted and used by the Apps UI Start/Restart action."
                    },
                    "install_command": {
                        "type": "string",
                        "description": "Command to install dependencies before starting (optional). Omit for Python apps with requirements.txt - a venv is auto-created. Each app runs in its own persistent isolated environment (Python venv, local node_modules, or stack-specific build cache), and dynamic runtime dependency installs may use network access. Examples: 'pip install -r requirements.txt', 'npm install', 'cargo fetch'"
                    },
                    "stop_command": {
                        "type": "string",
                        "description": "Optional direct command to run from the app directory before the managed runtime is stopped. Used as a best-effort graceful stop hook by the Apps UI Stop action. Keep it a single direct command such as 'npm run stop'; shell operators are rejected."
                    },
                    "commands": {
                        "type": "object",
                        "description": "Optional lifecycle command block. Supported keys: install/setup, start/entry, and stop. Values are persisted in app metadata and used by the Apps UI Start/Restart/Stop actions.",
                        "additionalProperties": { "type": "string" }
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
                        "description": "Required runtime inputs. String entries default to sensitive=true. Use object entries for per-key sensitivity, e.g. [{\"key\":\"API_TOKEN\",\"sensitive\":true},{\"key\":\"BASE_URL\",\"sensitive\":false}]. For authenticated APIs, declare secret headers/tokens here and read them from process env in a dynamic backend rather than embedding them in static browser files."
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
                        "description": format!(
                            "Optional container image used to run the app. Defaults to the installed {} image when available; use this only to override with a dedicated runner image.",
                            crate::branding::PRODUCT_NAME
                        )
                    },
                    "runtime_preference": {
                        "type": "string",
                        "enum": ["local", "container"],
                        "description": format!(
                            "Preferred runtime for dynamic apps. Default: container when Docker is configured for {}, otherwise local.",
                            crate::branding::PRODUCT_NAME
                        )
                    },
                    "runtime_required": {
                        "type": "boolean",
                        "description": "Whether the generated bundle needs a long-lived server/runtime. Omit to infer from entry_command/start_command; set false only when a bundle with lifecycle metadata should still be served as static files."
                    },
                    "runtime_reason": {
                        "type": "string",
                        "description": "Optional short explanation of why a dynamic runtime is needed for this generated bundle."
                    },
                    "expose_public": {
                        "type": "boolean",
                        "description": "Whether to expose this deployment through the configured remote-access provider. Default: false; ordinary app deployment remains local even if the app content is intended to be shared or read-only."
                    },
                    "access_guard": {
                        "type": "boolean",
                        "description": "Enable access-password guard for the local app URL. Defaults to false for local app deployments. Public exposure has its own mandatory public-surface guard and does not change this local setting."
                    },
                    "access_password": {
                        "type": "string",
                        "description": "Optional operator-chosen access password. Providing it enables local App Guard unless public exposure is the only guarded surface. If public exposure is requested and this is omitted, AgentArk generates a public-surface password."
                    },
                    "replace_existing": {
                        "type": "boolean",
                        "description": "Update/recreate the targeted deployed app in place when an app_id or matching app is available. Default: false."
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
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // Provider-based text/image-to-video generation (Runway/Luma/Fal/Veo/etc.)
        self.register_builtin_action(ActionDef {
            name: "generate_video".to_string(),
            description: "Generate an AI video via configured video providers (Runway, Luma, Fal, Sora, Veo, etc.) for text-to-video or image-to-video requests.".to_string(),
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
        authorization: integration_authorization("media_gen"),
        }).await;

        // Self-evolve - policy-first self-improvement
        self.register_builtin_action(ActionDef {
            name: "self_evolve".to_string(),
            description: format!(
                "Evolve {} behavior with an auditable promotion loop. Default mode is policy/strategy evolution (benchmark, lineage archive, statistical gating, canary rollout with replay gate, optional promotion). Code evolution is disabled by default and requires explicit allow_code_writes=true.",
                crate::branding::PRODUCT_NAME
            ),
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
                        "enum": ["policy", "strategy", "prompt", "specialist_prompt", "gepa_export", "gepa_run", "gepa_import", "gepa_status", "code"],
                        "description": "Evolution mode. policy (default) evolves runtime strategy; prompt/specialist_prompt evolve prompt surfaces; GEPA modes run offline seed export/run/import/status; code enables source mutation mode."
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
                    },
                    "gepa_run_id": {
                        "type": "string",
                        "description": "Optional GEPA run id used to locate .agentark/self_evolve/gepa/runs/<run_id> artifacts."
                    },
                    "export_path": {
                        "type": "string",
                        "description": "Optional path to a GEPA export.json file for gepa_run."
                    },
                    "candidates_path": {
                        "type": "string",
                        "description": "Optional path to GEPA candidates.jsonl for gepa_import or gepa_run output."
                    },
                    "gepa_quiet_window_seconds": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Quiet window required before GEPA work starts. Default 60."
                    },
                    "gepa_optimizer_timeout_seconds": {
                        "type": "integer",
                        "minimum": 30,
                        "description": "Maximum wall-clock seconds for the offline GEPA optimizer process. Default 900."
                    }
                }
            }),
            capabilities: vec!["self_evolve".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // ==================== ArkOrbit (per-user canvas) ====================

        self.register_builtin_action(ActionDef {
            name: "arkorbit_create_orbit".to_string(),
            description: "Create a new ArkOrbit canvas backed by durable orbit files. Use when the user wants a fresh, separate space for a different topic, project, or purpose. The new canvas is owned by the active user and persisted to disk; it does not become the default unless the user has none yet.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Short human-readable label shown in the orbit switcher."
                    },
                    "icon": {
                        "type": "string",
                        "description": "Optional emoji or short glyph rendered alongside the name in the switcher."
                    },
                    "color": {
                        "type": "string",
                        "description": "Optional CSS color string (e.g. '#7c3aed') used to tint the orbit chip."
                    },
                    "agent_instructions": {
                        "type": "string",
                        "description": "Optional free-form instructions scoped to this orbit. The agent receives them as structural context whenever the user chats inside this canvas."
                    }
                },
                "required": ["name"]
            }),
            capabilities: vec!["arkorbit".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "arkorbit_file_write".to_string(),
            description: "Fallback write primitive for ArkOrbit files. The fast orbit chat path normally applies structured orbit file operations directly; this action exists for non-streaming providers and other structured tool paths. The path must be index.html, orbit.json, or under mod/, data/, or assets/.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "orbit_id": {
                        "type": "string",
                        "description": "Selected orbit identifier."
                    },
                    "path": {
                        "type": "string",
                        "description": "Relative orbit file path. Allowed roots: mod/, data/, assets/, index.html, orbit.json."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full file contents to write atomically."
                    }
                },
                "required": ["orbit_id", "path", "content"]
            }),
            capabilities: vec!["arkorbit".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        Ok(())
    }

    fn normalize_action_definition(info: ActionDef) -> ActionDef {
        let mut normalized = info;
        normalized.authorization = Self::merged_authorization_for_action(&normalized);
        normalized
    }

    fn merged_authorization_for_action(info: &ActionDef) -> ActionAuthorization {
        let defaults = Self::default_authorization_for_action(info);
        let mut authorization = info.authorization.clone();
        if matches!(authorization.risk_level, ActionRiskLevel::None) {
            authorization.risk_level = defaults.risk_level;
        }
        if !authorization.requires_auth {
            authorization.requires_auth = defaults.requires_auth;
        }
        if authorization.allowed_roles.is_empty() {
            authorization.allowed_roles = defaults.allowed_roles;
        }
        if authorization.rate_limit.is_none() {
            authorization.rate_limit = defaults.rate_limit;
        }
        if !authorization.human_approval.required {
            authorization.human_approval.required = defaults.human_approval.required;
        }
        authorization
    }

    fn default_authorization_for_action(info: &ActionDef) -> ActionAuthorization {
        let lowered = info.name.trim().to_ascii_lowercase();
        let dangerous = Self::action_has_dangerous_capabilities(&info.capabilities);
        let background_sensitive =
            BACKGROUND_BLOCKED_ACTIONS.contains(&lowered.as_str()) || dangerous;

        if background_sensitive {
            return ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                ..Default::default()
            };
        }

        ActionAuthorization::default()
    }

    fn action_has_dangerous_capabilities(capabilities: &[String]) -> bool {
        capabilities.iter().any(|cap| {
            let permission = crate::security::action_guard::ActionGuard::parse_permission(cap);
            !matches!(
                permission,
                crate::security::action_guard::Permission::Custom(_)
            ) && matches!(
                crate::security::action_guard::ActionGuard::permission_risk(&permission),
                crate::security::action_guard::PermissionRisk::Dangerous
            )
        })
    }

    fn is_background_surface(surface: &ActionExecutionSurface) -> bool {
        matches!(
            surface,
            ActionExecutionSurface::Automation | ActionExecutionSurface::Background
        )
    }

    fn direct_trusted_chat_tool_override(auth_context: &ActionAuthorizationContext) -> bool {
        matches!(auth_context.surface, ActionExecutionSurface::Chat)
            && auth_context.direct_user_intent
            && auth_context
                .principal
                .as_ref()
                .is_some_and(|principal| principal.trusted)
    }

    fn risk_rank(level: &ActionRiskLevel) -> u8 {
        match level {
            ActionRiskLevel::None => 0,
            ActionRiskLevel::Low => 1,
            ActionRiskLevel::Medium => 2,
            ActionRiskLevel::High => 3,
            ActionRiskLevel::Critical => 4,
        }
    }

    fn truncate_audit_text(raw: &str, max_chars: usize) -> String {
        let redacted = crate::security::redact_pii(raw);
        let mut truncated = redacted.chars().take(max_chars).collect::<String>();
        if redacted.chars().count() > max_chars {
            truncated.push_str("...");
        }
        truncated
    }

    fn normalize_optional_audit_text(raw: Option<&str>, max_chars: usize) -> Option<String> {
        raw.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Self::truncate_audit_text(value, max_chars))
    }

    async fn log_authorization_audit(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        authorization: &ActionAuthorization,
        auth_context: &ActionAuthorizationContext,
        decision: &ActionAuthorizationDecision,
    ) {
        let Some(storage) = self.storage() else {
            return;
        };
        let principal_payload = auth_context.principal.as_ref().map(|principal| {
            serde_json::json!({
                "user_id": principal.user_id,
                "role": principal.role,
                "auth_source": principal.auth_source,
                "trusted": principal.trusted,
            })
        });
        let payload = serde_json::json!({
            "surface": auth_context.surface.as_key(),
            "direct_user_intent": auth_context.direct_user_intent,
            "current_turn_is_explicit_approval": auth_context.current_turn_is_explicit_approval,
            "principal": principal_payload,
            "authorization": authorization,
            "decision": {
                "allowed": decision.allowed,
                "reason": decision.reason,
                "matched_role": decision.matched_role,
                "rate_limit_key": decision.rate_limit_key,
            }
        });
        let arguments_text = serde_json::to_string(arguments)
            .ok()
            .map(|value| Self::truncate_audit_text(&value, 1200));
        let payload_text = serde_json::to_string(&payload)
            .ok()
            .map(|value| Self::truncate_audit_text(&value, 2000));
        let row = crate::storage::entities::operational_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            trace_id: None,
            conversation_id: None,
            channel: Self::truncate_audit_text(auth_context.surface.as_key(), 64),
            event_type: "tool_authorization".to_string(),
            success: decision.allowed,
            outcome: Self::truncate_audit_text(
                if decision.allowed {
                    "allowed"
                } else {
                    "blocked"
                },
                64,
            ),
            tool_name: Some(Self::truncate_audit_text(action_name, 128)),
            latency_ms: None,
            arguments: arguments_text,
            payload: payload_text,
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: Self::normalize_optional_audit_text(
                auth_context
                    .principal
                    .as_ref()
                    .map(|principal| principal.auth_source.as_str()),
                128,
            ),
        };
        if let Err(error) = storage.insert_operational_log(&row).await {
            tracing::debug!("Failed to insert authorization audit log: {}", error);
        }
    }

    fn capability_context_key(auth_context: &ActionAuthorizationContext) -> Option<String> {
        auth_context
            .capability_context_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(256).collect::<String>())
    }

    fn prune_capability_run_contexts(
        contexts: &mut HashMap<String, CapabilityRunCorrelationRecord>,
    ) {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(CAPABILITY_CONTEXT_TTL_SECS);
        contexts.retain(|_, record| record.updated_at >= cutoff);
        while contexts.len() > CAPABILITY_CONTEXT_LIMIT {
            let Some(oldest_key) = contexts
                .iter()
                .min_by_key(|(_, record)| record.updated_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            contexts.remove(&oldest_key);
        }
    }

    fn capability_correlation_message(
        action_name: &str,
        decision: &crate::security::capabilities::CapabilityCorrelationDecision,
    ) -> String {
        let detail = decision.message.as_deref().unwrap_or(match decision.effect {
            crate::security::capabilities::CapabilityCorrelationEffect::Block => {
                "Blocked by cross-layer capability policy."
            }
            crate::security::capabilities::CapabilityCorrelationEffect::RequireApproval => {
                "This action requires explicit approval because it combines sensitive access with external delivery."
            }
            crate::security::capabilities::CapabilityCorrelationEffect::Allow => {
                "Allowed by capability policy."
            }
        });
        match decision.effect {
            crate::security::capabilities::CapabilityCorrelationEffect::Block => {
                format!(
                    "Tool '{}' is blocked by security policy. {}",
                    action_name, detail
                )
            }
            crate::security::capabilities::CapabilityCorrelationEffect::RequireApproval => {
                format!(
                    "Tool '{}' requires explicit approval before it can run. {}",
                    action_name, detail
                )
            }
            crate::security::capabilities::CapabilityCorrelationEffect::Allow => detail.to_string(),
        }
    }

    async fn authorize_capability_correlation(
        &self,
        action_name: &str,
        action_def: &ActionDef,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Option<ActionAuthorizationDecision> {
        if matches!(
            auth_context.surface,
            ActionExecutionSurface::Internal | ActionExecutionSurface::Test
        ) {
            return None;
        }
        let Some(context_key) = Self::capability_context_key(auth_context) else {
            return None;
        };
        let candidate = crate::security::capabilities::observations_from_action_def(
            "runtime",
            action_def,
            Some(arguments),
        );
        if candidate.is_empty() {
            return None;
        }

        let mut contexts = self.capability_run_contexts.write().await;
        Self::prune_capability_run_contexts(&mut contexts);
        let record =
            contexts
                .entry(context_key.clone())
                .or_insert_with(|| CapabilityRunCorrelationRecord {
                    updated_at: chrono::Utc::now(),
                    context: crate::security::capabilities::RunCapabilityContext::default(),
                });
        let decision = crate::security::capabilities::evaluate_capability_correlation(
            record.context.observations(),
            &candidate,
        );
        match decision.effect {
            crate::security::capabilities::CapabilityCorrelationEffect::Allow => {
                record.context.extend(candidate);
                record
                    .context
                    .retain_recent(CAPABILITY_CONTEXT_OBSERVATION_LIMIT);
                record.updated_at = chrono::Utc::now();
                None
            }
            crate::security::capabilities::CapabilityCorrelationEffect::Block => {
                drop(contexts);
                self.record_capability_correlation_decision(
                    action_name,
                    &context_key,
                    "blocked",
                    &decision,
                )
                .await;
                Some(ActionAuthorizationDecision::deny(
                    Self::capability_correlation_message(action_name, &decision),
                ))
            }
            crate::security::capabilities::CapabilityCorrelationEffect::RequireApproval => {
                if auth_context.current_turn_is_explicit_approval {
                    record.context.extend(candidate);
                    record
                        .context
                        .retain_recent(CAPABILITY_CONTEXT_OBSERVATION_LIMIT);
                    record.updated_at = chrono::Utc::now();
                    drop(contexts);
                    self.record_capability_correlation_decision(
                        action_name,
                        &context_key,
                        "approved",
                        &decision,
                    )
                    .await;
                    None
                } else {
                    drop(contexts);
                    self.record_capability_correlation_decision(
                        action_name,
                        &context_key,
                        "approval_required",
                        &decision,
                    )
                    .await;
                    Some(ActionAuthorizationDecision::require_explicit_approval(
                        Self::capability_correlation_message(action_name, &decision),
                    ))
                }
            }
        }
    }

    async fn record_capability_correlation_decision(
        &self,
        action_name: &str,
        context_key: &str,
        outcome: &str,
        decision: &crate::security::capabilities::CapabilityCorrelationDecision,
    ) {
        let Some(report) = decision.report.as_ref() else {
            return;
        };
        let rules = report
            .matched_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let subjects = report
            .observations
            .iter()
            .map(|observation| format!("{}:{}", observation.layer, observation.entity_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        let severity = if matches!(
            decision.effect,
            crate::security::capabilities::CapabilityCorrelationEffect::Block
        ) {
            "high"
        } else {
            "medium"
        };
        self.record_security_event(
            "capability_correlation",
            severity,
            format!(
                "Runtime capability correlation: outcome={}, action='{}', rules=[{}], subjects=[{}]",
                outcome, action_name, rules, subjects
            ),
            Some(format!(
                "scope=runtime;context={};action={}",
                Self::truncate_audit_text(context_key, 128),
                action_name
            )),
        )
        .await;
    }

    pub async fn authorize_action_invocation(
        &self,
        action_name: &str,
        action_def: Option<&ActionDef>,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<ActionAuthorizationDecision> {
        let authorization = action_def
            .map(Self::merged_authorization_for_action)
            .unwrap_or_default();

        if let Some(decision) = self
            .authorize_action_scope(action_name, arguments, auth_context)
            .await
        {
            self.log_authorization_audit(
                action_name,
                arguments,
                &authorization,
                auth_context,
                &decision,
            )
            .await;
            return Ok(decision);
        }

        let decision = match auth_context.surface {
            ActionExecutionSurface::Internal | ActionExecutionSurface::Test => {
                ActionAuthorizationDecision::allow(
                    "Internal execution bypassed the interactive permission gate.",
                )
            }
            _ if authorization.human_approval.required
                && !auth_context.current_turn_is_explicit_approval =>
            {
                ActionAuthorizationDecision::deny(format!(
                    "Tool '{}' requires explicit user approval before it can run.",
                    action_name
                ))
            }
            _ if Self::direct_trusted_chat_tool_override(auth_context) => {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because this is a direct authenticated chat request.",
                    action_name
                ))
            }
            _ if auth_context.direct_user_intent
                && matches!(
                    auth_context.surface,
                    ActionExecutionSurface::Chat | ActionExecutionSurface::Api
                )
                && auth_context
                    .principal
                    .as_ref()
                    .is_some_and(|principal| principal.trusted) =>
            {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because this is a direct authenticated user request.",
                    action_name
                ))
            }
            _ if Self::is_background_surface(&auth_context.surface)
                && auth_context.direct_user_intent
                && auth_context
                    .principal
                    .as_ref()
                    .is_some_and(|principal| principal.trusted) =>
            {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because this automation originated from a direct authenticated user request.",
                    action_name
                ))
            }
            _ if Self::is_background_surface(&auth_context.surface)
                && Self::risk_rank(&authorization.risk_level)
                    >= Self::risk_rank(&ActionRiskLevel::High) =>
            {
                ActionAuthorizationDecision::deny(format!(
                    "Tool '{}' is blocked in background or automation runs. Start it from a direct authenticated chat or API request instead.",
                    action_name
                ))
            }
            _ if authorization.requires_auth
                && !auth_context
                    .principal
                    .as_ref()
                    .is_some_and(|principal| principal.trusted) =>
            {
                ActionAuthorizationDecision::deny(format!(
                    "Tool '{}' requires a trusted local session. Run it from the authenticated UI or API instead of a background or anonymous context.",
                    action_name
                ))
            }
            _ if !authorization.allowed_roles.is_empty() => {
                let Some(principal) = auth_context.principal.as_ref() else {
                    let decision = ActionAuthorizationDecision::deny(format!(
                        "Tool '{}' requires an authorized local session with role access.",
                        action_name
                    ));
                    self.log_authorization_audit(
                        action_name,
                        arguments,
                        &authorization,
                        auth_context,
                        &decision,
                    )
                    .await;
                    return Ok(decision);
                };
                let matched_role = authorization
                    .allowed_roles
                    .iter()
                    .find(|role| role.eq_ignore_ascii_case(principal.role.as_str()))
                    .cloned();
                if let Some(role) = matched_role {
                    let mut decision = ActionAuthorizationDecision::allow(format!(
                        "Tool '{}' is allowed for the current trusted local session.",
                        action_name
                    ));
                    decision.matched_role = Some(role);
                    decision
                } else {
                    ActionAuthorizationDecision::deny(format!(
                        "Tool '{}' is not allowed for the current local session role '{}'.",
                        action_name, principal.role
                    ))
                }
            }
            _ => ActionAuthorizationDecision::allow(format!(
                "Tool '{}' is allowed for this request.",
                action_name
            )),
        };

        self.log_authorization_audit(
            action_name,
            arguments,
            &authorization,
            auth_context,
            &decision,
        )
        .await;
        if !decision.allowed {
            return Ok(decision);
        }

        if let Some(action_def) = action_def {
            if let Some(capability_decision) = self
                .authorize_capability_correlation(action_name, action_def, arguments, auth_context)
                .await
            {
                self.log_authorization_audit(
                    action_name,
                    arguments,
                    &authorization,
                    auth_context,
                    &capability_decision,
                )
                .await;
                return Ok(capability_decision);
            }

            let unapproved_permissions = self
                .unapproved_permissions_for_action(action_def, auth_context)
                .await;
            if !unapproved_permissions.is_empty() {
                let denied = ActionAuthorizationDecision::deny(
                    Self::build_permission_requirement_error(action_name, &unapproved_permissions),
                );
                self.log_authorization_audit(
                    action_name,
                    arguments,
                    &authorization,
                    auth_context,
                    &denied,
                )
                .await;
                return Ok(denied);
            }
        }

        Ok(decision)
    }

    async fn register_builtin_action(&self, info: ActionDef) {
        let info = Self::normalize_action_definition(info);
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
                extension_pack_binding: None,
            },
        );
    }

    /// Register an action with workflow content from SKILL.md
    async fn register_workflow_action(&self, info: ActionDef, workflow: String) {
        let info = Self::normalize_action_definition(info);
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
                extension_pack_binding: None,
            },
        );
    }

    async fn register_cli_action(&self, info: ActionDef, binding: CliToolBinding) {
        let info = Self::normalize_action_definition(info);
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
                extension_pack_binding: None,
            },
        );
    }

    /// Register an MCP-backed action (external tool/resource)
    pub async fn register_mcp_action(&self, info: ActionDef, binding: McpBinding) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_mcp_action(&info, &binding).await;
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
                extension_pack_binding: None,
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!("Failed to persist MCP action review state: {}", error);
                }
            }
            Err(error) => {
                tracing::warn!("Failed to review MCP action during registration: {}", error);
            }
        }
    }

    /// Register a plugin-backed action
    pub async fn register_plugin_action(&self, info: ActionDef, binding: PluginBinding) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_plugin_action(&info, &binding).await;
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
                extension_pack_binding: None,
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!("Failed to persist plugin action review state: {}", error);
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to review plugin action during registration: {}",
                    error
                );
            }
        }
    }

    /// Register an imported custom API action.
    pub async fn register_custom_api_action(&self, info: ActionDef, binding: CustomApiBinding) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_custom_api_action(&info, &binding).await;
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
                extension_pack_binding: None,
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!(
                        "Failed to persist custom API action review state: {}",
                        error
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to review custom API action during registration: {}",
                    error
                );
            }
        }
    }

    /// Register an installed extension-pack feature as a real runtime action.
    pub async fn register_extension_pack_action(
        &self,
        info: ActionDef,
        binding: ExtensionPackActionBinding,
    ) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_extension_pack_action(&info, &binding).await;
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
                extension_pack_binding: Some(binding),
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!(
                        "Failed to persist extension-pack action review state: {}",
                        error
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to review extension-pack action during registration: {}",
                    error
                );
            }
        }
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
            authorization: Default::default(),
        }
    }

    #[cfg(test)]
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
        if let Some(ref guard) = self.action_guard {
            if let Err(error) = guard.resign_action(&skill_dir, skill_name).await {
                tracing::warn!("Failed to sign CLI skill '{}': {}", skill_name, error);
            }
        }
        let (_parsed, workflow_content, frontmatter) = self
            .parse_action_md(&skill_path, ActionSource::Custom)
            .await?;
        let binding = CliToolBinding {
            executable_path: manifest.executable_path.clone(),
            verify_args: manifest.verify_args.clone(),
            auth_profile_id: Self::extract_auth_profile_id_from_frontmatter(&frontmatter),
            auth_env_exports: Self::extract_auth_env_exports_from_frontmatter(&frontmatter),
        };
        let review = self
            .review_cli_action(&skill_dir, &info, &workflow_content, &frontmatter, &binding)
            .await?;
        self.register_cli_action(info, binding).await;
        self.upsert_action_review(review.clone()).await?;
        tracing::info!(
            "Installed CLI skill '{}' backed by {}",
            skill_name,
            manifest.executable_path
        );
        if !review.allow_execute {
            tracing::warn!(
                "CLI skill '{}' installed in blocked/unready state: {:?}",
                skill_name,
                review.blocked_reason
            );
        }
        Ok(())
    }

    async fn load_cli_skill_actions(&self) -> Result<()> {
        let mut entries = match tokio::fs::read_dir(&self.cli_skills_dir).await {
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

        while let Ok(Some(entry)) = entries.next_entry().await {
            let is_dir = entry
                .file_type()
                .await
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            let path = entry.path();
            let manifest_path = path.join("manifest.json");
            let skill_path = path.join("SKILL.md");
            let manifest_exists = tokio::fs::metadata(&manifest_path)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            let skill_exists = tokio::fs::metadata(&skill_path)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            if !manifest_exists || !skill_exists {
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
            let parsed = match self
                .parse_action_md(&skill_path, ActionSource::Custom)
                .await
            {
                Ok(parsed) => parsed,
                Err(error) => {
                    tracing::warn!(
                        "Failed to parse CLI skill markdown {:?}: {}",
                        skill_path,
                        error
                    );
                    continue;
                }
            };
            let (_parsed_info, workflow_content, frontmatter) = parsed;
            let binding = CliToolBinding {
                executable_path: manifest.executable_path.clone(),
                verify_args: manifest.verify_args.clone(),
                auth_profile_id: Self::extract_auth_profile_id_from_frontmatter(&frontmatter),
                auth_env_exports: Self::extract_auth_env_exports_from_frontmatter(&frontmatter),
            };
            let review = self
                .review_cli_action(&path, &info, &workflow_content, &frontmatter, &binding)
                .await?;
            self.register_cli_action(info, binding).await;
            self.upsert_action_review(review).await?;
        }

        Ok(())
    }

    /// Remove all MCP-backed actions
    pub async fn unregister_mcp_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.mcp_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.mcp_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove MCP-backed actions for a specific server
    pub async fn unregister_mcp_actions_for_server(&self, server_id: &str) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| {
                    action
                        .mcp_binding
                        .as_ref()
                        .is_some_and(|binding| binding.server_id == server_id)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| {
                if let Some(binding) = &a.mcp_binding {
                    binding.server_id != server_id
                } else {
                    true
                }
            });
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove all plugin-backed actions
    pub async fn unregister_plugin_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.plugin_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.plugin_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove plugin-backed actions for a specific plugin
    pub async fn unregister_plugin_actions_for_plugin(&self, plugin_id: &str) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| {
                    action
                        .plugin_binding
                        .as_ref()
                        .is_some_and(|binding| binding.plugin_id == plugin_id)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| {
                if let Some(binding) = &a.plugin_binding {
                    binding.plugin_id != plugin_id
                } else {
                    true
                }
            });
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove all imported custom API actions.
    pub async fn unregister_custom_api_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.custom_api_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.custom_api_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove all installed extension-pack feature actions.
    pub async fn unregister_extension_pack_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.extension_pack_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.extension_pack_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
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
        action_name: &str,
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
        let review = self
            .get_action_review(action_name)
            .await
            .unwrap_or_default();
        let mut injected_env = BTreeMap::new();
        if !review.required_env.is_empty() {
            let required_secret_env = review
                .required_env
                .iter()
                .filter(|env| !binding.auth_env_exports.contains_key(*env))
                .cloned()
                .collect::<Vec<_>>();
            let placeholder_map = required_secret_env
                .iter()
                .map(|env| {
                    (
                        env.clone(),
                        serde_json::Value::String(format!("{{{{env:{}}}}}", env)),
                    )
                })
                .collect::<serde_json::Map<String, serde_json::Value>>();
            if !placeholder_map.is_empty() {
                let resolved = self.resolve_secret_placeholders(
                    action_name,
                    &serde_json::Value::Object(placeholder_map),
                )?;
                if let Some(obj) = resolved.as_object() {
                    for env in &required_secret_env {
                        if let Some(value) = obj.get(env).and_then(|value| value.as_str()) {
                            injected_env.insert(env.clone(), value.to_string());
                        }
                    }
                }
            }
        }
        if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            if binding.auth_env_exports.is_empty() {
                anyhow::bail!(
                    "CLI auth profile '{}' is bound but no auth.env_exports mapping is declared.",
                    auth_profile_id
                );
            }
            let storage = self.storage().ok_or_else(|| {
                anyhow::anyhow!("Storage is unavailable for auth profile lookups")
            })?;
            let auth_exports =
                crate::core::auth_profiles::AuthProfileControlPlane::resolve_env_exports(
                    &storage,
                    auth_profile_id,
                    &binding.auth_env_exports,
                )
                .await?;
            for (key, value) in auth_exports {
                injected_env.insert(key, value);
            }
        }

        let mut command = tokio::process::Command::new(executable);
        command.args(&args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        for (key, value) in injected_env {
            command.env(key, value);
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
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                if let Some(storage) = self.storage() {
                    let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                        &storage,
                        auth_profile_id,
                    )
                    .await;
                }
            }
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
        self.execute_action_with_context(
            action_name,
            arguments,
            &ActionAuthorizationContext::default(),
        )
        .await
    }

    pub async fn validate_action_invocation_with_context(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<()> {
        let info = {
            let actions = self.actions.read().await;
            actions
                .get(action_name)
                .map(|action| action.info.clone())
                .ok_or_else(|| {
                    crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::NotFound,
                        format!("Unknown action: {}", action_name),
                    )
                })?
        };

        let authorization_decision = self
            .authorize_action_invocation(action_name, Some(&info), arguments, auth_context)
            .await?;
        if !authorization_decision.allowed {
            return Err(crate::actions::structured_action_error(
                ActionErrorDomain::Auth,
                ActionErrorReason::PermissionDenied,
                authorization_decision.reason,
            ));
        }
        let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

        if !chat_override {
            match self.refresh_action_review_state(action_name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        return Err(crate::actions::structured_action_error(
                            ActionErrorDomain::Action,
                            ActionErrorReason::Unavailable,
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to execute.", action_name)
                            }),
                        ));
                    }
                }
                None if info.source != ActionSource::System => {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' has no persisted security review and cannot execute.",
                            action_name
                        ),
                    ));
                }
                None => {}
            }
        }

        if !chat_override {
            if info.source != ActionSource::System {
                let disabled = self.disabled_actions.read().await;
                if disabled.contains(action_name) {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' is disabled. Re-enable it in the UI before running.",
                            action_name
                        ),
                    ));
                }
            } else if !self.is_action_integration_ready(&info).await {
                let integration_id = info
                    .authorization
                    .access
                    .integration_ids
                    .first()
                    .or_else(|| info.authorization.access.extension_pack_ids.first())
                    .map(String::as_str)
                    .unwrap_or("required");
                return Err(crate::actions::structured_action_error(
                    ActionErrorDomain::Integration,
                    ActionErrorReason::NotConnected,
                    format!(
                        "Action '{}' is unavailable because required integration '{}' is not ready.",
                        action_name, integration_id
                    ),
                ));
            }
        }

        match action_name {
            "http_get" => {
                let url = arguments
                    .get("url")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        crate::actions::structured_action_error(
                            ActionErrorDomain::Action,
                            ActionErrorReason::MissingInput,
                            "Missing URL",
                        )
                    })?;
                self.resolve_http_get_url_for_context(url, auth_context)
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }

    pub async fn execute_action_with_context(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        let (
            sandbox_mode,
            cli_binding,
            mcp_binding,
            plugin_binding,
            custom_api_binding,
            extension_pack_binding,
            source,
            info,
        ) = {
            let actions = self.actions.read().await;
            let action = actions.get(action_name).ok_or_else(|| {
                crate::actions::structured_action_error(
                    ActionErrorDomain::Action,
                    ActionErrorReason::NotFound,
                    format!("Unknown action: {}", action_name),
                )
            })?;
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
                action.extension_pack_binding.clone(),
                action.info.source.clone(),
                action.info.clone(),
            )
        };

        let authorization_decision = self
            .authorize_action_invocation(action_name, Some(&info), arguments, auth_context)
            .await?;
        if !authorization_decision.allowed {
            return Err(crate::actions::structured_action_error(
                ActionErrorDomain::Auth,
                ActionErrorReason::PermissionDenied,
                authorization_decision.reason,
            ));
        }
        let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

        if !chat_override {
            match self.refresh_action_review_state(action_name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        return Err(crate::actions::structured_action_error(
                            ActionErrorDomain::Action,
                            ActionErrorReason::Unavailable,
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to execute.", action_name)
                            }),
                        ));
                    }
                }
                None if source != ActionSource::System => {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' has no persisted security review and cannot execute.",
                            action_name
                        ),
                    ));
                }
                None => {}
            }
        }

        if !chat_override {
            if source != ActionSource::System {
                let disabled = self.disabled_actions.read().await;
                if disabled.contains(action_name) {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' is disabled. Re-enable it in the UI before running.",
                            action_name
                        ),
                    ));
                }
            } else if !self.is_action_integration_ready(&info).await {
                let integration_id = info
                    .authorization
                    .access
                    .integration_ids
                    .first()
                    .or_else(|| info.authorization.access.extension_pack_ids.first())
                    .map(String::as_str)
                    .unwrap_or("required");
                return Err(crate::actions::structured_action_error(
                    ActionErrorDomain::Integration,
                    ActionErrorReason::NotConnected,
                    format!(
                        "Action '{}' is unavailable because required integration '{}' is not ready.",
                        action_name, integration_id
                    ),
                ));
            }
        }

        // Resolve secrets at execution time so they never appear in LLM-visible
        // tool-call arguments or execution traces.
        let resolved_args = self.resolve_secret_placeholders(action_name, arguments)?;

        #[cfg(feature = "ssh")]
        if matches!(action_name, "ssh" | "ssh_connections") {
            let allowed_connections = auth_context
                .agent_access_scope
                .as_ref()
                .map(|scope| scope.ssh_connection_names.as_slice());
            return match action_name {
                "ssh" => {
                    crate::actions::ssh::ssh_execute_scoped(
                        &self.config_dir,
                        &resolved_args,
                        allowed_connections,
                    )
                    .await
                }
                "ssh_connections" => {
                    crate::actions::ssh::ssh_list_connections_scoped(
                        &self.config_dir,
                        allowed_connections,
                    )
                    .await
                }
                _ => unreachable!(),
            };
        }

        if let Some(binding) = cli_binding {
            return self
                .execute_cli_action(action_name, binding, &resolved_args)
                .await;
        }

        if let Some(binding) = mcp_binding {
            return self.execute_mcp_action(binding, &resolved_args).await;
        }

        if let Some(binding) = plugin_binding {
            let outbound_args = if Self::action_def_requires_outbound_gate(&info) {
                Self::sanitize_outbound_action_arguments(action_name, &resolved_args)?
            } else {
                resolved_args.clone()
            };
            return self.execute_plugin_action(binding, &outbound_args).await;
        }

        if let Some(binding) = custom_api_binding {
            let outbound_args = if binding.read_only {
                resolved_args.clone()
            } else {
                Self::sanitize_outbound_action_arguments(action_name, &resolved_args)?
            };
            return self
                .execute_custom_api_action(binding, &outbound_args)
                .await;
        }

        if let Some(binding) = extension_pack_binding {
            let outbound_args = if binding.read_only {
                resolved_args.clone()
            } else {
                Self::sanitize_outbound_action_arguments(action_name, &resolved_args)?
            };
            return self
                .execute_extension_pack_action(binding, &outbound_args)
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
            SandboxMode::Wasm => {
                self.execute_wasm(action_name, &resolved_args, auth_context)
                    .await
            }
            SandboxMode::Docker => {
                self.execute_docker(action_name, &resolved_args, auth_context)
                    .await
            }
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
                let placeholder_kind = match kind {
                    "secret" => MissingSecretPlaceholderKind::Secret,
                    "env" => MissingSecretPlaceholderKind::Env,
                    _ => MissingSecretPlaceholderKind::Secret,
                };
                let val = match placeholder_kind {
                    MissingSecretPlaceholderKind::Secret => resolve_secret(key),
                    MissingSecretPlaceholderKind::Env => resolve_env(key),
                }
                .ok_or_else(|| {
                    anyhow::Error::new(MissingSecretPlaceholder::new(
                        action_name,
                        placeholder_kind,
                        key,
                    ))
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

    fn action_def_requires_outbound_gate(info: &ActionDef) -> bool {
        let outbound = &info.authorization.outbound;
        !outbound.read_only && (outbound.outbound_write || outbound.public_publish)
    }

    fn sanitize_outbound_action_arguments(
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let privacy = crate::security::sanitize_outbound_json(
            arguments,
            &crate::security::OutboundPrivacyPolicy::default(),
        );
        match privacy.decision {
            crate::security::OutboundPrivacyDecision::Allow => Ok(arguments.clone()),
            crate::security::OutboundPrivacyDecision::RedactedAllow => {
                tracing::warn!(
                    action = action_name,
                    redactions = ?privacy.redactions,
                    reasons = ?privacy.reasons,
                    "Outbound privacy gate redacted action arguments"
                );
                Ok(privacy.sanitized_value)
            }
            crate::security::OutboundPrivacyDecision::Block => Err(anyhow::anyhow!(
                "{}",
                crate::security::format_outbound_privacy_block(
                    &format!("action '{}'", action_name),
                    &privacy.reasons,
                )
            )),
        }
    }

    async fn execute_mcp_action(
        &self,
        binding: McpBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("MCP action arguments denied by outbound URL guard")?;
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
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("Plugin action arguments denied by outbound URL guard")?;
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
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("Custom API action arguments denied by outbound URL guard")?;
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
        let auth_overlay = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            Some(
                self.resolve_auth_profile_http(auth_profile_id)
                    .await?
                    .overlay,
            )
        } else {
            None
        };
        if let Some(overlay) = auth_overlay.as_ref() {
            overlay.apply_to_url(&mut url);
        }
        url = crate::security::tool_args_guard::check_outward_url_anyhow(
            url.as_str(),
            &self.tool_args_guard_config(),
        )
        .await
        .with_context(|| format!("custom API URL denied for '{}'", binding.operation_name))?;

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

        request = if let Some(overlay) = auth_overlay.as_ref() {
            overlay.apply_to_request_builder(request)?
        } else {
            match binding.auth_mode {
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
        let rendered = crate::security::redact_secret_input(&rendered).text;
        let rendered = crate::security::sanitize_untrusted_output("custom_api", &rendered);
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Custom API '{}' returned HTTP {}:\n{}",
                binding.operation_name,
                status,
                rendered
            ));
        }
        if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            if let Some(storage) = self.storage() {
                let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                    &storage,
                    auth_profile_id,
                )
                .await;
            }
        }
        Ok(format!(
            "{} {} succeeded.\n{}",
            binding.method.to_ascii_uppercase(),
            binding.operation_name,
            rendered
        ))
    }

    async fn execute_extension_pack_action(
        &self,
        binding: ExtensionPackActionBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("Extension-pack action arguments denied by outbound URL guard")?;
        let registry = self
            .extension_pack_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not initialized"))?;
        let mut registry = registry.write().await;
        let result = registry
            .invoke_feature(
                crate::extension_packs::ExtensionPackInvokeRequest {
                    pack_id: Some(binding.pack_id.clone()),
                    connection_id: binding.connection_id.clone(),
                    feature_id: binding.feature_id.clone(),
                    arguments: arguments.clone(),
                },
                self.mcp_registry.clone(),
                self.plugin_registry.clone(),
            )
            .await?;
        if !result.ok {
            anyhow::bail!(
                "{}",
                result
                    .message
                    .or(result.error)
                    .unwrap_or_else(|| "Extension-pack invocation failed".to_string())
            );
        }
        let payload = serde_json::to_string_pretty(&result.data.unwrap_or(serde_json::Value::Null))
            .unwrap_or_else(|_| "null".to_string());
        Ok(crate::security::sanitize_untrusted_output(
            "extension_pack",
            &crate::security::redact_secret_input(&payload).text,
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
                    output.push_str(&format!(
                        "- {} (id: {}, action: {}, status: {})\n",
                        t.description, t.id, t.action, status_str
                    ));
                    if let Some(ref cron) = t.cron {
                        output.push_str(&format!("  Schedule: {}\n", cron));
                    }
                    if let Some(scheduled_for) = t.scheduled_for {
                        output.push_str(&format!("  Next run: {}\n", scheduled_for.to_rfc3339()));
                    }
                }
                Ok(output)
            }
            "list_watchers" => {
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;
                let filter = arguments
                    .get("filter")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("active");
                let limit = arguments
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(20)
                    .clamp(1, 100) as usize;
                let mut watchers = storage.list_watchers().await?;
                watchers.sort_by(|left, right| right.created_at.cmp(&left.created_at));
                let status_label = |status: &crate::core::watcher::WatcherStatus| -> &'static str {
                    match status {
                        crate::core::watcher::WatcherStatus::Active => "active",
                        crate::core::watcher::WatcherStatus::Paused => "paused",
                        crate::core::watcher::WatcherStatus::Triggered => "triggered",
                        crate::core::watcher::WatcherStatus::TimedOut => "timed_out",
                        crate::core::watcher::WatcherStatus::Cancelled => "cancelled",
                        crate::core::watcher::WatcherStatus::Failed { .. } => "failed",
                    }
                };
                let rows = watchers
                    .into_iter()
                    .filter(|watcher| filter == "all" || status_label(&watcher.status) == filter)
                    .take(limit)
                    .map(|watcher| {
                        let status = status_label(&watcher.status);
                        let status_error = match &watcher.status {
                            crate::core::watcher::WatcherStatus::Failed { error } => {
                                Some(error.clone())
                            }
                            _ => None,
                        };
                        serde_json::json!({
                            "id": watcher.id.to_string(),
                            "description": watcher.description,
                            "poll_action": watcher.poll_action,
                            "condition": watcher.condition,
                            "status": status,
                            "status_error": status_error,
                            "interval_secs": watcher.interval_secs,
                            "timeout_secs": watcher.timeout_secs,
                            "poll_count": watcher.poll_count,
                            "created_at": watcher.created_at.to_rfc3339(),
                            "last_poll_at": watcher.last_poll_at.map(|value| value.to_rfc3339()),
                            "next_poll_not_before": watcher
                                .next_poll_not_before
                                .map(|value| value.to_rfc3339()),
                            "notify_channel": watcher.notify_channel,
                            "on_trigger": watcher.on_trigger,
                            "last_error": watcher.last_error,
                            "last_poll_outcome": watcher.last_poll_outcome,
                        })
                    })
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    return Ok(format!("No {} watcher(s) found.", filter));
                }
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "filter": filter,
                    "count": rows.len(),
                    "watchers": rows,
                }))?)
            }
            "tunnel_control" => self.execute_tunnel_control(arguments).await,
            "background_session_manage" => {
                let operation = arguments
                    .get("operation")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing background session operation"))?;
                let valid = matches!(
                    operation,
                    "status"
                        | "list"
                        | "pause"
                        | "resume"
                        | "stop"
                        | "cancel"
                        | "delete"
                        | "update_delivery"
                );
                if !valid {
                    anyhow::bail!("Unsupported background session operation `{}`", operation);
                }
                if operation == "update_delivery"
                    && arguments
                        .get("delivery_channel")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_none()
                {
                    anyhow::bail!("update_delivery requires delivery_channel");
                }
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "background_session_manage",
                        "status": "completed",
                        "detail": format!("Prepared background session operation: {}", operation),
                    })
                ))
            }
            "schedule_task" => {
                let validate_schedule_item = |item: &serde_json::Value| -> Result<String> {
                    let task_desc = item
                        .get("task")
                        .and_then(|value| value.as_str())
                        .or_else(|| {
                            item.get("task_id")
                                .and_then(|value| value.as_str())
                                .map(|_| "existing task")
                        })
                        .ok_or_else(|| anyhow::anyhow!("Missing task description"))?;

                    let schedule_info =
                        if let Some(cron_expr) = item.get("cron").and_then(|v| v.as_str()) {
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
                        } else if let Some(at_time) = item.get("at").and_then(|v| v.as_str()) {
                            // Validate ISO timestamp
                            chrono::DateTime::parse_from_rfc3339(at_time)
                                .map_err(|e| anyhow::anyhow!("Invalid timestamp: {}", e))?;
                            format!("at:{}", at_time)
                        } else {
                            return Err(anyhow::anyhow!(
                                "Must specify either 'cron' or 'at' for scheduling"
                            ));
                        };
                    Ok(format!("Task: {}; schedule: {}", task_desc, schedule_info))
                };

                let detail = if let Some(items) = arguments.get("items") {
                    let items = items
                        .as_array()
                        .filter(|items| !items.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!("schedule_task.items must be a non-empty array")
                        })?;
                    let inheritable_keys = [
                        "task",
                        "report_to",
                        "action",
                        "action_arguments",
                        "allow_duplicate",
                        "validation",
                        "max_attempts",
                        "stall_timeout_secs",
                        "retry_backoff_secs",
                        "automation_policy",
                    ];
                    for (index, item) in items.iter().enumerate() {
                        let Some(item_obj) = item.as_object() else {
                            return Err(anyhow::anyhow!(
                                "schedule_task.items[{}] must be an object",
                                index
                            ));
                        };
                        let mut merged = serde_json::Map::new();
                        for key in inheritable_keys {
                            if let Some(value) = arguments.get(key) {
                                merged.insert(key.to_string(), value.clone());
                            }
                        }
                        for (key, value) in item_obj {
                            merged.insert(key.clone(), value.clone());
                        }
                        merged.remove("items");
                        validate_schedule_item(&serde_json::Value::Object(merged)).map_err(
                            |error| {
                                anyhow::anyhow!("Invalid schedule item {}: {}", index + 1, error)
                            },
                        )?;
                    }
                    format!("Prepared {} scheduled task item(s)", items.len())
                } else {
                    validate_schedule_item(arguments)?
                };

                // Return structured scheduling info - actual scheduling is handled by the agent's task queue
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "schedule_task",
                        "status": "completed",
                        "detail": detail,
                    })
                ))
            }
            "watch" => {
                // Return structured watcher info - actual watcher creation is handled by Agent::handle_watch
                let validate_watch_item = |item: &serde_json::Value| -> Result<String> {
                    if item
                        .get("watcher_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .is_some_and(|value| !value.is_empty())
                    {
                        return Ok("existing watcher".to_string());
                    }
                    let desc = item
                        .get("description")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("Missing watcher description"))?;
                    for key in ["poll_action", "condition", "on_trigger"] {
                        if item.get(key).is_none() {
                            return Err(anyhow::anyhow!("Missing watcher `{}`", key));
                        }
                    }
                    Ok(desc.to_string())
                };
                let desc = if let Some(items) = arguments.get("items") {
                    let items = items
                        .as_array()
                        .filter(|items| !items.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("watch.items must be a non-empty array"))?;
                    let inheritable_keys = [
                        "description",
                        "poll_action",
                        "poll_arguments",
                        "condition",
                        "on_trigger",
                        "interval_secs",
                        "timeout_secs",
                        "timeout_hours",
                        "timeout_days",
                        "until_stopped",
                        "notify_channel",
                        "allow_duplicate",
                        "validation",
                        "max_attempts",
                        "stall_timeout_secs",
                        "retry_backoff_secs",
                        "automation_policy",
                    ];
                    for (index, item) in items.iter().enumerate() {
                        let Some(item_obj) = item.as_object() else {
                            return Err(anyhow::anyhow!(
                                "watch.items[{}] must be an object",
                                index
                            ));
                        };
                        let mut merged = serde_json::Map::new();
                        for key in inheritable_keys {
                            if let Some(value) = arguments.get(key) {
                                merged.insert(key.to_string(), value.clone());
                            }
                        }
                        for (key, value) in item_obj {
                            merged.insert(key.clone(), value.clone());
                        }
                        merged.remove("items");
                        validate_watch_item(&serde_json::Value::Object(merged)).map_err(
                            |error| anyhow::anyhow!("Invalid watch item {}: {}", index + 1, error),
                        )?;
                    }
                    format!("Prepared {} watcher item(s)", items.len())
                } else {
                    validate_watch_item(arguments).unwrap_or_else(|_| "watcher".to_string())
                };
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "watch",
                        "status": "completed",
                        "detail": desc,
                    })
                ))
            }
            "delegate" => {
                let task = arguments
                    .get("task")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing delegated task"))?;
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "delegate",
                        "status": "completed",
                        "detail": task,
                    })
                ))
            }
            "manage_actions" => self.execute_manage_actions(arguments).await,
            "ark_inspect" => self.execute_ark_inspect(arguments).await,
            "memory_lookup" => self.execute_memory_lookup(arguments).await,
            "agentark_capability_lookup" => {
                self.execute_agentark_capability_lookup(arguments).await
            }
            "list_integrations" => self.execute_list_integrations(arguments).await,
            "inspect_integration" => self.execute_inspect_integration(arguments).await,
            "postgres_schema_inspect" => self.execute_postgres_schema_inspect(arguments).await,
            "postgres_query_readonly" => self.execute_postgres_query_readonly(arguments).await,
            "capability_acquire" => self.execute_capability_acquire(arguments).await,
            "capability_resolve" => self.execute_capability_resolve(arguments).await,
            "connector_request" => self.execute_connector_request(arguments).await,
            "lan_discover" => crate::actions::lan::lan_discover(arguments).await,
            "extension_pack_list" => self.execute_extension_pack_list(arguments).await,
            "extension_pack_search" => self.execute_extension_pack_search(arguments).await,
            "extension_pack_install" => self.execute_extension_pack_install(arguments).await,
            "extension_pack_scaffold" => self.execute_extension_pack_scaffold(arguments).await,
            "custom_messaging_channel_upsert" => {
                self.execute_custom_messaging_channel_upsert(arguments)
                    .await
            }
            "extension_pack_connect" => self.execute_extension_pack_connect(arguments).await,
            "extension_pack_set_enabled" => {
                self.execute_extension_pack_set_enabled(arguments).await
            }
            "extension_pack_runtime_install" => {
                self.execute_extension_pack_runtime_install(arguments).await
            }
            "extension_pack_runtime_verify" => {
                self.execute_extension_pack_runtime_verify(arguments).await
            }
            "extension_pack_runtime_update" => {
                self.execute_extension_pack_runtime_update(arguments).await
            }
            "extension_pack_runtime_uninstall" => {
                self.execute_extension_pack_runtime_uninstall(arguments)
                    .await
            }
            "extension_pack_test_connection" => {
                self.execute_extension_pack_test_connection(arguments).await
            }
            "extension_pack_list_events" => {
                self.execute_extension_pack_list_events(arguments).await
            }
            "extension_pack_invoke" => self.execute_extension_pack_invoke(arguments).await,
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
                crate::actions::google_workspace::gws_help(&self.config_dir, arguments).await
            }
            "google_workspace_gws_schema" => {
                crate::actions::google_workspace::gws_schema(&self.config_dir, arguments).await
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

                let config = build_search_config(&self.config_dir, self.storage.as_ref()).await;
                let response =
                    crate::actions::search::execute_search_response(&args, &config).await?;
                let detail = crate::actions::search::format_search_results(&response);
                Ok(structured_tool_completion_output(
                    "web_search",
                    "completed",
                    detail,
                    serde_json::json!({
                        "query": response.query,
                        "backend": response.backend,
                        "results": response.results,
                    }),
                ))
            }
            "research" => {
                let args: crate::actions::research::ResearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid research arguments: {}", e))?;

                let config = build_search_config(&self.config_dir, self.storage.as_ref()).await;
                crate::actions::research::execute_research(&args, &config).await
            }
            "moltbook" => {
                let sub_action = arguments
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing Moltbook action"))?;
                let connector =
                    crate::integrations::moltbook::MoltbookConnector::new_with_config_dir(
                        self.config_dir.clone(),
                    );
                let result =
                    crate::integrations::Integration::execute(&connector, sub_action, arguments)
                        .await?;
                Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
            }
            "session_search" => self.execute_session_search(arguments).await,
            "document_lookup" => self.execute_document_lookup(arguments).await,
            "vision_ocr" => self.execute_vision_ocr(arguments).await,
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
                    .user_agent(crate::branding::user_agent_with_suffix(
                        "(AI Agent Browser)",
                    ))
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

                let content = match extract {
                    "title" => {
                        if title.is_empty() {
                            "(no title found)".to_string()
                        } else {
                            title.clone()
                        }
                    }
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
                            "(no links found)".to_string()
                        } else {
                            // Limit to 50 links to avoid overwhelming output
                            let display_links: Vec<&str> =
                                links.iter().take(50).map(|s| s.as_str()).collect();
                            format!(
                                "Found {} links (showing up to 50):\n{}",
                                links.len(),
                                display_links.join("\n")
                            )
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
                        format!(
                            "## Title\n{}\n\n## Content\n{}\n\n## Links\n{}",
                            if title.is_empty() {
                                "(no title)"
                            } else {
                                &title
                            },
                            text,
                            links_section
                        )
                    }
                    _ => {
                        // Default: extract text content
                        let text = Self::html_to_text(&html);
                        if text.is_empty() {
                            "(no text content extracted)".to_string()
                        } else {
                            text
                        }
                    }
                };
                Ok(structured_tool_completion_output(
                    "browse",
                    "completed",
                    browse_completion_detail(url, &title, extract, &content),
                    serde_json::json!({
                        "url": url,
                        "title": title,
                        "extract": extract,
                        "content": content,
                    }),
                ))
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

                let pdf_bytes = Self::generate_simple_pdf_bytes(title, content, style);
                tokio::fs::write(&output_path, pdf_bytes).await?;
                Ok(format!("PDF generated: {}", output_path.display()))
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
                                "- [{}] {} {} - {} ({}){}\n",
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
            "home_assistant" => self.execute_home_assistant(arguments).await,
            "home_assistant_call_service" => {
                self.execute_home_assistant_call_service(arguments).await
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
                        anyhow::anyhow!(
                            "Invalid timezone '{}'. Expected an IANA name such as Asia/Kolkata.",
                            timezone_name
                        )
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
                    .ok_or_else(|| {
                        crate::actions::structured_action_error(
                            ActionErrorDomain::Channel,
                            ActionErrorReason::MissingInput,
                            "notify_user requires a non-empty `message`",
                        )
                    })?;
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
            // ArkOrbit (per-user limitless canvas)
            "arkorbit_create_orbit" => {
                let service = self.arkorbit_service()?;
                let user_id = self.current_user_id()?.to_string();
                crate::actions::arkorbit::create_orbit(&service, &user_id, arguments).await
            }
            "arkorbit_file_write" => {
                let service = self.arkorbit_service()?;
                crate::actions::arkorbit::orbit_file_write(&service, arguments).await
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
                            let sensitive_required =
                                Self::collect_sensitive_required_fields_from_schema(
                                    &action.info.input_schema,
                                );
                            let missing: Vec<String> = required
                                .iter()
                                .filter(|k| !Self::has_non_empty_argument(arguments, k))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                let sensitive_missing = missing
                                    .iter()
                                    .filter(|key| {
                                        sensitive_required.iter().any(|required| required == *key)
                                    })
                                    .cloned()
                                    .collect();
                                let payload = WorkflowMissingInputsPayload {
                                    action: other.to_string(),
                                    missing,
                                    sensitive_missing,
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
                Err(crate::actions::structured_action_error(
                    ActionErrorDomain::Action,
                    ActionErrorReason::NotFound,
                    format!("Unknown native action: {}", action_name),
                ))
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

    fn companion_device_is_connected(state: &crate::core::CompanionDeviceState) -> bool {
        matches!(
            state,
            crate::core::CompanionDeviceState::Online
                | crate::core::CompanionDeviceState::Idle
                | crate::core::CompanionDeviceState::Busy
        )
    }

    fn connected_surface_item(
        surface: &str,
        id: impl Into<String>,
        name: impl Into<String>,
        kind: impl Into<String>,
        status: impl Into<String>,
    ) -> serde_json::Value {
        serde_json::json!({
            "surface": surface,
            "id": id.into(),
            "name": name.into(),
            "kind": kind.into(),
            "status": status.into(),
        })
    }

    fn integration_inventory_section_counts(value: &serde_json::Value) -> serde_json::Value {
        fn array_len(value: &serde_json::Value, key: &str) -> usize {
            value
                .get(key)
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or(0)
        }

        serde_json::json!({
            "builtin_integrations": array_len(value, "integrations"),
            "gateway_channels": array_len(value, "channels"),
            "notification_channels": array_len(value, "channels"),
            "custom_apis": array_len(value, "custom_apis"),
            "webhook_sources": array_len(value, "sources"),
            "companion_devices": array_len(value, "devices"),
        })
    }

    async fn companion_device_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let plane = crate::core::CompanionControlPlane::new(storage);
        let devices = match plane.list_devices().await {
            Ok(devices) => devices,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let overview = match plane.overview().await {
            Ok(overview) => Some(overview),
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let total = devices.len();
        let connected_total = devices
            .iter()
            .filter(|device| Self::companion_device_is_connected(&device.state))
            .count();
        let mut connected_items = Vec::new();
        let visible_devices = devices
            .into_iter()
            .filter(|device| !only_connected || Self::companion_device_is_connected(&device.state))
            .map(|device| {
                let connected = Self::companion_device_is_connected(&device.state);
                if connected {
                    connected_items.push(Self::connected_surface_item(
                        "companion_devices",
                        device.id.clone(),
                        device.display_name.clone(),
                        device.platform.clone(),
                        format!("{:?}", device.state).to_ascii_lowercase(),
                    ));
                }
                serde_json::json!({
                    "id": device.id,
                    "display_name": device.display_name,
                    "preset_id": device.preset_id,
                    "platform": device.platform,
                    "model": device.model,
                    "state": device.state,
                    "connected": connected,
                    "transport": device.transport,
                    "available_capabilities": device.available_capabilities,
                    "granted_capabilities": device.granted_capabilities,
                    "token_capabilities": device.token_capabilities,
                    "paired_at": device.paired_at,
                    "last_seen_at": device.last_seen_at,
                    "owner": device.owner,
                    "command_count": device.command_count,
                    "attestation": {
                        "verified": device.attestation.verified,
                        "provider": device.attestation.provider,
                        "platform": device.attestation.platform,
                        "verified_at": device.attestation.verified_at,
                        "reason": device.attestation.reason,
                    },
                    "trusted_unattested": device.trusted_unattested,
                })
            })
            .collect::<Vec<_>>();
        (
            serde_json::json!({
                "available": true,
                "surface": "companion_devices",
                "overview": overview,
                "total": total,
                "connected_total": connected_total,
                "filtered_to_connected": only_connected,
                "devices": visible_devices,
            }),
            connected_items,
        )
    }

    fn integration_status_label(status: &crate::integrations::IntegrationStatus) -> String {
        match status {
            crate::integrations::IntegrationStatus::NotConfigured => "not_configured".to_string(),
            crate::integrations::IntegrationStatus::NeedsAuth => "needs_auth".to_string(),
            crate::integrations::IntegrationStatus::Connected => "connected".to_string(),
            crate::integrations::IntegrationStatus::Error(_) => "error".to_string(),
        }
    }

    async fn builtin_integrations_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
        let mut rows = Vec::new();
        let mut connected_items = Vec::new();
        for info in manager.list().await {
            let enabled = manager.is_enabled(&info.id);
            let connected = enabled
                && matches!(
                    info.status,
                    crate::integrations::IntegrationStatus::Connected
                );
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "integrations",
                    info.id.clone(),
                    info.name.clone(),
                    "builtin",
                    "connected",
                ));
            }
            if only_connected && !connected {
                continue;
            }
            rows.push(serde_json::json!({
                "id": info.id,
                "name": info.name,
                "description": info.description,
                "icon": info.icon,
                "capabilities": info.capabilities,
                "status": info.status,
                "status_label": Self::integration_status_label(&info.status),
                "enabled_for_agent": enabled,
                "connected": connected,
            }));
        }
        let total = rows.len();
        (
            serde_json::json!({
                "available": true,
                "surface": "builtin_integrations",
                "filtered_to_connected": only_connected,
                "visible_total": total,
                "connected_total": connected_items.len(),
                "integrations": rows,
            }),
            connected_items,
        )
    }

    fn email_notification_configured_from_config(&self, config: &crate::core::AgentConfig) -> bool {
        let mut backends = Vec::new();
        if crate::integrations::effective_integration_enabled(&self.config_dir, "gmail")
            && self
                .settings_manager()
                .ok()
                .and_then(|manager| manager.get_custom_secret("gmail_tokens").ok().flatten())
                .is_some_and(|value| !value.trim().is_empty())
        {
            backends.push(crate::core::email_delivery::EMAIL_PROVIDER_GMAIL.to_string());
        }
        if crate::integrations::effective_integration_enabled(&self.config_dir, "google_workspace")
            && crate::actions::google_workspace::granted_bundles(&self.config_dir)
                .map(|bundles| bundles.iter().any(|bundle| bundle == "gmail"))
                .unwrap_or(false)
        {
            backends.push(crate::core::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE.to_string());
        }
        if crate::core::email_delivery::external_email_delivery_is_ready(&config.email) {
            if let Some(provider_id) =
                crate::core::email_delivery::external_email_provider_id(&config.email)
            {
                if !backends.iter().any(|existing| existing == &provider_id) {
                    backends.push(provider_id);
                }
            }
        }
        crate::core::email_delivery::email_channel_is_ready(&config.email.provider, &backends)
    }

    async fn gateway_channels_inventory(
        &self,
        only_connected: bool,
    ) -> (
        serde_json::Value,
        Vec<serde_json::Value>,
        BTreeMap<String, bool>,
    ) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
                BTreeMap::new(),
            );
        };
        let config = match self.settings_manager().and_then(|manager| manager.load()) {
            Ok(config) => config,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                    BTreeMap::new(),
                );
            }
        };
        let payload = match crate::core::load_gateway_channels(&storage, &config).await {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                    BTreeMap::new(),
                );
            }
        };
        let mut configured = BTreeMap::new();
        let mut connected_items = Vec::new();
        let mut channels = Vec::new();
        for channel in payload.channels {
            let connected = channel.enabled
                && (matches!(
                    channel.status.as_str(),
                    "connected" | "ready" | "configured"
                ) || channel.connected_account_count > 0);
            configured.insert(channel.id.clone(), channel.configured || connected);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "messaging_channels",
                    channel.id.clone(),
                    channel.name.clone(),
                    channel.kind.clone(),
                    channel.status.clone(),
                ));
            }
            if only_connected && !connected {
                continue;
            }
            let mut value = serde_json::to_value(channel).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("connected".to_string(), serde_json::json!(connected));
            }
            channels.push(value);
        }
        let accounts = if only_connected {
            payload
                .accounts
                .into_iter()
                .filter(|account| {
                    account.enabled
                        && matches!(
                            account.status.trim().to_ascii_lowercase().as_str(),
                            "connected" | "ready" | "syncing"
                        )
                })
                .collect::<Vec<_>>()
        } else {
            payload.accounts
        };
        (
            serde_json::json!({
                "available": true,
                "surface": "gateway_channels",
                "summary": payload.summary,
                "filtered_to_connected": only_connected,
                "channels": channels,
                "accounts": accounts,
            }),
            connected_items,
            configured,
        )
    }

    async fn messaging_channels_inventory(
        &self,
        only_connected: bool,
        bundled_configured: &BTreeMap<String, bool>,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let Some(registry) = self.extension_pack_registry.clone() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "Extension-pack registry is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let config = self
            .settings_manager()
            .and_then(|manager| manager.load())
            .ok();
        let email_configured = config
            .as_ref()
            .is_some_and(|config| self.email_notification_configured_from_config(config));
        let config_manager = self.settings_manager().ok();
        let packs_guard = registry.read().await;
        let bundled_check = |channel_id: &str| -> bool {
            let normalized = channel_id.trim().to_ascii_lowercase();
            if normalized == "email" {
                return email_configured;
            }
            bundled_configured
                .get(&normalized)
                .copied()
                .unwrap_or(false)
        };
        let ctx = crate::channels::messaging_registry::ChannelQueryContext {
            bundled_configured: &bundled_check,
            extension_packs: &*packs_guard,
            storage: &storage,
            config_dir: &self.config_dir,
            data_dir: self.data_dir(),
            config_manager: config_manager.as_ref(),
        };
        let descriptors = match crate::channels::messaging_registry::MessagingChannelRegistry::new()
            .list(&ctx)
            .await
        {
            Ok(descriptors) => descriptors,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let mut connected_items = Vec::new();
        let mut channels = Vec::new();
        for descriptor in descriptors {
            if descriptor.configured {
                connected_items.push(Self::connected_surface_item(
                    "notification_channels",
                    descriptor.id.clone(),
                    descriptor.display_name.clone(),
                    match &descriptor.source {
                        crate::channels::messaging_registry::ChannelSource::Bundled => "bundled",
                        crate::channels::messaging_registry::ChannelSource::ExtensionPack { .. } => {
                            "extension_pack"
                        }
                        crate::channels::messaging_registry::ChannelSource::CustomMessagingChannel {
                            ..
                        } => "custom_messaging_channel",
                    },
                    "configured",
                ));
            }
            if only_connected && !descriptor.configured {
                continue;
            }
            let mut value = serde_json::to_value(descriptor).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "connected".to_string(),
                    object
                        .get("configured")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!(false)),
                );
            }
            channels.push(value);
        }
        (
            serde_json::json!({
                "available": true,
                "surface": "notification_channels",
                "filtered_to_connected": only_connected,
                "connected_total": connected_items.len(),
                "channels": channels,
            }),
            connected_items,
        )
    }

    async fn custom_apis_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let apis =
            match crate::custom_apis::list_custom_apis(&storage, &self.config_dir, self.data_dir())
                .await
            {
                Ok(apis) => apis,
                Err(error) => {
                    return (
                        serde_json::json!({
                            "available": false,
                            "error": error.to_string()
                        }),
                        Vec::new(),
                    );
                }
            };
        let total = apis.len();
        let mut rows = Vec::new();
        let mut connected_items = Vec::new();
        for api in apis {
            let connected = api.config.enabled
                && api.action_count > 0
                && (matches!(
                    api.config.auth_mode,
                    crate::custom_apis::CustomApiAuthMode::None
                ) || api.secret_configured);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "custom_apis",
                    api.config.id.clone(),
                    api.config.name.clone(),
                    "custom_api",
                    "connected",
                ));
            }
            if only_connected && !connected {
                continue;
            }
            let mut value = serde_json::to_value(api).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("connected".to_string(), serde_json::json!(connected));
            }
            rows.push(value);
        }
        (
            serde_json::json!({
                "available": true,
                "surface": "custom_apis",
                "total": total,
                "connected_total": connected_items.len(),
                "filtered_to_connected": only_connected,
                "custom_apis": rows,
            }),
            connected_items,
        )
    }

    async fn webhook_sources_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let payload = match crate::channels::http::webhooks::list_webhook_source_inventory(
            &storage,
            &self.config_dir,
            self.data_dir(),
            only_connected,
        )
        .await
        {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let connected_items = payload
            .get("sources")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter(|source| {
                source
                    .get("connected")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .map(|source| {
                Self::connected_surface_item(
                    "webhook_sources",
                    source
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                    source
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                    source
                        .get("provider")
                        .and_then(|value| value.as_str())
                        .unwrap_or("webhook"),
                    "connected",
                )
            })
            .collect::<Vec<_>>();
        (payload, connected_items)
    }

    fn plugin_connected(plugin: &crate::plugins::registry::PluginView) -> bool {
        plugin.plugin.enabled
            && plugin.plugin.last_error.is_none()
            && (matches!(
                plugin.plugin.auth_mode,
                crate::plugins::registry::PluginAuthMode::None
            ) || plugin.token_configured)
    }

    async fn plugins_inventory(
        &self,
        only_connected: bool,
    ) -> (
        Option<Vec<crate::plugins::registry::PluginView>>,
        Vec<serde_json::Value>,
    ) {
        let Some(registry) = self.plugin_registry.clone() else {
            return (None, Vec::new());
        };
        let guard = registry.read().await;
        let plugins = match guard.list_plugins().await {
            Ok(plugins) => plugins,
            Err(_) => return (None, Vec::new()),
        };
        let mut visible = Vec::new();
        let mut connected_items = Vec::new();
        for plugin in plugins {
            let connected = Self::plugin_connected(&plugin);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "plugins",
                    plugin.plugin.id.clone(),
                    plugin.plugin.name.clone(),
                    "plugin",
                    "connected",
                ));
            }
            if !only_connected || connected {
                visible.push(plugin);
            }
        }
        (Some(visible), connected_items)
    }

    fn mcp_server_connected(server: &crate::mcp::registry::McpServerView) -> bool {
        server.enabled
            && server.last_error.is_none()
            && (server.tool_count > 0 || (server.resources_enabled && server.resource_count > 0))
    }

    async fn mcp_servers_inventory(
        &self,
        only_connected: bool,
    ) -> (
        Option<Vec<crate::mcp::registry::McpServerView>>,
        Vec<serde_json::Value>,
    ) {
        let Some(registry) = self.mcp_registry.clone() else {
            return (None, Vec::new());
        };
        let guard = registry.read().await;
        let servers = match guard.list_servers(false).await {
            Ok(servers) => servers,
            Err(_) => return (None, Vec::new()),
        };
        let mut visible = Vec::new();
        let mut connected_items = Vec::new();
        for server in servers {
            let connected = Self::mcp_server_connected(&server);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "mcp_servers",
                    server.id.clone(),
                    server.name.clone(),
                    "mcp_server",
                    "connected",
                ));
            }
            if !only_connected || connected {
                visible.push(server);
            }
        }
        (Some(visible), connected_items)
    }

    async fn execute_list_integrations(&self, arguments: &serde_json::Value) -> Result<String> {
        let query = arguments.get("query").and_then(|value| value.as_str());
        let kind = arguments.get("kind").and_then(|value| value.as_str());
        let only_connected = arguments
            .get("only_connected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let include_details = arguments
            .get("include_details")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let packs = if let Some(registry) = self.extension_pack_registry.clone() {
            let guard = registry.read().await;
            Some(guard.search_packs(query, kind).await?)
        } else {
            None
        };
        let mut connected_items = Vec::new();
        if let Some(packs) = packs.as_ref() {
            for pack in &packs.installed {
                let connected =
                    pack.enabled && matches!(pack.status.as_str(), "ready" | "connected");
                if connected && (!only_connected || connected) {
                    connected_items.push(Self::connected_surface_item(
                        "extension_packs",
                        pack.manifest.id.clone(),
                        pack.manifest.name.clone(),
                        "extension_pack",
                        pack.status.clone(),
                    ));
                }
            }
        }
        let (plugins, plugin_connected) = self.plugins_inventory(only_connected).await;
        connected_items.extend(plugin_connected);
        let (mcp_servers, mcp_connected) = self.mcp_servers_inventory(only_connected).await;
        connected_items.extend(mcp_connected);
        let (builtin_integrations, builtin_connected) =
            self.builtin_integrations_inventory(only_connected).await;
        connected_items.extend(builtin_connected);
        let (gateway_channels, gateway_connected, bundled_configured) =
            self.gateway_channels_inventory(only_connected).await;
        connected_items.extend(gateway_connected);
        let (messaging_channels, messaging_connected) = self
            .messaging_channels_inventory(only_connected, &bundled_configured)
            .await;
        connected_items.extend(messaging_connected);
        let (custom_apis, custom_api_connected) = self.custom_apis_inventory(only_connected).await;
        connected_items.extend(custom_api_connected);
        let (webhook_sources, webhook_connected) =
            self.webhook_sources_inventory(only_connected).await;
        connected_items.extend(webhook_connected);
        let (companion_devices, companion_connected) =
            self.companion_device_inventory(only_connected).await;
        connected_items.extend(companion_connected);
        connected_items.sort_by(|left, right| {
            let left_key = format!(
                "{}:{}",
                left.get("surface")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                left.get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            let right_key = format!(
                "{}:{}",
                right
                    .get("surface")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                right
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            left_key.cmp(&right_key)
        });
        let mut section_counts = serde_json::Map::new();
        section_counts.insert(
            "builtin_integrations".to_string(),
            Self::integration_inventory_section_counts(&builtin_integrations)
                .get("builtin_integrations")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "gateway_channels".to_string(),
            Self::integration_inventory_section_counts(&gateway_channels)
                .get("gateway_channels")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "notification_channels".to_string(),
            Self::integration_inventory_section_counts(&messaging_channels)
                .get("notification_channels")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "custom_apis".to_string(),
            Self::integration_inventory_section_counts(&custom_apis)
                .get("custom_apis")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "webhook_sources".to_string(),
            Self::integration_inventory_section_counts(&webhook_sources)
                .get("webhook_sources")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "companion_devices".to_string(),
            Self::integration_inventory_section_counts(&companion_devices)
                .get("companion_devices")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "extension_packs_installed".to_string(),
            serde_json::json!(packs
                .as_ref()
                .map(|packs| packs.installed.len())
                .unwrap_or_default()),
        );
        section_counts.insert(
            "plugins".to_string(),
            serde_json::json!(plugins.as_ref().map(Vec::len).unwrap_or_default()),
        );
        section_counts.insert(
            "mcp_servers".to_string(),
            serde_json::json!(mcp_servers.as_ref().map(Vec::len).unwrap_or_default()),
        );

        let mut payload = serde_json::json!({
            "connected_agentark_surfaces": {
                "total": connected_items.len(),
                "items": connected_items,
            },
            "section_counts": section_counts,
            "detail_available_via": "inspect_integration",
        });
        if include_details {
            if let Some(object) = payload.as_object_mut() {
                object.insert("builtin_integrations".to_string(), builtin_integrations);
                object.insert("gateway_channels".to_string(), gateway_channels);
                object.insert("notification_channels".to_string(), messaging_channels);
                object.insert("custom_apis".to_string(), custom_apis);
                object.insert("webhook_sources".to_string(), webhook_sources);
                object.insert("companion_devices".to_string(), companion_devices);
                object.insert("extension_packs".to_string(), serde_json::to_value(packs)?);
                object.insert("plugins".to_string(), serde_json::to_value(plugins)?);
                object.insert(
                    "mcp_servers".to_string(),
                    serde_json::to_value(mcp_servers)?,
                );
            }
        }
        Ok(serde_json::to_string_pretty(&payload)?)
    }

    fn integration_inspect_terms(value: Option<&str>) -> Vec<String> {
        value
            .unwrap_or_default()
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    fn integration_value_text(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
            serde_json::Value::String(text) => {
                out.push(' ');
                out.push_str(&text.to_ascii_lowercase());
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::integration_value_text(item, out);
                }
            }
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    out.push(' ');
                    out.push_str(&key.to_ascii_lowercase());
                    Self::integration_value_text(item, out);
                }
            }
        }
    }

    fn integration_value_matches(
        value: &serde_json::Value,
        id: Option<&str>,
        query_terms: &[String],
    ) -> bool {
        if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) {
            let id_lower = id.to_ascii_lowercase();
            if let Some(map) = value.as_object() {
                for key in [
                    "id",
                    "name",
                    "display_name",
                    "runtime_channel_id",
                    "channel_id",
                    "pack_id",
                ] {
                    if map
                        .get(key)
                        .and_then(|item| item.as_str())
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(id))
                    {
                        return true;
                    }
                }
                if let Some(manifest) = map.get("manifest").and_then(|item| item.as_object()) {
                    for key in ["id", "name"] {
                        if manifest
                            .get(key)
                            .and_then(|item| item.as_str())
                            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(id))
                        {
                            return true;
                        }
                    }
                }
                if let Some(connection) = map.get("connection").and_then(|item| item.as_object()) {
                    for key in ["id", "name", "pack_id"] {
                        if connection
                            .get(key)
                            .and_then(|item| item.as_str())
                            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(id))
                        {
                            return true;
                        }
                    }
                }
            }
            let mut text = String::new();
            Self::integration_value_text(value, &mut text);
            if text.split_whitespace().any(|part| part == id_lower) {
                return true;
            }
        }

        if query_terms.is_empty() {
            return false;
        }
        let mut text = String::new();
        Self::integration_value_text(value, &mut text);
        query_terms.iter().all(|term| text.contains(term))
    }

    fn integration_find_matches(
        items: impl IntoIterator<Item = serde_json::Value>,
        id: Option<&str>,
        query_terms: &[String],
        limit: usize,
    ) -> Vec<serde_json::Value> {
        items
            .into_iter()
            .filter(|item| Self::integration_value_matches(item, id, query_terms))
            .take(limit)
            .collect()
    }

    fn requested_surface_matches(requested: Option<&str>, candidates: &[&str]) -> bool {
        let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
            return true;
        };
        candidates
            .iter()
            .any(|candidate| requested.eq_ignore_ascii_case(candidate))
    }

    async fn execute_inspect_integration(&self, arguments: &serde_json::Value) -> Result<String> {
        let surface = arguments
            .get("surface")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let id = arguments
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let run_check = arguments
            .get("run_check")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if id.is_none() && query.is_none() {
            anyhow::bail!("inspect_integration requires an id or query");
        }
        let query_terms = Self::integration_inspect_terms(query);
        let mut matches = Vec::new();
        let mut checks = Vec::new();

        if Self::requested_surface_matches(surface, &["companion_devices", "companion_device"]) {
            let (payload, _) = self.companion_device_inventory(false).await;
            let devices = payload
                .get("devices")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_find_matches(devices, id, &query_terms, 8) {
                matches.push(serde_json::json!({
                    "surface": "companion_devices",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "stored_websocket_presence",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                        "state": record.get("state"),
                        "last_seen_at": record.get("last_seen_at"),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["integrations", "builtin_integrations"]) {
            let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
            for info in manager.list().await {
                let value = serde_json::json!({
                    "id": info.id,
                    "name": info.name,
                    "description": info.description,
                    "icon": info.icon,
                    "capabilities": info.capabilities,
                    "status": info.status,
                    "status_label": Self::integration_status_label(&info.status),
                    "enabled_for_agent": manager.is_enabled(&info.id),
                });
                if !Self::integration_value_matches(&value, id, &query_terms) {
                    continue;
                }
                let safe_check = if run_check {
                    Some(serde_json::json!({
                        "ran": true,
                        "kind": "readiness_status",
                        "ready_for_agent": manager.is_ready(&info.id).await,
                    }))
                } else {
                    None
                };
                matches.push(serde_json::json!({
                    "surface": "integrations",
                    "record": value,
                    "safe_check": safe_check,
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["gateway_channels", "messaging_channels"]) {
            let (payload, _, _) = self.gateway_channels_inventory(false).await;
            let channels = payload
                .get("channels")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_find_matches(channels, id, &query_terms, 8) {
                matches.push(serde_json::json!({
                    "surface": "gateway_channels",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "stored_channel_status",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                        "status": record.get("status"),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["notification_channels"]) {
            let (_, _, bundled_configured) = self.gateway_channels_inventory(false).await;
            let (payload, _) = self
                .messaging_channels_inventory(false, &bundled_configured)
                .await;
            let channels = payload
                .get("channels")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_find_matches(channels, id, &query_terms, 8) {
                matches.push(serde_json::json!({
                    "surface": "notification_channels",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "configured_state",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["custom_apis", "custom_api"]) {
            let (payload, _) = self.custom_apis_inventory(false).await;
            let apis = payload
                .get("custom_apis")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_find_matches(apis, id, &query_terms, 8) {
                let mut safe_check = serde_json::json!({
                    "ran": false,
                    "kind": "custom_api_test",
                    "reason": "run_check was false",
                });
                if run_check {
                    if let (Some(storage), Some(api_id)) = (
                        self.storage(),
                        record.get("id").and_then(|value| value.as_str()),
                    ) {
                        safe_check = match Box::pin(crate::custom_apis::test_custom_api(
                            &storage,
                            &self.config_dir,
                            self.data_dir(),
                            self,
                            api_id,
                        ))
                        .await
                        {
                            Ok(result) => serde_json::json!({
                                "ran": true,
                                "kind": "custom_api_test",
                                "ok": result.ok,
                                "action_name": result.action_name,
                                "detail": result.detail,
                            }),
                            Err(error) => serde_json::json!({
                                "ran": true,
                                "kind": "custom_api_test",
                                "ok": false,
                                "error": error.to_string(),
                            }),
                        };
                    }
                }
                matches.push(serde_json::json!({
                    "surface": "custom_apis",
                    "record": record,
                    "safe_check": safe_check,
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["webhook_sources", "webhooks"]) {
            let (payload, _) = self.webhook_sources_inventory(false).await;
            let sources = payload
                .get("sources")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_find_matches(sources, id, &query_terms, 8) {
                matches.push(serde_json::json!({
                    "surface": "webhook_sources",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "stored_secret_and_enabled_state",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                        "secret_configured": record.get("secret_configured"),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["extension_packs", "extension_pack"]) {
            if let Some(registry) = self.extension_pack_registry.clone() {
                let guard = registry.read().await;
                let packs = guard.search_packs(None, None).await?;
                let installed = packs
                    .installed
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for record in Self::integration_find_matches(installed, id, &query_terms, 8) {
                    let pack_id = record
                        .get("manifest")
                        .and_then(|manifest| manifest.get("id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    let connections = guard
                        .list_connections(pack_id)
                        .await
                        .ok()
                        .and_then(|value| serde_json::to_value(value).ok())
                        .unwrap_or_else(|| serde_json::json!([]));
                    let events = guard
                        .list_events(pack_id, 10)
                        .await
                        .ok()
                        .and_then(|value| serde_json::to_value(value).ok());
                    matches.push(serde_json::json!({
                        "surface": "extension_packs",
                        "record": record,
                        "connections": connections,
                        "recent_events": events,
                        "safe_check": {
                            "ran": true,
                            "kind": "connection_state",
                        }
                    }));
                }
            }
        }

        if Self::requested_surface_matches(surface, &["plugins"]) {
            if let Some(registry) = self.plugin_registry.clone() {
                let guard = registry.read().await;
                let plugins = guard
                    .list_plugins()
                    .await?
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for record in Self::integration_find_matches(plugins, id, &query_terms, 8) {
                    matches.push(serde_json::json!({
                        "surface": "plugins",
                        "record": record,
                        "safe_check": {
                            "ran": true,
                            "kind": "stored_plugin_status",
                            "connected": record.get("enabled").and_then(|value| value.as_bool()).unwrap_or(false)
                                && record.get("last_error").is_none(),
                        }
                    }));
                }
            }
        }

        if Self::requested_surface_matches(surface, &["mcp_servers", "mcp"]) {
            if let Some(registry) = self.mcp_registry.clone() {
                let guard = registry.read().await;
                let servers = guard
                    .list_servers(true)
                    .await?
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for record in Self::integration_find_matches(servers, id, &query_terms, 8) {
                    matches.push(serde_json::json!({
                        "surface": "mcp_servers",
                        "record": record,
                        "safe_check": {
                            "ran": true,
                            "kind": "registered_tool_resource_state",
                            "connected": record.get("enabled").and_then(|value| value.as_bool()).unwrap_or(false)
                                && record.get("last_error").is_none(),
                        }
                    }));
                }
            }
        }

        checks.push(serde_json::json!({
            "run_check_requested": run_check,
            "matches_returned": matches.len(),
            "truncation_avoidance": "list_integrations returns compact overview by default; this action returns targeted detail.",
        }));

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": if matches.is_empty() { "not_found" } else { "ok" },
            "surface": surface,
            "id": id,
            "query": query,
            "matches": matches,
            "diagnostics": checks,
        }))?)
    }

    async fn execute_home_assistant(&self, arguments: &serde_json::Value) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("home_assistant requires an operation"))?;
        if !matches!(
            operation,
            "list_entities" | "search_entities" | "get_state" | "get_services"
        ) {
            anyhow::bail!("home_assistant only supports read-only operations");
        }
        let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
        let result = manager
            .execute("home_assistant", operation, arguments)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_home_assistant_call_service(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
        let result = manager
            .execute("home_assistant", "call_service", arguments)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    fn session_search_terms(query: Option<&str>) -> Vec<String> {
        query
            .unwrap_or_default()
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    fn session_search_score(text: &str, terms: &[String]) -> usize {
        if terms.is_empty() {
            return 1;
        }
        let haystack = text.to_ascii_lowercase();
        terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count()
    }

    fn session_search_snippet(text: &str, max_chars: usize) -> String {
        let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() <= max_chars {
            compact
        } else {
            format!(
                "{}...",
                compact
                    .chars()
                    .take(max_chars.saturating_sub(3))
                    .collect::<String>()
            )
        }
    }

    async fn execute_session_search(&self, arguments: &serde_json::Value) -> Result<String> {
        let storage = self.runtime_storage()?;
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let terms = Self::session_search_terms(query);
        let scope = arguments
            .get("scope")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("all");
        if !matches!(scope, "all" | "conversations" | "messages" | "traces") {
            anyhow::bail!("session_search scope must be all, conversations, messages, or traces");
        }
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(8)
            .clamp(1, 25);
        let conversation_id = arguments
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let scan_limit = (limit * 12).clamp(40, 300);
        let mut hits: Vec<(usize, String, serde_json::Value)> = Vec::new();

        if matches!(scope, "all" | "conversations") {
            if let Some(conversation_id) = conversation_id {
                if let Some(conversation) = storage.get_conversation(conversation_id).await? {
                    let score = Self::session_search_score(&conversation.title, &terms);
                    hits.push((
                        score,
                        conversation.updated_at.clone(),
                        serde_json::json!({
                            "type": "conversation",
                            "id": conversation.id,
                            "title": conversation.title,
                            "channel": conversation.channel,
                            "message_count": conversation.message_count,
                            "updated_at": conversation.updated_at,
                            "match_score": score,
                        }),
                    ));
                }
            } else {
                for conversation in storage
                    .list_conversations(scan_limit, 0, None, &[], None)
                    .await?
                {
                    let text = format!("{} {}", conversation.title, conversation.channel);
                    let score = Self::session_search_score(&text, &terms);
                    hits.push((
                        score,
                        conversation.updated_at.clone(),
                        serde_json::json!({
                            "type": "conversation",
                            "id": conversation.id,
                            "title": conversation.title,
                            "channel": conversation.channel,
                            "message_count": conversation.message_count,
                            "updated_at": conversation.updated_at,
                            "match_score": score,
                        }),
                    ));
                }
            }
        }

        if matches!(scope, "all" | "messages") {
            let messages = if let Some(conversation_id) = conversation_id {
                storage
                    .get_recent_messages(conversation_id, scan_limit)
                    .await?
            } else {
                storage
                    .get_recent_messages_across_conversations(scan_limit)
                    .await?
            };
            for message in messages {
                let score = Self::session_search_score(&message.content, &terms);
                hits.push((
                    score,
                    message.timestamp.clone(),
                    serde_json::json!({
                        "type": "message",
                        "id": message.id,
                        "conversation_id": message.conversation_id,
                        "role": message.role,
                        "timestamp": message.timestamp,
                        "trace_id": message.trace_id,
                        "snippet": Self::session_search_snippet(&message.content, 420),
                        "match_score": score,
                    }),
                ));
            }
        }

        if matches!(scope, "all" | "traces") && conversation_id.is_none() {
            for trace in storage
                .list_execution_trace_summaries(None, scan_limit, 0)
                .await?
            {
                let text = format!(
                    "{} {} {}",
                    trace.message,
                    trace.steps_json,
                    trace.model.clone().unwrap_or_default()
                );
                let score = Self::session_search_score(&text, &terms);
                hits.push((
                    score,
                    trace.created_at.clone(),
                    serde_json::json!({
                        "type": "trace",
                        "id": trace.id,
                        "message": Self::session_search_snippet(&trace.message, 300),
                        "channel": trace.channel,
                        "started_at": trace.started_at,
                        "completed_at": trace.completed_at,
                        "duration_ms": trace.duration_ms,
                        "step_count": trace.step_count,
                        "total_tokens": trace.total_tokens,
                        "cost_usd": trace.cost_usd,
                        "complexity": trace.complexity,
                        "created_at": trace.created_at,
                        "match_score": score,
                    }),
                ));
            }
        }

        hits.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let results = hits
            .into_iter()
            .take(limit as usize)
            .map(|(_, _, payload)| payload)
            .collect::<Vec<_>>();

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query.unwrap_or(""),
            "scope": scope,
            "conversation_id": conversation_id,
            "result_count": results.len(),
            "results": results,
        }))?)
    }

    fn media_provider_secret(
        config: &AgentConfig,
        provider: crate::integrations::media_gen::MediaProvider,
    ) -> Option<String> {
        let explicit = config
            .media_gen
            .provider_api_keys
            .iter()
            .find_map(|(raw_provider, key)| {
                let configured =
                    crate::integrations::media_gen::MediaProvider::parse(raw_provider)?;
                let trimmed = key.trim();
                if configured == provider && !trimmed.is_empty() && trimmed != "[ENCRYPTED]" {
                    Some(trimmed.to_string())
                } else {
                    None
                }
            });
        if explicit.is_some() {
            return explicit;
        }
        if provider == crate::integrations::media_gen::MediaProvider::OpenAiDalle {
            if let Some(key) = config.model_pool.slots.iter().find_map(|slot| {
                let crate::core::LlmProvider::OpenAI {
                    api_key, base_url, ..
                } = &slot.provider
                else {
                    return None;
                };
                if !slot.enabled
                    || api_key.trim().is_empty()
                    || api_key.trim() == "[ENCRYPTED]"
                    || crate::core::llm_provider::openai_provider_label(base_url.as_deref())
                        != "openai"
                {
                    return None;
                }
                Some(api_key.trim().to_string())
            }) {
                return Some(key);
            }
            if let crate::core::LlmProvider::OpenAI {
                api_key, base_url, ..
            } = &config.llm
            {
                if !api_key.trim().is_empty()
                    && api_key.trim() != "[ENCRYPTED]"
                    && crate::core::llm_provider::openai_provider_label(base_url.as_deref())
                        == "openai"
                {
                    return Some(api_key.trim().to_string());
                }
            }
        }
        None
    }

    fn media_provider_base_url(
        config: &AgentConfig,
        provider: crate::integrations::media_gen::MediaProvider,
    ) -> String {
        let explicit =
            config
                .media_gen
                .provider_base_urls
                .iter()
                .find_map(|(raw_provider, base_url)| {
                    let configured =
                        crate::integrations::media_gen::MediaProvider::parse(raw_provider)?;
                    if configured == provider {
                        let trimmed = base_url.trim().trim_end_matches('/');
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                    None
                });
        if let Some(base_url) = explicit {
            return base_url;
        }
        if provider == crate::integrations::media_gen::MediaProvider::OpenAiDalle {
            if let Some(base_url) = config.model_pool.slots.iter().find_map(|slot| {
                let crate::core::LlmProvider::OpenAI { base_url, .. } = &slot.provider else {
                    return None;
                };
                if !slot.enabled
                    || crate::core::llm_provider::openai_provider_label(base_url.as_deref())
                        != "openai"
                {
                    return None;
                }
                base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.trim_end_matches('/').to_string())
            }) {
                return base_url;
            }
            if let crate::core::LlmProvider::OpenAI { base_url, .. } = &config.llm {
                if crate::core::llm_provider::openai_provider_label(base_url.as_deref()) == "openai"
                {
                    if let Some(base_url) = base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.trim_end_matches('/').to_string())
                    {
                        return base_url;
                    }
                }
            }
        }
        provider.default_base_url().to_string()
    }

    fn select_vision_provider(
        config: &AgentConfig,
        requested: Option<&str>,
    ) -> Result<crate::integrations::media_gen::MediaProvider> {
        use crate::integrations::media_gen::MediaProvider;

        let supports_vision = |provider: MediaProvider| {
            matches!(
                provider,
                MediaProvider::OpenAiDalle | MediaProvider::GoogleGemini
            )
        };

        if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
            let provider = MediaProvider::parse(requested)
                .ok_or_else(|| anyhow::anyhow!("Unknown vision provider '{}'", requested))?;
            if !supports_vision(provider) {
                anyhow::bail!(
                    "Provider '{}' is not available for vision_ocr. Configure OpenAI Images or Google Gemini.",
                    provider.id()
                );
            }
            return Ok(provider);
        }

        if let Some(default_provider) = config
            .media_gen
            .default_image_provider
            .as_deref()
            .and_then(MediaProvider::parse)
        {
            if supports_vision(default_provider)
                && Self::media_provider_secret(config, default_provider).is_some()
            {
                return Ok(default_provider);
            }
        }

        [MediaProvider::OpenAiDalle, MediaProvider::GoogleGemini]
            .iter()
            .copied()
            .find(|provider| Self::media_provider_secret(config, *provider).is_some())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "vision_ocr needs a configured OpenAI Images or Google Gemini provider key in Settings > Media."
                )
            })
    }

    fn default_vision_model(
        provider: crate::integrations::media_gen::MediaProvider,
    ) -> &'static str {
        use crate::integrations::media_gen::MediaProvider;
        match provider {
            MediaProvider::OpenAiDalle => "gpt-4.1-mini",
            MediaProvider::GoogleGemini => "gemini-2.5-flash",
            _ => "gpt-4.1-mini",
        }
    }

    fn vision_instruction(task: &str, question: Option<&str>) -> Result<String> {
        let extra = question
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        let base = match task {
            "extract_text" => {
                "Extract all visible text from the image. Preserve reading order, line breaks, tables, labels, and important layout cues where possible."
            }
            "describe" => {
                "Describe the image accurately and concisely. Include objects, visible UI, scene context, and any notable text."
            }
            "answer_question" => {
                if extra.is_empty() {
                    anyhow::bail!("vision_ocr answer_question requires a question");
                }
                "Answer the user's question using only what can be seen in the image. Note uncertainty when the image is ambiguous."
            }
            "analyze_document" => {
                "Analyze the visual document. Extract text, identify structure, summarize key fields, and call out unclear or missing values."
            }
            other => anyhow::bail!("Unsupported vision_ocr task '{}'", other),
        };
        if extra.is_empty() {
            Ok(base.to_string())
        } else {
            Ok(format!(
                "{base}\n\nUser question or extra instructions: {extra}"
            ))
        }
    }

    fn normalized_vision_mime(
        filename: &str,
        content_type: Option<&str>,
        bytes: &[u8],
    ) -> Result<String> {
        let signature = Self::upload_signature(filename, content_type, bytes);
        let signature_mime = signature
            .get("mime")
            .and_then(|value| value.as_str())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let content_mime = content_type
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected = signature_mime
            .filter(|mime| mime.starts_with("image/"))
            .or_else(|| signature_mime.filter(|mime| *mime == "application/pdf"))
            .or_else(|| {
                content_mime
                    .filter(|mime| mime.starts_with("image/") || *mime == "application/pdf")
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "vision_ocr accepts image or PDF uploads/URLs. Detected payload is not a supported vision input."
                )
            })?;
        Ok(selected.to_ascii_lowercase())
    }

    fn vision_inline_max_bytes(mime_type: &str) -> usize {
        if mime_type == "application/pdf" {
            VISION_DOCUMENT_INLINE_MAX_BODY_BYTES
        } else {
            VISION_IMAGE_INLINE_MAX_BODY_BYTES
        }
    }

    async fn load_vision_input(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(String, String, String, Vec<u8>)> {
        let upload_id = arguments
            .get("upload_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let image_url = arguments
            .get("image_url")
            .and_then(|value| value.as_str())
            .or_else(|| arguments.get("file_url").and_then(|value| value.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty());

        match (upload_id, image_url) {
            (Some(_), Some(_)) => {
                anyhow::bail!("vision_ocr accepts either upload_id or a URL, not both")
            }
            (Some(upload_id), None) => {
                let file = self.resolve_upload_for_sandbox(upload_id).await?;
                let mime = Self::normalized_vision_mime(
                    &file.filename,
                    file.content_type.as_deref(),
                    &file.bytes,
                )?;
                let max_bytes = Self::vision_inline_max_bytes(&mime);
                if file.bytes.len() > max_bytes {
                    anyhow::bail!(
                        "vision_ocr input is too large: {} bytes (max {})",
                        file.bytes.len(),
                        max_bytes
                    );
                }
                Ok((
                    format!("upload:{}", file.filename),
                    file.filename,
                    mime,
                    file.bytes,
                ))
            }
            (None, Some(raw_url)) => {
                let url = self.validate_http_get_url(raw_url).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(45))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .user_agent(crate::branding::user_agent_with_suffix(
                        "(Vision OCR Fetch)",
                    ))
                    .build()
                    .map_err(|error| {
                        anyhow::anyhow!("Failed to build vision fetch client: {}", error)
                    })?;
                let response = client.get(url.clone()).send().await?;
                let status = response.status();
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    anyhow::bail!("vision_ocr input fetch returned {}: {}", status, body);
                }
                let bytes = response.bytes().await?;
                let filename = url
                    .path_segments()
                    .and_then(|segments| segments.last())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("image");
                let bytes = bytes.to_vec();
                let mime = Self::normalized_vision_mime(filename, content_type.as_deref(), &bytes)?;
                let max_bytes = Self::vision_inline_max_bytes(&mime);
                if bytes.len() > max_bytes {
                    anyhow::bail!(
                        "vision_ocr input is too large: {} bytes (max {})",
                        bytes.len(),
                        max_bytes
                    );
                }
                Ok((url.as_str().to_string(), filename.to_string(), mime, bytes))
            }
            (None, None) => anyhow::bail!("vision_ocr requires upload_id, image_url, or file_url"),
        }
    }

    fn openai_response_output_text(value: &serde_json::Value) -> Option<String> {
        if let Some(text) = value
            .get("output_text")
            .and_then(|text| text.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }

        let mut parts = Vec::new();
        if let Some(outputs) = value.get("output").and_then(|output| output.as_array()) {
            for output in outputs {
                let Some(content) = output.get("content").and_then(|content| content.as_array())
                else {
                    continue;
                };
                for item in content {
                    if let Some(text) = item
                        .get("text")
                        .and_then(|text| text.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                    {
                        parts.push(text.to_string());
                    }
                }
            }
        }

        let joined = parts.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn openai_chat_vision_response_output_text(value: &serde_json::Value) -> Option<String> {
        if let Some(text) = value
            .pointer("/choices/0/message/content")
            .and_then(|content| {
                content.as_str().map(ToString::to_string).or_else(|| {
                    let mut parts = Vec::new();
                    for item in content.as_array()? {
                        if let Some(text) = item
                            .get("text")
                            .and_then(|text| text.as_str())
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                        {
                            parts.push(text.to_string());
                        }
                    }
                    let joined = parts.join("\n").trim().to_string();
                    (!joined.is_empty()).then_some(joined)
                })
            })
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        {
            return Some(text);
        }
        Self::openai_response_output_text(value)
    }

    fn gemini_response_output_text(value: &serde_json::Value) -> Option<String> {
        let mut parts = Vec::new();
        let candidates = value.get("candidates").and_then(|value| value.as_array())?;
        for candidate in candidates {
            let Some(content_parts) = candidate
                .pointer("/content/parts")
                .and_then(|value| value.as_array())
            else {
                continue;
            };
            for part in content_parts {
                if let Some(text) = part
                    .get("text")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    parts.push(text.to_string());
                }
            }
        }
        let joined = parts.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    async fn execute_openai_vision(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        detail: &str,
        instruction: &str,
        filename: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let encoded_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let media_input = if mime_type == "application/pdf" {
            serde_json::json!({
                "type": "input_file",
                "filename": filename,
                "file_data": encoded_data,
            })
        } else {
            let image_url = format!("data:{mime_type};base64,{encoded_data}");
            serde_json::json!({
                "type": "input_image",
                "image_url": image_url,
                "detail": detail,
            })
        };
        let body = serde_json::json!({
            "model": model,
            "input": [{
                "role": "user",
                "content": [
                    { "type": "input_text", "text": instruction },
                    media_input
                ]
            }]
        });
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .map_err(|error| anyhow::anyhow!("Failed to build OpenAI vision client: {}", error))?
            .post(format!("{}/responses", base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            anyhow::bail!("OpenAI vision error {}: {}", status, value);
        }
        Self::openai_response_output_text(&value)
            .ok_or_else(|| anyhow::anyhow!("OpenAI vision response did not include text output"))
    }

    async fn execute_openai_chat_vision(
        &self,
        api_key: &str,
        base_url: Option<&str>,
        model: &str,
        detail: &str,
        instruction: &str,
        filename: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        if mime_type == "application/pdf" {
            anyhow::bail!(
                "Primary chat vision fallback supports image uploads. Configure OpenAI Images or Google Gemini media vision for PDF analysis."
            );
        }
        let encoded_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let image_url = format!("data:{mime_type};base64,{encoded_data}");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .map_err(|error| {
                anyhow::anyhow!("Failed to build OpenAI-compatible vision client: {}", error)
            })?;
        let request_config = crate::core::llm_provider::resolve_openai_request_config(
            &client, api_key, base_url, model,
        )
        .await?;
        if request_config.uses_codex_cli_oauth {
            anyhow::bail!(
                "OpenAI Subscription Codex backend is not available for uploaded image analysis. Configure a media vision provider or an OpenAI-compatible vision chat model."
            );
        }
        let body = serde_json::json!({
            "model": model,
            "stream": false,
            "max_tokens": 1800,
            "messages": [
                {
                    "role": "system",
                    "content": "You are AgentArk's chat visual-analysis tool. Analyze only observable content in the supplied upload. Return concise, user-facing text suitable for later answer synthesis or memory extraction. Do not infer sensitive traits, credentials, hidden data, or facts not visible in the image."
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": format!("{instruction}\n\nFilename: {filename}")
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": image_url,
                                "detail": detail
                            }
                        }
                    ]
                }
            ]
        });
        let endpoint = format!("{}/chat/completions", request_config.base_url);
        let mut request = client
            .post(endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        if !request_config.api_key.is_empty() {
            request = request.bearer_auth(&request_config.api_key);
        }
        if request_config.is_openrouter {
            request = request
                .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                .header("X-Title", crate::branding::PRODUCT_NAME);
        }
        let response = request.json(&body).send().await?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            anyhow::bail!("OpenAI-compatible vision error {}: {}", status, value);
        }
        Self::openai_chat_vision_response_output_text(&value).ok_or_else(|| {
            anyhow::anyhow!("OpenAI-compatible vision response did not include text output")
        })
    }

    async fn execute_gemini_vision(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        instruction: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let image_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let body = serde_json::json!({
            "contents": [{
                "parts": [
                    {
                        "inline_data": {
                            "mime_type": mime_type,
                            "data": image_data
                        }
                    },
                    { "text": instruction }
                ]
            }]
        });
        let url = reqwest::Url::parse(&format!(
            "{}/models/{}:generateContent",
            base_url.trim_end_matches('/'),
            model.trim().trim_start_matches("models/")
        ))?;
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .map_err(|error| anyhow::anyhow!("Failed to build Gemini vision client: {}", error))?
            .post(url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            anyhow::bail!("Gemini vision error {}: {}", status, value);
        }
        Self::gemini_response_output_text(&value)
            .ok_or_else(|| anyhow::anyhow!("Gemini vision response did not include text output"))
    }

    fn openai_chat_vision_candidate_is_usable(
        api_key: &str,
        model: &str,
        base_url: Option<&str>,
    ) -> bool {
        let model = model.trim();
        if model.is_empty() {
            return false;
        }
        if base_url.is_some_and(crate::core::llm_provider::is_codex_cli_base_url) {
            return false;
        }

        let provider_label = crate::core::llm_provider::openai_provider_label(base_url);
        let missing_api_key = {
            let trimmed = api_key.trim();
            trimmed.is_empty() || trimmed == "[ENCRYPTED]"
        };
        if missing_api_key {
            !matches!(
                provider_label,
                crate::core::llm_provider::OPENAI_PROVIDER_ID
                    | crate::core::llm_provider::OPENROUTER_PROVIDER_ID
            )
        } else {
            true
        }
    }

    fn push_openai_chat_vision_candidate(
        candidates: &mut Vec<OpenAiChatVisionCandidate>,
        provider: &crate::core::LlmProvider,
    ) {
        let crate::core::LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } = provider
        else {
            return;
        };
        if Self::openai_chat_vision_candidate_is_usable(api_key, model, base_url.as_deref()) {
            candidates.push(OpenAiChatVisionCandidate {
                api_key: api_key.clone(),
                model: model.clone(),
                base_url: base_url.clone(),
            });
        }
    }

    fn openai_compatible_chat_vision_candidates(
        config: &AgentConfig,
    ) -> Vec<OpenAiChatVisionCandidate> {
        let mut candidates = Vec::new();

        if !config.model_pool.slots.is_empty() {
            for slot in
                config.model_pool.slots.iter().filter(|slot| {
                    slot.enabled && slot.role == crate::core::config::ModelRole::Primary
                })
            {
                Self::push_openai_chat_vision_candidate(&mut candidates, &slot.provider);
            }
            for slot in
                config.model_pool.slots.iter().filter(|slot| {
                    slot.enabled && slot.role != crate::core::config::ModelRole::Primary
                })
            {
                Self::push_openai_chat_vision_candidate(&mut candidates, &slot.provider);
            }
        }

        Self::push_openai_chat_vision_candidate(&mut candidates, &config.llm);
        Self::dedupe_openai_chat_vision_candidates(candidates)
    }

    fn dedupe_openai_chat_vision_candidates(
        candidates: Vec<OpenAiChatVisionCandidate>,
    ) -> Vec<OpenAiChatVisionCandidate> {
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for candidate in candidates {
            let key = format!(
                "{}\n{}\n{}",
                candidate.provider_label(),
                candidate.base_url.as_deref().unwrap_or("").trim(),
                candidate.model.trim().to_ascii_lowercase()
            );
            if seen.insert(key) {
                deduped.push(candidate);
            }
        }
        deduped
    }

    fn compact_vision_error(error: &anyhow::Error) -> String {
        let mut parts = Vec::new();
        for cause in error.chain() {
            let redacted = crate::security::redact_secret_input(&cause.to_string()).text;
            let collapsed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
            if !collapsed.is_empty() && parts.last() != Some(&collapsed) {
                parts.push(collapsed);
            }
        }
        let mut text = parts.join(": ");
        const MAX_ERROR_CHARS: usize = 900;
        if text.chars().count() > MAX_ERROR_CHARS {
            text = text.chars().take(MAX_ERROR_CHARS).collect::<String>();
            text.push_str("...");
        }
        text
    }

    async fn execute_vision_ocr(&self, arguments: &serde_json::Value) -> Result<String> {
        let settings = self.settings_manager()?.load()?;
        let requested_provider = arguments.get("provider").and_then(|value| value.as_str());
        let task = arguments
            .get("task")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("extract_text");
        let detail = arguments
            .get("detail")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("auto");
        if !matches!(detail, "auto" | "low" | "high") {
            anyhow::bail!("vision_ocr detail must be auto, low, or high");
        }
        let instruction = Self::vision_instruction(
            task,
            arguments.get("question").and_then(|value| value.as_str()),
        )?;
        let (source, filename, mime_type, bytes) = self.load_vision_input(arguments).await?;
        let selected_media_provider = Self::select_vision_provider(&settings, requested_provider);
        let model_override = arguments
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if requested_provider.is_some() {
            let provider = selected_media_provider?;
            let api_key = Self::media_provider_secret(&settings, provider).ok_or_else(|| {
                anyhow::anyhow!(
                    "Provider '{}' is selected for vision_ocr but has no configured API key.",
                    provider.id()
                )
            })?;
            let base_url = Self::media_provider_base_url(&settings, provider);
            let model = arguments
                .get("model")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| Self::default_vision_model(provider))
                .to_string();

            let text = match provider {
                crate::integrations::media_gen::MediaProvider::OpenAiDalle => {
                    self.execute_openai_vision(
                        &api_key,
                        &base_url,
                        &model,
                        detail,
                        &instruction,
                        &filename,
                        &mime_type,
                        &bytes,
                    )
                    .await?
                }
                crate::integrations::media_gen::MediaProvider::GoogleGemini => {
                    self.execute_gemini_vision(
                        &api_key,
                        &base_url,
                        &model,
                        &instruction,
                        &mime_type,
                        &bytes,
                    )
                    .await?
                }
                _ => unreachable!("select_vision_provider only returns supported vision providers"),
            };

            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "provider": provider.id(),
                "mode": "media_vision",
                "model": model,
                "task": task,
                "source": source,
                "mime_type": mime_type,
                "text": text,
            }))?);
        }

        let media_provider_error = selected_media_provider
            .as_ref()
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();

        let mut failures = Vec::new();
        if mime_type != "application/pdf" {
            let mut attempted_chat = HashSet::new();
            for candidate in Self::openai_compatible_chat_vision_candidates(&settings) {
                let model = model_override
                    .unwrap_or(candidate.model.as_str())
                    .to_string();
                let attempt_key = format!(
                    "{}\n{}\n{}",
                    candidate.provider_label(),
                    candidate.base_url.as_deref().unwrap_or("").trim(),
                    model.trim().to_ascii_lowercase()
                );
                if !attempted_chat.insert(attempt_key.clone()) {
                    continue;
                }
                match self
                    .execute_openai_chat_vision(
                        candidate.api_key.as_str(),
                        candidate.base_url.as_deref(),
                        &model,
                        detail,
                        &instruction,
                        &filename,
                        &mime_type,
                        &bytes,
                    )
                    .await
                {
                    Ok(text) => {
                        return Ok(serde_json::to_string_pretty(&serde_json::json!({
                            "provider": candidate.provider_label(),
                            "mode": "configured_chat_vision",
                            "model": model,
                            "task": task,
                            "source": source,
                            "mime_type": mime_type,
                            "text": text,
                        }))?);
                    }
                    Err(error) => {
                        failures.push(format!(
                            "{} model '{}' failed: {}",
                            candidate.provider_label(),
                            model,
                            Self::compact_vision_error(&error)
                        ));
                    }
                }
            }
        }

        if let Ok(provider) = selected_media_provider {
            let api_key = Self::media_provider_secret(&settings, provider).ok_or_else(|| {
                anyhow::anyhow!(
                    "Provider '{}' is selected for vision_ocr but has no configured API key.",
                    provider.id()
                )
            })?;
            let base_url = Self::media_provider_base_url(&settings, provider);
            let model = model_override
                .unwrap_or_else(|| Self::default_vision_model(provider))
                .to_string();

            let result = match provider {
                crate::integrations::media_gen::MediaProvider::OpenAiDalle => {
                    self.execute_openai_vision(
                        &api_key,
                        &base_url,
                        &model,
                        detail,
                        &instruction,
                        &filename,
                        &mime_type,
                        &bytes,
                    )
                    .await
                }
                crate::integrations::media_gen::MediaProvider::GoogleGemini => {
                    self.execute_gemini_vision(
                        &api_key,
                        &base_url,
                        &model,
                        &instruction,
                        &mime_type,
                        &bytes,
                    )
                    .await
                }
                _ => unreachable!("select_vision_provider only returns supported vision providers"),
            };

            match result {
                Ok(text) => {
                    return Ok(serde_json::to_string_pretty(&serde_json::json!({
                        "provider": provider.id(),
                        "mode": "media_vision",
                        "model": model,
                        "task": task,
                        "source": source,
                        "mime_type": mime_type,
                        "text": text,
                    }))?);
                }
                Err(error) => {
                    failures.push(format!(
                        "{} model '{}' failed: {}",
                        provider.id(),
                        model,
                        Self::compact_vision_error(&error)
                    ));
                }
            }
        }

        if failures.is_empty() {
            if mime_type == "application/pdf" {
                anyhow::bail!(
                    "vision_ocr could not analyze this PDF because no dedicated media vision provider is configured. Configure OpenAI Images or Google Gemini in Settings > Media."
                );
            }
            anyhow::bail!(
                "vision_ocr could not analyze this image because no usable configured chat vision model was available and no dedicated media vision provider is configured ({media_provider_error})."
            );
        }

        anyhow::bail!(
            "vision_ocr could not analyze this upload. Attempts: {}{}",
            failures.join(" | "),
            if media_provider_error.is_empty() {
                String::new()
            } else {
                format!(" | media provider selection: {media_provider_error}")
            }
        )
    }

    fn runtime_storage(&self) -> Result<crate::storage::Storage> {
        self.storage
            .clone()
            .ok_or_else(|| anyhow::anyhow!("AgentArk storage is not available in this runtime"))
    }

    fn compact_text(value: &str, max_chars: usize) -> String {
        if value.chars().count() <= max_chars {
            return value.to_string();
        }
        value.chars().take(max_chars).collect::<String>()
    }

    async fn load_storage_json_value(
        storage: &crate::storage::Storage,
        key: &str,
    ) -> Option<serde_json::Value> {
        storage
            .get(key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
    }

    fn preview_json_array(
        value: Option<serde_json::Value>,
        limit: usize,
    ) -> Option<serde_json::Value> {
        match value {
            Some(serde_json::Value::Array(items)) => Some(serde_json::Value::Array(
                items
                    .into_iter()
                    .rev()
                    .take(limit)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
            )),
            other => other,
        }
    }

    fn pulse_event_from_storage_row(
        row: crate::storage::arkpulse_event::Model,
    ) -> Option<crate::sentinel::PulseEvent> {
        Some(crate::sentinel::PulseEvent {
            timestamp: row.timestamp,
            status: row.status,
            message: row.message,
            summary: row.summary,
            flags: serde_json::from_str(&row.flags_json).ok()?,
            overdue_tasks: row.overdue_tasks.max(0) as usize,
            failed_tasks: row.failed_tasks.max(0) as usize,
            details: serde_json::from_str(&row.details_json).ok()?,
        })
    }

    fn summarize_pulse_event(row: &crate::storage::arkpulse_event::Model) -> serde_json::Value {
        let flags = serde_json::from_str::<serde_json::Value>(&row.flags_json)
            .unwrap_or_else(|_| serde_json::json!([]));
        let details = serde_json::from_str::<serde_json::Value>(&row.details_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        let doctor_findings = details
            .get("doctor_findings")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let health_checks = details
            .get("health_checks")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        serde_json::json!({
            "timestamp": row.timestamp,
            "status": row.status,
            "message": Self::compact_text(&row.message, 180),
            "summary": Self::compact_text(&row.summary, 220),
            "flags": flags,
            "overdue_tasks": row.overdue_tasks.max(0),
            "failed_tasks": row.failed_tasks.max(0),
            "doctor_finding_count": doctor_findings.len(),
            "health_check_count": health_checks.len(),
            "details": details,
        })
    }

    async fn inspect_arkpulse_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let rows = storage.list_arkpulse_events(limit.max(12)).await?;
        let stored_count = storage
            .count_arkpulse_events()
            .await
            .unwrap_or(rows.len() as u64);
        let latest = rows.first();
        let latest_details = latest
            .and_then(|row| serde_json::from_str::<serde_json::Value>(&row.details_json).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let latest_flags = latest
            .and_then(|row| serde_json::from_str::<serde_json::Value>(&row.flags_json).ok())
            .unwrap_or_else(|| serde_json::json!([]));
        let mut anomalies = Vec::new();
        if let Some(row) = latest {
            if !row.status.eq_ignore_ascii_case("ok") {
                anomalies.push(serde_json::json!({
                    "severity": row.status,
                    "message": Self::compact_text(&row.summary, 220),
                }));
            }
            if row.failed_tasks > 0 {
                anomalies.push(serde_json::json!({
                    "severity": "warn",
                    "message": format!("{} failed task(s) were observed in the latest ArkPulse run.", row.failed_tasks),
                }));
            }
            if row.overdue_tasks > 0 {
                anomalies.push(serde_json::json!({
                    "severity": "warn",
                    "message": format!("{} overdue task(s) were observed in the latest ArkPulse run.", row.overdue_tasks),
                }));
            }
        }
        if let Some(findings) = latest_details
            .get("doctor_findings")
            .and_then(|value| value.as_array())
        {
            for finding in findings.iter().take(limit as usize) {
                let severity = finding
                    .get("severity")
                    .and_then(|value| value.as_str())
                    .unwrap_or("info");
                if severity.eq_ignore_ascii_case("info") {
                    continue;
                }
                anomalies.push(serde_json::json!({
                    "severity": severity,
                    "title": finding.get("title").and_then(|value| value.as_str()),
                    "message": finding.get("message").and_then(|value| value.as_str()),
                    "target": finding.get("target").and_then(|value| value.as_str()),
                }));
            }
        }
        Ok(serde_json::json!({
            "surface": "arkpulse",
            "running": crate::sentinel::is_pulse_running(),
            "stored_event_count": stored_count,
            "latest_status": latest.map(|row| row.status.clone()),
            "latest_timestamp": latest.map(|row| row.timestamp.clone()),
            "latest_flags": latest_flags,
            "anomalies": anomalies,
            "recent_events": rows.iter().take(limit as usize).map(Self::summarize_pulse_event).collect::<Vec<_>>(),
        }))
    }

    async fn inspect_gateway_ops_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let config = self.settings_manager()?.load()?;
        let pulse_rows = storage.list_arkpulse_events(limit.max(12)).await?;
        let pulse_events = pulse_rows
            .into_iter()
            .filter_map(Self::pulse_event_from_storage_row)
            .collect::<Vec<_>>();
        let overview = crate::core::GatewayOpsControlPlane::overview_from_parts(
            storage,
            &config,
            Some(pulse_events.as_slice()),
        )
        .await?;
        Ok(serde_json::json!({
            "surface": "gateway_ops",
            "overview": overview,
        }))
    }

    async fn inspect_sentinel_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let autonomy_settings = storage
            .get(crate::core::AUTONOMY_SETTINGS_STORAGE_KEY)
            .await?
            .and_then(|raw| serde_json::from_slice::<crate::core::AutonomySettings>(&raw).ok())
            .unwrap_or_default();
        let scan_state = Self::load_storage_json_value(storage, "sentinel_scan_state_v1").await;
        let observations = Self::preview_json_array(
            Self::load_storage_json_value(storage, "sentinel_observations_v1").await,
            limit as usize,
        );
        let proposals = Self::preview_json_array(
            Self::load_storage_json_value(storage, "sentinel_proposals_v1").await,
            limit as usize,
        );
        let background_learning =
            crate::channels::http::load_background_learning_feed(storage, &autonomy_settings).await;
        Ok(serde_json::json!({
            "surface": "sentinel",
            "autonomy_mode": autonomy_settings.autonomy_mode,
            "agent_paused": autonomy_settings.agent_paused,
            "settings": autonomy_settings.sentinel,
            "scan_state": scan_state,
            "observation_count": observations.as_ref().and_then(|value| value.as_array()).map(|items| items.len()).unwrap_or(0),
            "proposal_count": proposals.as_ref().and_then(|value| value.as_array()).map(|items| items.len()).unwrap_or(0),
            "observations": observations,
            "proposals": proposals,
            "background_learning": background_learning,
        }))
    }

    async fn inspect_evolution_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let learning_enabled = crate::core::learning::load_learning_enabled(storage).await;
        let learning_model_slot = crate::core::learning::load_learning_model_slot(storage).await;
        let learning_queue_cap = crate::core::learning::load_learning_queue_cap(storage).await;
        let queue_counts = storage.learning_queue_counts().await?;
        let candidates = storage
            .list_learning_candidates_with_options(None, false, limit)
            .await?;
        let patterns = storage
            .list_procedural_patterns(None, None, &["active", "draft"], limit)
            .await?;
        let items = storage
            .list_active_experience_items(
                &["constraint", "personal_fact", "lesson", "procedure"],
                None,
                None,
                limit,
            )
            .await?;
        let recent_runs = storage.list_recent_experience_runs_any_scope(limit).await?;
        Ok(serde_json::json!({
            "surface": "evolution",
            "learning_enabled": learning_enabled,
            "learning_model_slot": learning_model_slot,
            "learning_queue_cap": learning_queue_cap,
            "queue_counts": queue_counts,
            "review_queue_size": candidates.iter().filter(|candidate| candidate.approval_status == "draft").count(),
            "recent_candidates": candidates,
            "recent_patterns": patterns,
            "recent_items": items,
            "recent_runs": recent_runs,
        }))
    }

    async fn inspect_trace_json(
        &self,
        storage: &crate::storage::Storage,
        trace_id: Option<&str>,
        limit: u64,
    ) -> Result<serde_json::Value> {
        if let Some(trace_id) = trace_id.map(str::trim).filter(|value| !value.is_empty()) {
            let trace = storage.get_execution_trace(trace_id).await?;
            let logs = storage
                .list_operational_logs_for_trace_ids(&[trace_id.to_string()], limit.max(12))
                .await?;
            return Ok(serde_json::json!({
                "surface": "trace",
                "trace_id": trace_id,
                "trace": trace,
                "operational_logs": logs,
            }));
        }

        let traces = storage
            .list_execution_trace_summaries(None, limit, 0)
            .await?;
        let trace_ids = traces
            .iter()
            .map(|trace| trace.id.clone())
            .collect::<Vec<_>>();
        let logs = storage
            .list_operational_logs_for_trace_ids(&trace_ids, limit.max(12))
            .await?;
        Ok(serde_json::json!({
            "surface": "trace",
            "recent_traces": traces,
            "recent_operational_logs": logs,
        }))
    }

    async fn inspect_moltbook_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let activity = Self::preview_json_array(
            Self::load_storage_json_value(storage, "moltbook_activity_log_v1").await,
            limit as usize,
        )
        .unwrap_or_else(|| serde_json::json!([]));
        let recent_errors = activity
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("level")
                            .and_then(|value| value.as_str())
                            .map(|level| level.eq_ignore_ascii_case("error"))
                            .unwrap_or(false)
                    })
                    .take(limit as usize)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(serde_json::json!({
            "surface": "moltbook",
            "configured": crate::integrations::moltbook::MoltbookConnector::new_with_config_dir(
                self.config_dir.clone()
            ).has_configured_api_key(),
            "recent_activity": activity,
            "recent_errors": recent_errors,
        }))
    }

    fn agentark_knowledge_query_terms(query: &str) -> Vec<String> {
        query
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    fn agentark_knowledge_match_score(text: &str, terms: &[String]) -> usize {
        if terms.is_empty() {
            return 1;
        }
        let haystack = text.to_ascii_lowercase();
        terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count()
    }

    fn agentark_knowledge_chunk_field(content: &str, field: &str) -> Option<String> {
        let prefix = format!("{field}:");
        content.lines().find_map(|line| {
            line.strip_prefix(&prefix)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    }

    fn agentark_knowledge_hit_json(
        hit: crate::core::document_search::DocumentSearchHit,
    ) -> serde_json::Value {
        let title = Self::agentark_knowledge_chunk_field(&hit.content, "title")
            .unwrap_or_else(|| hit.filename.clone());
        let source = Self::agentark_knowledge_chunk_field(&hit.content, "source")
            .unwrap_or_else(|| "agentark_knowledge".to_string());
        let url = Self::agentark_knowledge_chunk_field(&hit.content, "url");
        let tags = Self::agentark_knowledge_chunk_field(&hit.content, "tags");
        serde_json::json!({
            "source": source,
            "result_type": "agentark_knowledge_document",
            "title": title,
            "document_id": hit.document_id,
            "chunk_index": hit.chunk_index,
            "content": Self::compact_text(&hit.content, 1800),
            "score": hit.score,
            "lexical_score": hit.lexical_score,
            "dense_score": hit.dense_score,
            "match_reason": hit.match_reason,
            "url": url,
            "tags": tags,
        })
    }

    fn document_lookup_hit_json(
        hit: crate::core::document_search::DocumentSearchHit,
    ) -> serde_json::Value {
        serde_json::json!({
            "filename": hit.filename,
            "document_id": hit.document_id,
            "chunk_index": hit.chunk_index,
            "content": Self::compact_text(&hit.content, 1800),
            "score": hit.score,
            "lexical_score": hit.lexical_score,
            "dense_score": hit.dense_score,
            "match_reason": hit.match_reason,
        })
    }

    fn document_lookup_doc_ids_from_arguments(
        arguments: &serde_json::Value,
    ) -> Result<std::collections::HashSet<String>> {
        let mut doc_ids = std::collections::HashSet::new();
        let Some(items) = arguments.get("doc_ids") else {
            return Ok(doc_ids);
        };
        let Some(items) = items.as_array() else {
            anyhow::bail!("document_lookup doc_ids must be an array when supplied");
        };
        for item in items {
            let Some(raw) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !raw
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
            {
                anyhow::bail!("document_lookup doc_ids contain unsupported characters");
            }
            doc_ids.insert(raw.to_string());
            if doc_ids.len() >= 16 {
                break;
            }
        }
        Ok(doc_ids)
    }

    async fn execute_document_lookup(&self, arguments: &serde_json::Value) -> Result<String> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("document_lookup requires a non-empty query"))?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(6)
            .clamp(1, 12) as usize;
        let project_id = arguments
            .get("project_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_doc_ids = Self::document_lookup_doc_ids_from_arguments(arguments)?;
        let storage = self.runtime_storage()?;
        let mut docs = storage.list_documents_for_search(project_id).await?;
        if !requested_doc_ids.is_empty() {
            docs.retain(|doc| requested_doc_ids.contains(&doc.id));
        }
        let matches = crate::core::document_search::search_document_models(
            &storage,
            self.embedding_client.as_deref(),
            query,
            limit,
            docs,
        )
        .await?
        .into_iter()
        .map(Self::document_lookup_hit_json)
        .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "retrieval": {
                "mode": if self.embedding_client.is_some() {
                    "document_chunks_with_dense_similarity"
                } else {
                    "document_chunks_lexical"
                },
                "embedding_available": self.embedding_client.is_some(),
                "max_results": limit,
                "scoped_doc_ids": requested_doc_ids.iter().cloned().collect::<Vec<_>>(),
            },
            "results": matches,
        }))?)
    }

    fn agentark_capability_overview_result(actions: &[ActionDef]) -> serde_json::Value {
        let mut capability_counts = std::collections::BTreeMap::<String, usize>::new();
        let mut integration_counts = std::collections::BTreeMap::<String, usize>::new();
        let mut source_counts = std::collections::BTreeMap::<String, usize>::new();
        for action in actions {
            if action.capabilities.is_empty() {
                *capability_counts
                    .entry("uncategorized".to_string())
                    .or_default() += 1;
            } else {
                for capability in &action.capabilities {
                    let capability = capability.trim();
                    if !capability.is_empty() {
                        *capability_counts.entry(capability.to_string()).or_default() += 1;
                    }
                }
            }
            let metadata = action.planner_metadata();
            *integration_counts
                .entry(format!("{:?}", metadata.integration_class))
                .or_default() += 1;
            *source_counts
                .entry(format!("{:?}", action.source))
                .or_default() += 1;
        }
        let top_capabilities = capability_counts
            .iter()
            .rev()
            .take(48)
            .map(|(name, count)| format!("{name} ({count})"))
            .collect::<Vec<_>>();
        serde_json::json!({
            "source": crate::core::agentark_knowledge::RUNTIME_SOURCE,
            "result_type": "live_capability_registry_overview",
            "title": "Live AgentArk capability registry",
            "content": Self::compact_text(
                &format!(
                    "Current enabled action count: {}. Capability groups: {}. Integration classes: {:?}. Action sources: {:?}. This live registry is authoritative for current availability; AgentArk manual documents are supplemental context.",
                    actions.len(),
                    if top_capabilities.is_empty() { "none".to_string() } else { top_capabilities.join(", ") },
                    integration_counts,
                    source_counts,
                ),
                1800,
            ),
            "action_count": actions.len(),
            "capability_counts": capability_counts,
            "integration_class_counts": integration_counts,
            "action_source_counts": source_counts,
            "score": 1.0,
            "match_reason": "live_registry_overview",
        })
    }

    fn agentark_capability_action_result(
        action: &ActionDef,
        score: f64,
        match_reason: &str,
    ) -> serde_json::Value {
        let metadata = action.planner_metadata();
        let caps = if action.capabilities.is_empty() {
            "none".to_string()
        } else {
            action.capabilities.join(", ")
        };
        let content = format!(
            "`{}` | capabilities: {} | source: {:?} | role: {:?} | integration: {:?} | delivery: {:?} | side_effect: {:?} | requires_auth: {} | {}",
            action.name,
            caps,
            action.source,
            metadata.role,
            metadata.integration_class,
            metadata.delivery_mode,
            metadata.side_effect_level,
            metadata.requires_auth || action.authorization.requires_auth,
            action.description
        );
        serde_json::json!({
            "source": crate::core::agentark_knowledge::RUNTIME_SOURCE,
            "result_type": "live_action",
            "title": action.name.clone(),
            "action_name": action.name.clone(),
            "description": action.description.clone(),
            "version": action.version.clone(),
            "capabilities": action.capabilities.clone(),
            "action_source": action.source.clone(),
            "planner_metadata": metadata.clone(),
            "authorization": {
                "requires_auth": action.authorization.requires_auth,
                "risk_level": action.authorization.risk_level.clone(),
                "human_approval": action.authorization.human_approval.clone(),
            },
            "content": Self::compact_text(&content, 1800),
            "score": score,
            "match_reason": match_reason,
        })
    }

    fn agentark_capability_registry_results(
        actions: &[ActionDef],
        query: &str,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        if limit == 0 {
            return Vec::new();
        }
        let terms = Self::agentark_knowledge_query_terms(query);
        let mut scored = actions
            .iter()
            .map(|action| {
                let metadata = action.planner_metadata();
                let searchable = format!(
                    "{}\n{}\n{}\n{:?}\n{:?}\n{:?}\n{:?}",
                    action.name,
                    action.description,
                    action.capabilities.join("\n"),
                    action.source,
                    metadata.role,
                    metadata.integration_class,
                    metadata.delivery_mode
                );
                let raw_score = Self::agentark_knowledge_match_score(&searchable, &terms);
                let score = if terms.is_empty() {
                    1.0
                } else {
                    raw_score as f64 / terms.len().max(1) as f64
                };
                (action, raw_score, score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .2
                .partial_cmp(&left.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.name.cmp(&right.0.name))
        });

        let mut results = vec![Self::agentark_capability_overview_result(actions)];
        results.extend(
            scored
                .into_iter()
                .filter(|(_, raw_score, _)| terms.is_empty() || *raw_score > 0)
                .take(limit.saturating_sub(1))
                .map(|(action, raw_score, score)| {
                    let reason = if terms.is_empty() {
                        "live_registry_default"
                    } else if raw_score > 0 {
                        "live_registry_lexical_match"
                    } else {
                        "live_registry_context"
                    };
                    Self::agentark_capability_action_result(action, score, reason)
                }),
        );
        results
    }

    fn agentark_knowledge_fallback_results(
        actions: &[ActionDef],
        query: &str,
        limit: usize,
        doc_ids: &std::collections::HashSet<String>,
        source_filter: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let terms = Self::agentark_knowledge_query_terms(query);
        let mut scored = crate::core::agentark_knowledge::build_seed_agentark_knowledge_documents(actions)
            .into_iter()
            .filter(|doc| doc_ids.is_empty() || doc_ids.contains(&doc.id))
            .filter(|doc| source_filter.map(|source| doc.source == source).unwrap_or(true))
            .flat_map(|doc| {
                let title = doc.title.clone();
                let doc_id = doc.id.clone();
                let url = doc.url.clone();
                let tags = doc.tags.clone();
                let terms_for_doc = terms.clone();
                doc.chunks
                    .into_iter()
                    .enumerate()
                    .map(move |(chunk_index, content)| {
                        let raw_score =
                            Self::agentark_knowledge_match_score(&content, &terms_for_doc);
                        let score = if terms_for_doc.is_empty() {
                            1.0
                        } else {
                            raw_score as f64 / terms_for_doc.len().max(1) as f64
                        };
                        (
                            score,
                            serde_json::json!({
                                "source": Self::agentark_knowledge_chunk_field(&content, "source").unwrap_or_else(|| "agentark_knowledge".to_string()),
                                "result_type": "agentark_knowledge_document",
                                "title": title.clone(),
                                "document_id": doc_id.clone(),
                                "chunk_index": chunk_index,
                                "content": Self::compact_text(&content, 1800),
                                "score": score,
                                "lexical_score": score,
                                "dense_score": serde_json::Value::Null,
                                "match_reason": "lexical_fallback",
                                "url": url.clone(),
                                "tags": tags.clone(),
                            }),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|(score, _)| *score > 0.0 || terms.is_empty())
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, value)| value)
            .collect()
    }

    fn agentark_knowledge_doc_ids_from_arguments(
        arguments: &serde_json::Value,
    ) -> Result<std::collections::HashSet<String>> {
        let mut doc_ids = std::collections::HashSet::new();
        let Some(items) = arguments.get("doc_ids") else {
            return Ok(doc_ids);
        };
        let Some(items) = items.as_array() else {
            anyhow::bail!("agentark_capability_lookup doc_ids must be an array when supplied");
        };
        for item in items {
            let Some(raw) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !crate::core::agentark_knowledge::is_agentark_knowledge_document_id(raw)
                || !raw
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '-' | '_'))
            {
                anyhow::bail!(
                    "agentark_capability_lookup doc_ids may only contain AgentArk knowledge document IDs"
                );
            }
            doc_ids.insert(raw.to_string());
            if doc_ids.len() >= 16 {
                break;
            }
        }
        Ok(doc_ids)
    }

    async fn execute_agentark_capability_lookup(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("agentark_capability_lookup requires a non-empty query")
            })?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(4)
            .clamp(1, 8) as usize;
        let requested_doc_ids = Self::agentark_knowledge_doc_ids_from_arguments(arguments)?;
        let actions = self.list_enabled_actions().await?;
        let registry_results = Self::agentark_capability_registry_results(&actions, query, limit);
        let embedding_available = self.embedding_client.is_some();
        let (mode, supplemental_results) = match self.runtime_storage() {
            Ok(storage) => {
                let mut agentark_docs = match storage
                    .list_documents_by_id_prefix(
                        crate::core::agentark_knowledge::DOCUMENT_ID_PREFIX,
                        512,
                    )
                    .await
                {
                    Ok(docs) => docs,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "AgentArk manual document lookup failed; returning live registry only"
                        );
                        Vec::new()
                    }
                };
                if !requested_doc_ids.is_empty() {
                    agentark_docs.retain(|doc| requested_doc_ids.contains(&doc.id));
                }
                let semantic_results = if agentark_docs.is_empty() {
                    Vec::new()
                } else {
                    match crate::core::document_search::search_document_models(
                        &storage,
                        self.embedding_client.as_deref(),
                        query,
                        limit.saturating_mul(4).max(limit),
                        agentark_docs,
                    )
                    .await
                    {
                        Ok(hits) => hits
                            .into_iter()
                            .map(Self::agentark_knowledge_hit_json)
                            .filter(|hit| {
                                hit.get("source").and_then(|value| value.as_str())
                                    == Some(crate::core::agentark_knowledge::CURATED_SOURCE)
                            })
                            .take(limit)
                            .collect::<Vec<_>>(),
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "AgentArk manual search failed; falling back to generated manual text"
                            );
                            Vec::new()
                        }
                    }
                };
                if semantic_results.is_empty() {
                    (
                        "live_registry_with_bounded_lexical_manual_fallback",
                        Self::agentark_knowledge_fallback_results(
                            &actions,
                            query,
                            limit,
                            &requested_doc_ids,
                            Some(crate::core::agentark_knowledge::CURATED_SOURCE),
                        ),
                    )
                } else {
                    (
                        if requested_doc_ids.is_empty() {
                            "live_registry_with_pgvector_manual_chunks"
                        } else {
                            "live_registry_with_pgvector_scoped_manual_chunks"
                        },
                        semantic_results,
                    )
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "AgentArk manual storage unavailable; returning live registry only"
                );
                ("live_registry_only", Vec::new())
            }
        };
        let mut matches = registry_results;
        matches.extend(supplemental_results);
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "retrieval": {
                "mode": mode,
                "embedding_available": embedding_available,
                "result_scope": "agentark_capability_registry_and_manual",
                "max_results_per_source": limit,
                "authoritative_source": crate::core::agentark_knowledge::RUNTIME_SOURCE,
                "supplemental_source": crate::core::agentark_knowledge::CURATED_SOURCE,
                "capability_registry_action_count": actions.len(),
                "scoped_doc_ids": requested_doc_ids.iter().cloned().collect::<Vec<_>>(),
            },
            "results": matches,
        }))?)
    }

    fn memory_lookup_terms(query: &str) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter_map(|term| {
                let term = term.trim().to_ascii_lowercase();
                if term.chars().count() < 2 || !seen.insert(term.clone()) {
                    None
                } else {
                    Some(term)
                }
            })
            .collect()
    }

    fn memory_lookup_score<'a>(
        terms: &[String],
        weighted_fields: impl IntoIterator<Item = (&'a str, f32)>,
    ) -> f32 {
        if terms.is_empty() {
            return 0.0;
        }
        let fields = weighted_fields
            .into_iter()
            .map(|(value, weight)| (value.to_ascii_lowercase(), weight))
            .collect::<Vec<_>>();
        let mut score = 0.0f32;
        for term in terms {
            for (field, weight) in &fields {
                if field.contains(term) {
                    score += *weight;
                }
            }
        }
        score
    }

    fn memory_lookup_include_sensitive_experience_item(
        item: &crate::storage::experience_item::Model,
    ) -> bool {
        let sensitivity = item
            .metadata
            .get("sensitivity")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_");
        !matches!(sensitivity.as_str(), "sensitive" | "crisis_sensitive")
    }

    fn memory_lookup_experience_json(
        item: crate::storage::experience_item::Model,
    ) -> serde_json::Value {
        let key = item
            .metadata
            .get("key")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let memory_kind = item
            .metadata
            .get("memory_kind")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        serde_json::json!({
            "id": item.id,
            "kind": item.kind,
            "scope": item.scope,
            "project_id": item.project_id,
            "conversation_id": item.conversation_id,
            "title": Self::compact_text(&item.title, 180),
            "content": Self::compact_text(&item.content, 420),
            "key": key,
            "memory_kind": memory_kind,
            "confidence": item.confidence,
            "support_count": item.support_count,
            "updated_at": item.updated_at,
        })
    }

    async fn execute_memory_lookup(&self, arguments: &serde_json::Value) -> Result<String> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("memory_lookup requires a non-empty query"))?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(5)
            .clamp(1, 12) as usize;
        let include_semantic = arguments
            .get("include_semantic")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let include_structured = arguments
            .get("include_structured")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let include_procedures = arguments
            .get("include_procedures")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let include_lessons = arguments
            .get("include_lessons")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let project_id = arguments
            .get("project_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let conversation_id = arguments
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let storage = self.runtime_storage()?;
        let terms = Self::memory_lookup_terms(query);
        let semantic_kinds = if include_lessons {
            vec![
                "identity",
                "preference",
                "location",
                "workflow",
                "constraint",
                "personal_fact",
                "other",
                "lesson",
                "procedure",
            ]
        } else {
            vec![
                "identity",
                "preference",
                "location",
                "workflow",
                "constraint",
                "personal_fact",
                "other",
            ]
        };

        let semantic_facts = if include_semantic {
            let mut scored = storage
                .list_active_experience_items(&semantic_kinds, project_id, conversation_id, 96)
                .await?
                .into_iter()
                .filter(Self::memory_lookup_include_sensitive_experience_item)
                .map(|item| {
                    let key = item
                        .metadata
                        .get("key")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let memory_kind = item
                        .metadata
                        .get("memory_kind")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let score = Self::memory_lookup_score(
                        &terms,
                        [
                            (item.kind.as_str(), 2.0),
                            (key.as_str(), 2.0),
                            (memory_kind.as_str(), 2.0),
                            (item.title.as_str(), 1.5),
                            (item.content.as_str(), 1.0),
                            (item.normalized_key.as_str(), 1.0),
                        ],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| Self::memory_lookup_experience_json(item))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let preferences = if include_structured {
            let mut scored = storage
                .list_user_preferences(48, 0, project_id)
                .await?
                .into_iter()
                .map(|item| {
                    let score = Self::memory_lookup_score(
                        &terms,
                        [(item.key.as_str(), 2.0), (item.value.as_str(), 1.0)],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| {
                    serde_json::json!({
                        "id": item.id,
                        "key": item.key,
                        "value": Self::compact_text(&item.value, 320),
                        "sensitivity": item.sensitivity,
                        "confidence": item.confidence,
                        "project_id": item.project_id,
                        "updated_at": item.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let user_data = if include_structured {
            let mut scored = storage
                .list_user_data_items(48, 0, project_id, None)
                .await?
                .into_iter()
                .map(|item| {
                    let url = item.url.clone().unwrap_or_default();
                    let score = Self::memory_lookup_score(
                        &terms,
                        [
                            (item.kind.as_str(), 2.0),
                            (item.title.as_str(), 1.5),
                            (item.content.as_str(), 1.0),
                            (url.as_str(), 0.5),
                        ],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| {
                    serde_json::json!({
                        "id": item.id,
                        "kind": item.kind,
                        "title": Self::compact_text(&item.title, 180),
                        "content": Self::compact_text(&item.content, 320),
                        "url": item.url,
                        "pinned": item.pinned,
                        "project_id": item.project_id,
                        "conversation_id": item.conversation_id,
                        "updated_at": item.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let knowledge = if include_structured {
            let mut scored = storage
                .list_visible_knowledge_items(48, 0, project_id)
                .await?
                .into_iter()
                .map(|item| {
                    let tags = item.tags.clone().unwrap_or_default();
                    let source = item.source.clone().unwrap_or_default();
                    let score = Self::memory_lookup_score(
                        &terms,
                        [
                            (item.title.as_str(), 1.5),
                            (item.content.as_str(), 1.0),
                            (tags.as_str(), 1.0),
                            (source.as_str(), 0.5),
                        ],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| {
                    serde_json::json!({
                        "id": item.id,
                        "title": Self::compact_text(&item.title, 180),
                        "content": Self::compact_text(&item.content, 420),
                        "source": item.source,
                        "url": item.url,
                        "tags": item.tags,
                        "project_id": item.project_id,
                        "updated_at": item.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let procedures = if include_procedures {
            storage
                .search_procedural_patterns(query, project_id, conversation_id, limit as u64)
                .await?
                .into_iter()
                .map(|hit| {
                    serde_json::json!({
                        "id": hit.pattern.id,
                        "status": hit.pattern.status,
                        "title": Self::compact_text(&hit.pattern.title, 180),
                        "trigger_summary": Self::compact_text(&hit.pattern.trigger_summary, 260),
                        "summary": Self::compact_text(&hit.pattern.summary, 420),
                        "score": hit.score,
                        "sample_count": hit.pattern.sample_count,
                        "success_rate": hit.pattern.success_rate,
                        "updated_at": hit.pattern.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "retrieval": {
                "mode": "bounded_local_memory",
                "max_results_per_bucket": limit,
                "include_semantic": include_semantic,
                "include_structured": include_structured,
                "include_procedures": include_procedures,
                "include_lessons": include_lessons,
                "scope": {
                    "project_id": project_id,
                    "conversation_id": conversation_id,
                },
            },
            "results": {
                "semantic_facts": semantic_facts,
                "preferences": preferences,
                "user_data": user_data,
                "knowledge": knowledge,
                "procedures": procedures,
            },
        }))?)
    }

    async fn execute_ark_inspect(&self, arguments: &serde_json::Value) -> Result<String> {
        ark_inspect::execute(self, arguments).await
    }

    async fn execute_postgres_schema_inspect(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self.runtime_storage()?;
        let payload = storage
            .inspect_postgres_schema_json(
                arguments
                    .get("table_filter")
                    .and_then(|value| value.as_str()),
                arguments
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(25),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&payload)?)
    }

    async fn execute_postgres_query_readonly(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self.runtime_storage()?;
        let request: crate::storage::ReadonlyTableQuery = serde_json::from_value(arguments.clone())
            .map_err(|error| {
                anyhow::anyhow!("Invalid structured database query arguments: {}", error)
            })?;
        let payload = storage.query_table_json(&request).await.map_err(|error| {
            anyhow::anyhow!(
                "{}. Inspect the live schema with postgres_schema_inspect and retry with corrected table or column names.",
                error
            )
        })?;
        Ok(serde_json::to_string_pretty(&payload)?)
    }

    async fn sync_extension_pack_runtime_actions(&self) -> Result<()> {
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let guard = registry.read().await;
        guard.sync_to_runtime(self).await
    }

    async fn execute_extension_pack_list(&self, arguments: &serde_json::Value) -> Result<String> {
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let query = arguments.get("query").and_then(|value| value.as_str());
        let kind = arguments.get("kind").and_then(|value| value.as_str());
        let guard = registry.read().await;
        Ok(serde_json::to_string_pretty(
            &guard.search_packs(query, kind).await?,
        )?)
    }

    async fn execute_extension_pack_search(&self, arguments: &serde_json::Value) -> Result<String> {
        self.execute_extension_pack_list(arguments).await
    }

    async fn execute_extension_pack_install(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let request: crate::extension_packs::ExtensionPackInstallRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack install arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let requested_pack_id = request
            .pack_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let has_explicit_source = request
            .source_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || request
                .source_path
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || request
                .manifest_text
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || request.manifest.is_some();
        if let Some(pack_id) = requested_pack_id.as_deref().filter(|_| !has_explicit_source) {
            let existing_or_catalog = {
                let guard = registry.read().await;
                guard.get_pack(pack_id).await?
            };
            match existing_or_catalog {
                Some(view) if view.installed => {
                    return Ok(serde_json::to_string_pretty(&serde_json::json!({
                        "status": "already_installed",
                        "installed": true,
                        "pack_id": view.manifest.id,
                        "message": "Extension pack is already installed.",
                        "pack": view,
                    }))?);
                }
                None => {
                    return Ok(serde_json::to_string_pretty(&serde_json::json!({
                        "status": "catalog_miss",
                        "installed": false,
                        "pack_id": pack_id,
                        "message": "No bundled extension pack matched this id, so no pack was installed from the catalog.",
                        "next_steps": [
                            "Install from source_url, source_path, manifest_text, or manifest when a pack source exists.",
                            "Use extension_pack_scaffold for a draft manifest-based pack.",
                            "Use capability_acquire for HTTP/API integrations that should be saved as custom API integrations."
                        ]
                    }))?);
                }
                _ => {}
            }
        }
        let pack = {
            let mut guard = registry.write().await;
            guard.install(request).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&pack)?)
    }

    async fn execute_extension_pack_scaffold(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let request: crate::extension_packs::ExtensionPackScaffoldRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack scaffold arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let pack = {
            let mut guard = registry.write().await;
            guard.scaffold(request).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&pack)?)
    }

    async fn execute_custom_messaging_channel_upsert(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self.runtime_storage()?;
        let request: crate::custom_messaging_channels::CustomMessagingChannelUpsertRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid custom messaging channel arguments: {}", error)
            })?;
        let requested_id = crate::custom_messaging_channels::config_id_for_request(&request);
        let existing = crate::custom_messaging_channels::get_custom_messaging_channel_config(
            &storage,
            &requested_id,
        )
        .await?;
        let operation = if existing.is_some() {
            "update"
        } else {
            "create"
        };
        let view = crate::custom_messaging_channels::upsert_custom_messaging_channel(
            &storage,
            &self.config_dir,
            self.data_dir(),
            request,
            existing.as_ref().map(|_| requested_id.as_str()),
        )
        .await?;
        self.record_custom_messaging_channel_upsert_event(&view, operation)
            .await;
        let integration_id = view
            .config
            .auth_manifest
            .as_ref()
            .map(|manifest| manifest.integration_id.clone());
        let channel_id = view.runtime_channel_id.clone();
        let config_id = view.config.id.clone();
        let channel_name = view.config.name.clone();
        let needs_credentials = view.requires_auth && !view.configured;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": if needs_credentials { "needs_credentials" } else { "configured" },
            "channel_id": channel_id,
            "integration_id": integration_id,
            "custom_messaging_channel": {
                "id": config_id,
                "name": channel_name,
                "configured": view.configured,
                "requires_auth": view.requires_auth,
            },
            "message": if needs_credentials {
                "Custom messaging channel saved. Credentials are still required. Use the secure credential form; do not paste secrets into normal chat."
            } else {
                "Custom messaging channel saved and ready."
            }
        }))?)
    }

    async fn execute_extension_pack_connect(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let request: crate::extension_packs::ExtensionPackConnectionUpsertRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack connect arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let (connection, oauth_hint) = {
            let mut guard = registry.write().await;
            let connection = guard.upsert_connection(pack_id, request).await?;
            let oauth_hint = guard.supports_connect_url(pack_id);
            (connection, oauth_hint)
        };
        self.sync_extension_pack_runtime_actions().await?;
        let redirect_uri = arguments
            .get("redirect_uri")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let connect_path = if oauth_hint {
            let mut path = format!(
                "/extension-packs/{}/connect-url",
                urlencoding::encode(pack_id)
            );
            if let Some(redirect_uri) = redirect_uri {
                let suffix = format!("redirect_uri={}", urlencoding::encode(redirect_uri));
                path.push('?');
                path.push_str(&suffix);
            }
            Some(path)
        } else {
            None
        };
        let connect_url = connect_path
            .as_deref()
            .map(|path| format!("{}{}", crate::core::net::internal_api_base_url(), path));
        let (pack_name, required_secrets) = {
            let guard = registry.read().await;
            let pack = guard.get_pack(pack_id).await?;
            let pack_name = pack
                .as_ref()
                .map(|pack| pack.manifest.name.clone())
                .unwrap_or_else(|| pack_id.to_string());
            let required_secrets = pack
                .as_ref()
                .map(|pack| {
                    pack.manifest
                        .auth
                        .required_secrets
                        .iter()
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|items| !items.is_empty())
                .unwrap_or_else(|| match connection.auth_mode {
                    crate::extension_packs::ExtensionPackAuthMode::ApiKey => {
                        vec!["api_key".to_string(), "access_token".to_string()]
                    }
                    crate::extension_packs::ExtensionPackAuthMode::Basic => {
                        vec!["username".to_string(), "password".to_string()]
                    }
                    _ => Vec::new(),
                });
            (pack_name, required_secrets)
        };
        let needs_credentials = !oauth_hint
            && matches!(
                connection.state,
                crate::extension_packs::ExtensionConnectionState::NeedsAuth
            )
            && !required_secrets.is_empty();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": if needs_credentials {
                "needs_credentials"
            } else if oauth_hint {
                "oauth_pending"
            } else {
                "connected"
            },
            "pack_id": pack_id,
            "pack_name": pack_name,
            "connection": connection,
            "required_secrets": required_secrets,
            "oauth_connect_in_ui": oauth_hint,
            "connect_url_endpoint": connect_path,
            "connect_url": connect_url,
            "message": if needs_credentials {
                "Connection draft saved. Credentials are still required. Never paste secrets, API keys, passwords, or sensitive data into normal chat. Use the secure credential UI."
            } else if oauth_hint {
                "Connection record saved. Complete OAuth by opening the returned connect_url in a browser."
            } else {
                "Connection saved."
            }
        }))?)
    }

    async fn execute_extension_pack_set_enabled(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let enabled = arguments
            .get("enabled")
            .and_then(|value| value.as_bool())
            .ok_or_else(|| anyhow::anyhow!("Missing enabled"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let pack = {
            let mut guard = registry.write().await;
            guard.set_pack_enabled(pack_id, enabled).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&pack)?)
    }

    async fn execute_extension_pack_runtime_install(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.install_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_extension_pack_runtime_verify(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.verify_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_extension_pack_runtime_update(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.update_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_extension_pack_runtime_uninstall(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.uninstall_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_extension_pack_test_connection(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let requested_connection_id = arguments
            .get("connection_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let mut guard = registry.write().await;
        let pick_connection_id =
            |connections: Vec<crate::extension_packs::ExtensionPackConnectionView>| {
                connections
                    .iter()
                    .find(|item| {
                        item.connection.enabled
                            && matches!(
                                item.state,
                                crate::extension_packs::ExtensionConnectionState::Ready
                            )
                    })
                    .or_else(|| connections.iter().find(|item| item.connection.enabled))
                    .or_else(|| connections.first())
                    .map(|item| item.connection.id.clone())
            };
        let resolved_connection_id = if let Some(connection_id) = requested_connection_id.clone() {
            connection_id
        } else {
            pick_connection_id(guard.list_connections(pack_id).await?).ok_or_else(|| {
                anyhow::anyhow!("No connection is configured for pack '{}'", pack_id)
            })?
        };
        let (resolved_connection_id, result) = match guard
            .test_connection(
                pack_id,
                &resolved_connection_id,
                self.mcp_registry.clone(),
                self.plugin_registry.clone(),
            )
            .await
        {
            Ok(result) => (resolved_connection_id, result),
            Err(error)
                if requested_connection_id
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(pack_id))
                    && error
                        .to_string()
                        .contains(&format!("Connection '{}' was not found", pack_id)) =>
            {
                let fallback_id = pick_connection_id(guard.list_connections(pack_id).await?)
                    .ok_or_else(|| {
                        anyhow::anyhow!("No connection is configured for pack '{}'", pack_id)
                    })?;
                let result = guard
                    .test_connection(
                        pack_id,
                        &fallback_id,
                        self.mcp_registry.clone(),
                        self.plugin_registry.clone(),
                    )
                    .await?;
                (fallback_id, result)
            }
            Err(error) => return Err(error),
        };
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "resolved_connection_id": resolved_connection_id,
            "result": result,
        }))?)
    }

    async fn execute_extension_pack_list_events(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(25) as usize;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let guard = registry.read().await;
        Ok(serde_json::to_string_pretty(
            &guard.list_events(pack_id, limit).await?,
        )?)
    }

    async fn execute_extension_pack_invoke(&self, arguments: &serde_json::Value) -> Result<String> {
        let request: crate::extension_packs::ExtensionPackInvokeRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack invoke arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let mut guard = registry.write().await;
        let result = guard
            .invoke_feature(
                request,
                self.mcp_registry.clone(),
                self.plugin_registry.clone(),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_capability_resolve(&self, arguments: &serde_json::Value) -> Result<String> {
        let goal = arguments
            .get("goal")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'goal' for capability resolution"))?;
        let requested_capability = arguments
            .get("requested_capability")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected_action = arguments
            .get("selected_action")
            .or_else(|| arguments.get("requested_action"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let failure_output = arguments
            .get("failure_output")
            .and_then(|value| value.as_str())
            .or_else(|| arguments.get("error").and_then(|value| value.as_str()))
            .unwrap_or("");
        let files = self.collect_code_execute_files(arguments).await?;
        let detected_inputs = files
            .iter()
            .map(|file| {
                Self::upload_signature(&file.filename, file.content_type.as_deref(), &file.bytes)
            })
            .collect::<Vec<_>>();

        let requested_capability_key =
            requested_capability.map(|value| value.to_ascii_lowercase().replace([' ', '-'], "_"));
        let selected_action_key = selected_action.map(str::to_ascii_lowercase);
        let has_audio_like_file = detected_inputs.iter().any(|input| {
            matches!(
                input.get("input_type").and_then(|value| value.as_str()),
                Some("audio") | Some("audio_video")
            )
        });
        let missing_binary = Self::detect_missing_binary_from_output(failure_output);

        let mut missing_capabilities = Vec::new();
        let mut routes = Vec::new();
        let mut next_actions = Vec::new();
        let mut notes = Vec::new();

        if let Some(binary) = missing_binary.as_deref() {
            missing_capabilities.push(serde_json::json!({
                "kind": "binary",
                "name": binary,
                "approval_required": true,
                "reason": "The previous execution failed because this executable is not present in the sandbox/runtime environment.",
            }));
            routes.push(serde_json::json!({
                "route": "host_install_approval",
                "approval_required": true,
                "auto_allowed": false,
                "reason": "Sandbox-local pip/npm installs are allowed, but OS/host binary installation is approval-gated.",
            }));
        }

        if selected_action_key.as_deref() == Some("transcribe_audio") && has_audio_like_file {
            next_actions.push(serde_json::json!({
                "name": "code_execute",
                "arguments": {
                    "language": "python",
                    "code": Self::build_sandbox_transcription_code(),
                    "files": arguments.get("files").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "file_payloads": arguments.get("file_payloads").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "network_access": true,
                    "timeout_secs": 600,
                    "execution_contract": {
                        "phase": "validate",
                        "target_validated_when_successful": true
                    }
                },
                "why": "Run the catalog-selected transcription action inside the code sandbox after byte-level media detection. The script checks for ffmpeg and emits a structured missing-binary marker instead of installing OS packages."
            }));
            routes.push(serde_json::json!({
                "route": "sandbox_code_execute",
                "approval_required": false,
                "auto_allowed": true,
                "reason": "Use sandbox-local Python packages first; do not run host installers unless the sandbox reports a missing binary.",
            }));
            notes.push("Audio-like upload detected by bytes; prefer the selected sandbox action path before any host install.".to_string());
        }

        let pack_query = requested_capability
            .or(selected_action)
            .unwrap_or(goal)
            .trim();
        if !pack_query.is_empty() {
            if let Some(registry) = self.extension_pack_registry.as_ref() {
                let pack_search = {
                    let guard = registry.read().await;
                    guard
                        .search_packs(Some(pack_query), Some("integration"))
                        .await
                        .ok()
                };
                if let Some(pack_search) = pack_search {
                    let top_installed = pack_search
                        .installed
                        .into_iter()
                        .take(3)
                        .collect::<Vec<_>>();
                    let top_catalog = pack_search.catalog.into_iter().take(3).collect::<Vec<_>>();
                    let mut candidate_labels = top_installed
                        .iter()
                        .map(|pack| {
                            format!(
                                "{} ({})",
                                pack.manifest.name.as_str(),
                                pack.manifest.id.as_str()
                            )
                        })
                        .collect::<Vec<_>>();
                    candidate_labels.extend(top_catalog.iter().map(|pack| {
                        format!(
                            "{} ({})",
                            pack.manifest.name.as_str(),
                            pack.manifest.id.as_str()
                        )
                    }));
                    if !candidate_labels.is_empty() {
                        notes.push(format!(
                            "Extension-pack candidates for this capability: {}.",
                            candidate_labels.join(", ")
                        ));
                        routes.push(serde_json::json!({
                            "route": "extension_pack",
                            "approval_required": false,
                            "auto_allowed": true,
                            "query": pack_query,
                            "installed_matches": top_installed.len(),
                            "catalog_matches": top_catalog.len(),
                            "requires_confirmation": top_installed.is_empty() && top_catalog.len() > 1,
                            "reason": "Use the generic extension-pack lifecycle for integration install, runtime setup, auth, and action registration.",
                        }));
                    }
                    for pack in &top_installed {
                        if !pack.enabled {
                            next_actions.push(serde_json::json!({
                                "name": "extension_pack_set_enabled",
                                "arguments": {
                                    "pack_id": pack.manifest.id.clone(),
                                    "enabled": true
                                },
                                "why": format!(
                                    "Enable the installed extension pack '{}' so its registered actions can be used.",
                                    pack.manifest.name.as_str()
                                )
                            }));
                            continue;
                        }
                        if pack.runtime_required
                            && pack.runtime_status
                                != crate::extension_packs::ExtensionPackRuntimeStatus::Ready
                        {
                            next_actions.push(serde_json::json!({
                                "name": "extension_pack_runtime_install",
                                "arguments": {
                                    "pack_id": pack.manifest.id.clone()
                                },
                                "why": format!(
                                    "Install or verify the local runtime declared by '{}'.",
                                    pack.manifest.name.as_str()
                                )
                            }));
                        }
                        if pack.needs_auth
                            && matches!(
                                pack.status.as_str(),
                                "needs_auth" | "runtime_missing" | "available"
                            )
                        {
                            next_actions.push(serde_json::json!({
                                "name": "extension_pack_connect",
                                "arguments": {
                                    "pack_id": pack.manifest.id.clone()
                                },
                                "why": format!(
                                    "Create or refresh the connection record for '{}'.",
                                    pack.manifest.name.as_str()
                                )
                            }));
                        }
                    }
                    if top_installed.is_empty() && top_catalog.len() == 1 {
                        let pack = &top_catalog[0];
                        next_actions.push(serde_json::json!({
                            "name": "extension_pack_install",
                            "arguments": {
                                "pack_id": pack.manifest.id.clone()
                            },
                            "why": format!(
                                "Install the catalog integration '{}' through the shared extension-pack flow.",
                                pack.manifest.name.as_str()
                            )
                        }));
                    }
                }
            }
        }

        if let Some(requested) = requested_capability_key.as_deref() {
            notes.push(format!("Requested capability hint: {}.", requested));
        }
        if let Some(action) = selected_action_key.as_deref() {
            notes.push(format!("Selected catalog action: {}.", action));
        }
        if detected_inputs.is_empty() {
            notes.push("No upload files were provided for byte-level inspection.".to_string());
        }
        if routes.is_empty() {
            routes.push(serde_json::json!({
                "route": "inspect_then_choose",
                "approval_required": false,
                "auto_allowed": true,
                "reason": "No concrete missing capability was detected yet. Inspect with the nearest read-only/workspace tool, then retry capability_resolve with any failure output.",
            }));
        }

        let approval_required = missing_capabilities.iter().any(|capability| {
            capability
                .get("approval_required")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        });
        let mut result = serde_json::json!({
            "resolver": "capability_resolve",
            "status": if approval_required { "needs_approval" } else { "ready" },
            "policy": "sandbox_first",
            "goal": goal,
            "detected_inputs": detected_inputs,
            "missing_capabilities": missing_capabilities,
            "acquisition_routes": routes,
            "next_actions": next_actions,
            "verification": {
                "required": true,
                "evidence": "The next action should produce successful tool output, app health/log evidence, or a concrete approval blocker."
            },
            "notes": notes,
        });

        if approval_required {
            result["approval_request"] = serde_json::json!({
                "title": "Capability approval required",
                "summary": "AgentArk detected a missing host/system capability that is not safe to install automatically.",
                "reason": missing_binary
                    .as_ref()
                    .map(|binary| format!("Missing binary: {}.", binary))
                    .unwrap_or_else(|| "A host-level capability is required.".to_string()),
                "risk_level": "environment_change",
                "risk_score": 72,
                "source": "capability_resolve",
                "comment_supported": true
            });
        }

        Ok(serde_json::to_string_pretty(&result)?)
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
        if Self::capability_acquire_has_http_endpoint(&arguments) {
            return self
                .execute_capability_acquire_custom_api(&arguments, &name, description)
                .await;
        }
        Err(anyhow::anyhow!(
            "capability_acquire only saves API integrations. Use the explicit Skills import/create flow for user skills, or extension-pack actions for manifest-based integrations."
        ))
    }

    fn capability_acquire_has_http_endpoint(arguments: &serde_json::Value) -> bool {
        let kind = Self::capability_string_argument(arguments, "kind")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if kind == "web_automation" {
            return false;
        }
        Self::capability_string_argument(arguments, "base_url").is_some()
            || Self::capability_string_argument(arguments, "path")
                .as_deref()
                .is_some_and(|path| path.starts_with("http://") || path.starts_with("https://"))
            || Self::capability_string_argument(arguments, "openapi_url").is_some()
            || Self::capability_string_argument(arguments, "openapi_text").is_some()
    }

    fn capability_auth_mode(
        arguments: &serde_json::Value,
    ) -> Option<crate::custom_apis::CustomApiAuthMode> {
        match Self::capability_string_argument(arguments, "auth_type")?
            .to_ascii_lowercase()
            .as_str()
        {
            "bearer" => Some(crate::custom_apis::CustomApiAuthMode::Bearer),
            "api_key_header" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyHeader),
            "api_key_query" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyQuery),
            "oauth2" => Some(crate::custom_apis::CustomApiAuthMode::OAuth2),
            "basic" => Some(crate::custom_apis::CustomApiAuthMode::Basic),
            "none" => Some(crate::custom_apis::CustomApiAuthMode::None),
            _ => None,
        }
    }

    fn capability_object_to_string_map(
        value: Option<&serde_json::Value>,
    ) -> BTreeMap<String, String> {
        value
            .and_then(|value| value.as_object())
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| {
                        Self::value_to_http_string(value)
                            .map(|value| (key.trim().to_string(), value.trim().to_string()))
                    })
                    .filter(|(key, _)| !key.is_empty())
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default()
    }

    fn capability_endpoint_parts(
        arguments: &serde_json::Value,
    ) -> Result<(String, String, BTreeMap<String, String>)> {
        let raw_base = Self::capability_string_argument(arguments, "base_url");
        let raw_path = Self::capability_string_argument(arguments, "path");
        let endpoint = match (raw_base.as_deref(), raw_path.as_deref()) {
            (_, Some(path)) if path.starts_with("http://") || path.starts_with("https://") => {
                path.to_string()
            }
            (Some(base), Some(path)) if !path.trim().is_empty() => {
                format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    path.trim_start_matches('/')
                )
            }
            (Some(base), _) => base.to_string(),
            _ => {
                return Err(anyhow::anyhow!(
                    "HTTP/API capability acquisition requires a base_url or absolute path"
                ));
            }
        };
        let parsed = reqwest::Url::parse(endpoint.as_str())
            .with_context(|| format!("Invalid API endpoint '{}'", endpoint))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("API endpoint must include a host"))?;
        let mut base_url = format!("{}://{}", parsed.scheme(), host);
        if let Some(port) = parsed.port() {
            base_url.push(':');
            base_url.push_str(&port.to_string());
        }
        let path = if parsed.path().trim().is_empty() {
            "/".to_string()
        } else {
            parsed.path().to_string()
        };
        let mut query = BTreeMap::new();
        for (key, value) in parsed.query_pairs() {
            query.insert(key.to_string(), value.to_string());
        }
        Ok((base_url, path, query))
    }

    fn capability_operation_draft(
        arguments: &serde_json::Value,
        name: &str,
        description: &str,
    ) -> Result<(
        String,
        crate::custom_apis::CustomApiOperationDraft,
        BTreeMap<String, String>,
    )> {
        let (base_url, path, mut default_query) = Self::capability_endpoint_parts(arguments)?;
        default_query.extend(Self::capability_object_to_string_map(
            arguments.get("default_query"),
        ));
        let method = Self::capability_string_argument(arguments, "method")
            .unwrap_or_else(|| "get".to_string())
            .to_ascii_uppercase();
        let read_only = method == "GET";
        let required_inputs = Self::capability_acquire_required_inputs(arguments);
        let mut parameters = Vec::new();
        let mut body_required = false;
        for input in required_inputs {
            if input.eq_ignore_ascii_case("body") {
                body_required = true;
                continue;
            }
            let location = if path.contains(&format!("{{{}}}", input))
                || path.contains(&format!(":{}", input))
            {
                crate::custom_apis::CustomApiParameterLocation::Path
            } else if read_only {
                crate::custom_apis::CustomApiParameterLocation::Query
            } else {
                body_required = true;
                continue;
            };
            parameters.push(crate::custom_apis::CustomApiParameter {
                name: input,
                location,
                required: true,
                description: None,
                schema_type: Some("string".to_string()),
            });
        }
        if !read_only && method != "DELETE" {
            let has_body_template = arguments
                .get("body_template")
                .is_some_and(|value| !value.is_null());
            body_required = body_required || has_body_template;
            parameters.push(crate::custom_apis::CustomApiParameter {
                name: "body".to_string(),
                location: crate::custom_apis::CustomApiParameterLocation::Body,
                required: body_required,
                description: Some("JSON request body for this endpoint".to_string()),
                schema_type: Some("object".to_string()),
            });
        }
        let operation_id = Self::normalize_generated_action_name(&format!("{} {}", method, path));
        let response_notes = Self::capability_string_argument(arguments, "response_notes");
        let operation_description = response_notes
            .filter(|notes| !notes.eq_ignore_ascii_case(description))
            .map(|notes| format!("{} {}", description.trim(), notes.trim()))
            .unwrap_or_else(|| description.trim().to_string());
        Ok((
            base_url,
            crate::custom_apis::CustomApiOperationDraft {
                id: if operation_id.is_empty() {
                    format!("{}-request", name)
                } else {
                    operation_id
                },
                name: format!("{} {}", method, path),
                method,
                path,
                description: operation_description,
                read_only,
                enabled: true,
                default_headers: Self::capability_object_to_string_map(
                    arguments.get("default_headers"),
                ),
                default_query,
                parameters,
                body_required,
            },
            Self::capability_object_to_string_map(arguments.get("default_headers")),
        ))
    }

    fn capability_auth_fields(
        arguments: &serde_json::Value,
    ) -> (
        crate::custom_apis::CustomApiAuthMode,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let mode =
            Self::capability_auth_mode(arguments).unwrap_or(crate::custom_apis::CustomApiAuthMode::None);
        let header = Self::capability_string_argument(arguments, "auth_header_name");
        match mode {
            crate::custom_apis::CustomApiAuthMode::Bearer
            | crate::custom_apis::CustomApiAuthMode::OAuth2 => (
                mode,
                Some(header.unwrap_or_else(|| "Authorization".to_string())),
                None,
                None,
            ),
            crate::custom_apis::CustomApiAuthMode::ApiKeyHeader => (
                mode,
                None,
                Some(header.unwrap_or_else(|| "X-API-Key".to_string())),
                None,
            ),
            crate::custom_apis::CustomApiAuthMode::ApiKeyQuery => (mode, None, header, None),
            crate::custom_apis::CustomApiAuthMode::Basic => (mode, None, None, None),
            crate::custom_apis::CustomApiAuthMode::None => (mode, None, None, None),
        }
    }

    async fn execute_capability_acquire_custom_api(
        &self,
        arguments: &serde_json::Value,
        name: &str,
        description: &str,
    ) -> Result<String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is required to save custom integrations"))?;
        let allow_duplicate = arguments
            .get("allow_duplicate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let (mut request, operation_count) =
            if Self::capability_string_argument(arguments, "openapi_url").is_some()
                || Self::capability_string_argument(arguments, "openapi_text").is_some()
            {
                let preview = crate::custom_apis::preview_custom_api(
                    crate::custom_apis::CustomApiPreviewRequest {
                        name: Some(name.to_string()),
                        base_url: Self::capability_string_argument(arguments, "base_url"),
                        openapi_url: Self::capability_string_argument(arguments, "openapi_url"),
                        openapi_text: Self::capability_string_argument(arguments, "openapi_text"),
                        curl_text: None,
                    },
                )
                .await?;
                let auth_mode = Self::capability_auth_mode(arguments).unwrap_or(preview.auth_mode);
                let auth_header = Self::capability_string_argument(arguments, "auth_header_name")
                    .or(preview.auth_header);
                let auth_name = if matches!(
                    auth_mode,
                    crate::custom_apis::CustomApiAuthMode::ApiKeyHeader
                        | crate::custom_apis::CustomApiAuthMode::ApiKeyQuery
                ) {
                    auth_header.clone().or(preview.auth_name)
                } else {
                    preview.auth_name
                };
                let operation_count = preview.operations.len();
                (
                    crate::custom_apis::CustomApiUpsertRequest {
                        id: Some(name.to_string()),
                        name: preview.suggested_name,
                        description: Some(description.to_string()),
                        base_url: preview.base_url,
                        enabled: Some(true),
                        auth_mode: Some(auth_mode),
                        auth_profile_id: None,
                        auth_header,
                        auth_name,
                        auth_username: preview.auth_username,
                        secret: None,
                        clear_secret: None,
                        allow_missing_secret: Some(true),
                        operations: preview.operations,
                    },
                    operation_count,
                )
            } else {
                let (base_url, operation, _) =
                    Self::capability_operation_draft(arguments, name, description)?;
                let (auth_mode, auth_header, auth_name, auth_username) =
                    Self::capability_auth_fields(arguments);
                (
                    crate::custom_apis::CustomApiUpsertRequest {
                        id: Some(name.to_string()),
                        name: name.to_string(),
                        description: Some(description.to_string()),
                        base_url,
                        enabled: Some(true),
                        auth_mode: Some(auth_mode),
                        auth_profile_id: None,
                        auth_header,
                        auth_name,
                        auth_username,
                        secret: None,
                        clear_secret: None,
                        allow_missing_secret: Some(true),
                        operations: vec![operation],
                    },
                    1,
                )
            };

        if allow_duplicate {
            let existing = crate::custom_apis::list_custom_apis(
                storage,
                &self.config_dir,
                self.data_dir(),
            )
            .await?;
            if existing.iter().any(|item| item.config.id == name) {
                request.id = Some(format!("{}-{}", name, uuid::Uuid::new_v4().simple()));
            }
        }
        let request_id = request.id.clone().unwrap_or_else(|| name.to_string());
        let existing = crate::custom_apis::list_custom_apis(
            storage,
            &self.config_dir,
            self.data_dir(),
        )
        .await?
        .into_iter()
        .any(|item| item.config.id == request_id);
        let path_id = if existing && !allow_duplicate {
            Some(request_id.as_str())
        } else {
            None
        };
        let view = crate::custom_apis::upsert_custom_api(
            storage,
            &self.config_dir,
            self.data_dir(),
            self,
            request,
            path_id,
        )
        .await?;
        let mut lines = vec![
            format!("Custom API integration `{}` saved.", view.config.name),
            "It is available in Settings > Integrations under custom API integrations.".to_string(),
            format!("Registered API actions: {}", view.action_count),
            format!("Endpoint base URL: {}", view.config.base_url),
        ];
        if operation_count != view.action_count {
            lines.push(format!(
                "Imported {} operation(s); {} are enabled.",
                operation_count, view.action_count
            ));
        }
        if !matches!(
            view.config.auth_mode,
            crate::custom_apis::CustomApiAuthMode::None
        ) && !view.secret_configured
        {
            lines.push(
                "Authentication still needs to be configured from the custom API integration settings."
                    .to_string(),
            );
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
        self.validate_connector_request_url(&spec.url).await?;

        let retry = spec.retry.normalized();
        let timeout_secs = spec.timeout_secs.clamp(1, 300);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
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

    fn action_scope_hint_for_loaded_action(
        _action_name: &str,
        loaded: &LoadedAction,
    ) -> ActionScopeHint {
        ActionScopeHint {
            mcp_server_id: loaded
                .mcp_binding
                .as_ref()
                .map(|binding| binding.server_id.clone()),
            custom_api_id: loaded
                .custom_api_binding
                .as_ref()
                .map(|binding| binding.api_id.clone()),
            integration_ids: loaded.info.authorization.access.integration_ids.clone(),
            extension_pack_ids: {
                let mut ids = loaded.info.authorization.access.extension_pack_ids.clone();
                if let Some(binding) = loaded.extension_pack_binding.as_ref() {
                    if !ids.iter().any(|value| value == &binding.pack_id) {
                        ids.push(binding.pack_id.clone());
                    }
                }
                ids
            },
            requires_ssh_connection: loaded.info.authorization.access.requires_ssh_connection,
            channel_targets: loaded.info.authorization.access.channel_targets.clone(),
        }
    }

    fn fallback_action_scope_hint(_action_name: &str) -> ActionScopeHint {
        ActionScopeHint::default()
    }

    fn normalize_scope_channel_target(value: Option<&str>, default_target: &str) -> String {
        match value
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .map(|raw| raw.to_ascii_lowercase())
        {
            Some(channel) if matches!(channel.as_str(), "push" | "auto" | "default") => {
                "preferred".to_string()
            }
            Some(channel)
                if matches!(
                    channel.as_str(),
                    "app" | "app_notification" | "app_notifications" | "in_app"
                ) =>
            {
                String::new()
            }
            Some(channel) if channel == "http" => "web".to_string(),
            Some(channel) => channel,
            None => default_target.to_string(),
        }
    }

    fn scoped_channel_target_for_hint(
        hint: &ActionScopeHint,
        arguments: &serde_json::Value,
    ) -> Option<String> {
        let target = hint.channel_targets.first()?;
        Some(Self::normalize_scope_channel_target(
            arguments
                .get(target.argument_key.as_str())
                .and_then(|value| value.as_str()),
            target.default_target.as_str(),
        ))
    }

    fn uses_broad_network(action: &ActionDef) -> bool {
        let outbound = &action.authorization.outbound;
        outbound.outbound_write || outbound.public_publish
    }

    fn builtin_dangerous_permissions(
        action: &ActionDef,
    ) -> Vec<crate::security::action_guard::Permission> {
        crate::security::action_guard::ActionGuard::permissions_from_capabilities(
            &action.capabilities,
        )
        .into_iter()
        .filter(|permission| {
            !matches!(
                permission,
                crate::security::action_guard::Permission::Custom(_)
            ) && Self::permission_needs_agent_approval(permission)
        })
        .collect()
    }

    fn permission_needs_agent_approval(
        permission: &crate::security::action_guard::Permission,
    ) -> bool {
        crate::security::action_guard::ActionGuard::permission_risk(permission)
            == crate::security::action_guard::PermissionRisk::Dangerous
    }

    pub fn action_permission_ids(action: &ActionDef) -> Vec<String> {
        let mut permission_ids = action.authorization.access.permission_ids.clone();
        permission_ids.extend(
            Self::builtin_dangerous_permissions(action)
                .into_iter()
                .map(|permission| permission.to_string()),
        );
        if !action.authorization.access.channel_targets.is_empty() {
            permission_ids.push("messaging_send".to_string());
        }
        permission_ids
            .into_iter()
            .map(|permission| permission.trim().to_ascii_lowercase())
            .filter(|permission| !permission.is_empty())
            .collect()
    }

    fn action_demands_broad_network_consent(action: &ActionDef) -> bool {
        Self::uses_broad_network(action)
            && !action
                .authorization
                .access
                .permission_ids
                .iter()
                .any(|permission| permission.trim().eq_ignore_ascii_case("broad_network"))
    }

    pub fn action_required_agent_permission_ids(action: &ActionDef) -> Vec<String> {
        let mut permission_ids = Self::action_permission_ids(action);
        if Self::action_demands_broad_network_consent(action) {
            permission_ids.push("broad_network".to_string());
        }
        permission_ids.sort();
        permission_ids.dedup();
        permission_ids
    }

    fn scope_contains_exact_value(allowed: &[String], candidate: &str) -> bool {
        let candidate = candidate.trim();
        allowed.iter().any(|value| value.trim() == candidate)
    }

    fn scope_contains_case_insensitive_value(allowed: &[String], candidate: &str) -> bool {
        let candidate = candidate.trim();
        allowed
            .iter()
            .any(|value| value.trim().eq_ignore_ascii_case(candidate))
    }

    fn scope_contains_channel_target(allowed: &[String], candidate: &str) -> bool {
        let candidate = candidate.trim().to_ascii_lowercase();
        allowed.iter().any(|value| {
            Self::normalize_scope_channel_target(Some(value.as_str()), "")
                .trim()
                .eq_ignore_ascii_case(candidate.as_str())
        })
    }

    fn scoped_actor_label(auth_context: &ActionAuthorizationContext) -> String {
        auth_context
            .agent_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("Agent '{}'", value))
            .unwrap_or_else(|| "This agent".to_string())
    }

    async fn authorize_action_scope(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Option<ActionAuthorizationDecision> {
        let scope = auth_context.agent_access_scope.as_ref()?;
        let hint = {
            let actions = self.actions.read().await;
            actions
                .get(action_name)
                .map(|loaded| Self::action_scope_hint_for_loaded_action(action_name, loaded))
        }
        .unwrap_or_else(|| Self::fallback_action_scope_hint(action_name));
        let actor = Self::scoped_actor_label(auth_context);

        if let Some(server_id) = hint.mcp_server_id.as_deref() {
            if !Self::scope_contains_exact_value(&scope.mcp_server_ids, server_id) {
                return Some(ActionAuthorizationDecision::deny(format!(
                    "{} is not allowed to use MCP server '{}'.",
                    actor, server_id
                )));
            }
        }

        if let Some(api_id) = hint.custom_api_id.as_deref() {
            if !Self::scope_contains_exact_value(&scope.custom_api_ids, api_id) {
                return Some(ActionAuthorizationDecision::deny(format!(
                    "{} is not allowed to use custom API '{}'.",
                    actor, api_id
                )));
            }
        }

        if !hint.integration_ids.is_empty()
            && !hint.integration_ids.iter().any(|integration_id| {
                Self::scope_contains_case_insensitive_value(&scope.integration_ids, integration_id)
            })
        {
            return Some(ActionAuthorizationDecision::deny(format!(
                "{} is not allowed to use integration(s): {}.",
                actor,
                hint.integration_ids.join(", ")
            )));
        }

        if !hint.extension_pack_ids.is_empty()
            && !hint.extension_pack_ids.iter().any(|pack_id| {
                Self::scope_contains_case_insensitive_value(&scope.extension_pack_ids, pack_id)
            })
        {
            return Some(ActionAuthorizationDecision::deny(format!(
                "{} is not allowed to use extension pack(s): {}.",
                actor,
                hint.extension_pack_ids.join(", ")
            )));
        }

        if hint.requires_ssh_connection {
            if scope.ssh_connection_names.is_empty() {
                return Some(ActionAuthorizationDecision::deny(format!(
                    "{} is not allowed to use SSH because no SSH connections are attached.",
                    actor
                )));
            }
            if action_name == "ssh" {
                if let Some(connection_name) = arguments
                    .get("connection")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if !Self::scope_contains_exact_value(
                        &scope.ssh_connection_names,
                        connection_name,
                    ) {
                        return Some(ActionAuthorizationDecision::deny(format!(
                            "{} is not allowed to use SSH connection '{}'.",
                            actor, connection_name
                        )));
                    }
                }
            }
        }

        if !hint.channel_targets.is_empty() {
            let channel_target = Self::scoped_channel_target_for_hint(&hint, arguments)
                .unwrap_or_else(|| "preferred".to_string());
            if !channel_target.is_empty() {
                if channel_target == "preferred" {
                    if scope.channel_ids.is_empty() {
                        return Some(ActionAuthorizationDecision::deny(format!(
                            "{} is not allowed to use preferred-channel delivery because no messaging channels are attached.",
                            actor
                        )));
                    }
                } else if !Self::scope_contains_channel_target(&scope.channel_ids, &channel_target)
                {
                    return Some(ActionAuthorizationDecision::deny(format!(
                        "{} is not allowed to use messaging channel '{}'.",
                        actor, channel_target
                    )));
                }
            }
        }

        None
    }

    async fn is_action_integration_ready(&self, action: &ActionDef) -> bool {
        let access = &action.authorization.access;
        let integration_ids = &access.integration_ids;
        let extension_pack_ids = &access.extension_pack_ids;
        if integration_ids.is_empty() && extension_pack_ids.is_empty() {
            return true;
        }
        if !integration_ids.is_empty() {
            let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
            let workspace_granted_bundles =
                if access.integration_features.contains_key("google_workspace") {
                    Some(
                        crate::actions::google_workspace::granted_bundles(&self.config_dir)
                            .unwrap_or_default(),
                    )
                } else {
                    None
                };
            for integration_id in integration_ids {
                if !manager.is_ready(integration_id).await {
                    continue;
                }
                let Some(features) = access.integration_features.get(integration_id) else {
                    return true;
                };
                if features.is_empty() {
                    return true;
                }
                let features_ready = match integration_id.as_str() {
                    "google_workspace" => {
                        workspace_granted_bundles.as_ref().is_some_and(|granted| {
                            features.iter().all(|feature| {
                                crate::actions::google_workspace::normalize_bundle_id(feature)
                                    .is_some_and(|normalized| {
                                        granted
                                            .iter()
                                            .any(|granted_bundle| granted_bundle == &normalized)
                                    })
                            })
                        })
                    }
                    _ => true,
                };
                if features_ready {
                    return true;
                }
            }
        }
        if !extension_pack_ids.is_empty() {
            let Some(registry) = self.extension_pack_registry.as_ref() else {
                return false;
            };
            let guard = registry.read().await;
            for pack_id in extension_pack_ids {
                let Ok(Some(pack)) = guard.get_pack(pack_id).await else {
                    continue;
                };
                if pack.enabled
                    && pack.installed
                    && matches!(pack.status.as_str(), "ready" | "connected")
                {
                    return true;
                }
            }
        }
        false
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
            use rand::RngExt;
            let span = ((backoff_ms as f64) * jitter_ratio).round() as i64;
            if span <= 0 {
                backoff_ms.max(25)
            } else {
                let mut rng = rand::rng();
                let jitter = rng.random_range(-span..=span);
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
                ));
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

    fn pdf_text_literal(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '(' => out.push_str("\\("),
                ')' => out.push_str("\\)"),
                '\t' => out.push(' '),
                ch if ch.is_ascii_graphic() || ch == ' ' => out.push(ch),
                _ => out.push(' '),
            }
        }
        out
    }

    fn wrap_pdf_text(text: &str, max_chars: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for raw_line in text.lines() {
            let mut current = String::new();
            for word in raw_line.split_whitespace() {
                let separator = usize::from(!current.is_empty());
                if !current.is_empty() && current.len() + separator + word.len() > max_chars {
                    lines.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
            if current.is_empty() {
                lines.push(String::new());
            } else {
                lines.push(current);
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    fn generate_simple_pdf_bytes(title: &str, content: &str, style: &str) -> Vec<u8> {
        const PAGE_WIDTH: usize = 612;
        const PAGE_HEIGHT: usize = 792;
        const LINES_PER_PAGE: usize = 42;
        let body_font = match style {
            "invoice" => 10,
            "report" | "letter" | "plain" => 11,
            _ => 11,
        };
        let title_font = match style {
            "invoice" => 20,
            "report" => 16,
            "letter" | "plain" => 14,
            _ => 14,
        };
        let mut lines = Vec::new();
        lines.push(title.trim().to_string());
        lines.push(String::new());
        lines.extend(Self::wrap_pdf_text(content, 92));
        let pages = lines.chunks(LINES_PER_PAGE).collect::<Vec<_>>();
        let page_count = pages.len().max(1);
        let catalog_id = 1usize;
        let pages_id = 2usize;
        let font_id = 3usize;
        let first_page_id = 4usize;
        let mut objects: Vec<String> = Vec::new();
        objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R >>\nendobj\n"
        ));
        let kids = (0..page_count)
            .map(|index| format!("{} 0 R", first_page_id + index * 2))
            .collect::<Vec<_>>()
            .join(" ");
        objects.push(format!(
            "{pages_id} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {page_count} >>\nendobj\n"
        ));
        objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n"
        ));
        for (index, page_lines) in pages.iter().enumerate() {
            let page_id = first_page_id + index * 2;
            let content_id = page_id + 1;
            let mut stream = String::from("BT\n72 740 Td\n");
            for (line_index, line) in page_lines.iter().enumerate() {
                if line_index == 0 && index == 0 {
                    stream.push_str(&format!("/F1 {title_font} Tf\n"));
                } else if line_index == 1 && index == 0 {
                    stream.push_str(&format!("/F1 {body_font} Tf\n"));
                }
                if line_index > 0 {
                    stream.push_str("0 -16 Td\n");
                }
                stream.push_str(&format!("({}) Tj\n", Self::pdf_text_literal(line)));
            }
            stream.push_str("ET\n");
            objects.push(format!(
                "{page_id} 0 obj\n<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>\nendobj\n"
            ));
            objects.push(format!(
                "{content_id} 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
                stream.as_bytes().len(),
                stream
            ));
        }

        let mut pdf = String::from("%PDF-1.4\n%\u{00e2}\u{00e3}\u{00cf}\u{00d3}\n");
        let mut offsets = vec![0usize];
        for object in &objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_offset = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()));
        for offset in offsets.iter().skip(1) {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root {catalog_id} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            offsets.len()
        ));
        pdf.into_bytes()
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
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        // For built-in actions, fall back to native with some wrapping
        match action_name {
            "http_get" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let parsed_url = self
                    .resolve_http_get_url_for_context(url, auth_context)
                    .await?;
                let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

                // Fast-path: try Lightpanda for external URLs (returns clean markdown)
                let has_custom_headers = arguments
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|h| !h.is_empty())
                    .unwrap_or(false);
                if !Self::http_get_url_is_privateish(&parsed_url) && !has_custom_headers {
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
                        if blocked && !chat_override {
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
                                    "\nSecurity warnings: {}",
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
                self.execute_capability_acquire(arguments).await
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

    #[cfg(feature = "docker")]
    fn docker_host_uses_socket_transport(host: &str) -> bool {
        let trimmed = host.trim();
        trimmed.starts_with("unix://") || trimmed.starts_with("npipe://")
    }

    /// Connect to Docker, honoring DOCKER_HOST transport instead of forcing HTTP.
    #[cfg(feature = "docker")]
    fn connect_docker() -> Result<bollard::Docker> {
        if let Ok(host) = std::env::var("DOCKER_HOST") {
            let trimmed = host.trim();
            if !trimmed.is_empty() {
                let transport = if Self::docker_host_uses_socket_transport(trimmed) {
                    "socket"
                } else {
                    "network"
                };
                tracing::debug!(
                    "Connecting to Docker via DOCKER_HOST={} ({})",
                    trimmed,
                    transport
                );
                return bollard::Docker::connect_with_defaults().map_err(|e| {
                    anyhow::anyhow!("Failed to connect to Docker at {}: {}", trimmed, e)
                });
            }
            bollard::Docker::connect_with_local_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))
        } else {
            bollard::Docker::connect_with_local_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))
        }
    }

    fn should_manage_local_sandbox_containers_for(
        role: Option<&str>,
        has_local_docker_endpoint: bool,
    ) -> bool {
        let is_control_plane = role
            .map(|value| value.trim().to_ascii_lowercase())
            .is_some_and(|value| matches!(value.as_str(), "control-plane" | "control"));
        !is_control_plane || has_local_docker_endpoint
    }

    pub(crate) fn should_manage_local_sandbox_containers() -> bool {
        #[cfg(feature = "docker")]
        {
            let role = std::env::var("AGENTARK_STACK_ROLE").ok();
            let has_local_docker_endpoint = std::env::var("DOCKER_HOST")
                .ok()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
                || Path::new("/var/run/docker.sock").exists();
            Self::should_manage_local_sandbox_containers_for(
                role.as_deref(),
                has_local_docker_endpoint,
            )
        }
        #[cfg(not(feature = "docker"))]
        {
            false
        }
    }

    pub async fn docker_available(&self) -> bool {
        #[cfg(feature = "docker")]
        {
            match Self::connect_docker() {
                Ok(docker) => docker.ping().await.is_ok(),
                Err(_) => false,
            }
        }
        #[cfg(not(feature = "docker"))]
        {
            let _ = self;
            true
        }
    }

    #[cfg(feature = "docker")]
    fn docker_security_opts() -> Vec<String> {
        let mut opts = vec!["no-new-privileges:true".to_string()];
        if let Ok(profile) = std::env::var("AGENTARK_DOCKER_SECCOMP_PROFILE") {
            let trimmed = profile.trim();
            if !trimmed.is_empty() {
                opts.push(format!("seccomp={}", trimmed));
            }
        }
        if let Ok(profile) = std::env::var("AGENTARK_DOCKER_APPARMOR_PROFILE") {
            let trimmed = profile.trim();
            if !trimmed.is_empty() {
                opts.push(format!("apparmor={}", trimmed));
            }
        }
        opts
    }

    #[cfg(feature = "docker")]
    fn sandbox_container_labels(
        action_name: &str,
        isolation: ContainerIsolation,
    ) -> HashMap<String, String> {
        HashMap::from([
            (
                AGENTARK_SANDBOX_LABEL_KEY.to_string(),
                AGENTARK_SANDBOX_LABEL_VALUE.to_string(),
            ),
            ("agentark.action".to_string(), action_name.to_string()),
            (
                "agentark.isolation".to_string(),
                isolation.label().to_string(),
            ),
            (
                "agentark.network_access".to_string(),
                if isolation.network_access() {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string(),
            ),
            (
                "agentark.created_at".to_string(),
                chrono::Utc::now().to_rfc3339(),
            ),
        ])
    }

    #[cfg(feature = "docker")]
    async fn remember_active_container(&self, id: &str) {
        let mut active = self.active_sandbox_containers.write().await;
        active.insert(id.to_string());
        crate::metrics::set_active_containers(active.len());
    }

    #[cfg(feature = "docker")]
    async fn forget_active_container(&self, id: &str) {
        let mut active = self.active_sandbox_containers.write().await;
        active.remove(id);
        crate::metrics::set_active_containers(active.len());
    }

    #[cfg(feature = "docker")]
    async fn update_container_reaper_status(&self, removed: u64, error: Option<String>) {
        let mut status = self.container_reaper_status.write().await;
        status.last_run_at = Some(chrono::Utc::now().to_rfc3339());
        status.last_removed_count = removed;
        status.total_removed_count = status.total_removed_count.saturating_add(removed);
        status.last_error = error;
    }

    pub async fn active_container_count(&self) -> usize {
        #[cfg(feature = "docker")]
        {
            return self.active_sandbox_containers.read().await.len();
        }
        #[cfg(not(feature = "docker"))]
        {
            0
        }
    }

    pub async fn container_reaper_status(&self) -> ContainerReaperStatus {
        #[cfg(feature = "docker")]
        {
            return self.container_reaper_status.read().await.clone();
        }
        #[cfg(not(feature = "docker"))]
        {
            ContainerReaperStatus::default()
        }
    }

    pub async fn reconcile_orphan_containers(&self) -> Result<u64> {
        #[cfg(feature = "docker")]
        {
            if !Self::should_manage_local_sandbox_containers() {
                tracing::debug!(
                    "Skipping local sandbox container reconciliation on control plane without a local Docker endpoint"
                );
                self.update_container_reaper_status(0, None).await;
                crate::metrics::record_container_sweeper_run("skipped", 0);
                return Ok(0);
            }

            let docker = match Self::connect_docker() {
                Ok(docker) => docker,
                Err(error) => {
                    let message = error.to_string();
                    self.update_container_reaper_status(0, Some(message.clone()))
                        .await;
                    crate::metrics::record_container_sweeper_run("error", 0);
                    return Err(error);
                }
            };

            let filters = HashMap::from([(
                "label".to_string(),
                vec![format!(
                    "{}={}",
                    AGENTARK_SANDBOX_LABEL_KEY, AGENTARK_SANDBOX_LABEL_VALUE
                )],
            )]);
            let containers = docker
                .list_containers(Some(bollard::query_parameters::ListContainersOptions {
                    all: true,
                    filters: Some(filters),
                    ..Default::default()
                }))
                .await?;
            let active = self.active_sandbox_containers.read().await.clone();
            let mut removed = 0u64;
            for container in containers {
                let Some(id) = container.id.as_deref() else {
                    continue;
                };
                if active.contains(id) {
                    continue;
                }
                Self::force_remove_container(&docker, id).await;
                removed = removed.saturating_add(1);
            }
            if let Err(error) = self.prune_stale_code_execute_artifacts().await {
                tracing::warn!(
                    "Failed to prune stale code execution artifacts during runtime reconciliation: {}",
                    error
                );
            }
            self.update_container_reaper_status(removed, None).await;
            crate::metrics::record_container_sweeper_run("ok", removed);
            Ok(removed)
        }
        #[cfg(not(feature = "docker"))]
        {
            Ok(0)
        }
    }

    async fn prune_stale_path_entries(&self, root: &Path, max_age_secs: u64) -> Result<u64> {
        let mut removed = 0u64;
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(max_age_secs))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut entries = match tokio::fs::read_dir(root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let modified = match metadata.modified() {
                Ok(modified) => modified,
                Err(_) => continue,
            };
            if modified > cutoff {
                continue;
            }
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(&path).await?;
                removed = removed.saturating_add(1);
            } else {
                tokio::fs::remove_file(&path).await?;
                removed = removed.saturating_add(1);
            }
        }

        Ok(removed)
    }

    async fn prune_stale_native_code_execute_temp_dirs(&self) -> Result<u64> {
        let temp_root = std::env::temp_dir();
        let mut removed = 0u64;
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(
                CODE_EXECUTE_NATIVE_TEMP_RETENTION_SECS,
            ))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut entries = match tokio::fs::read_dir(&temp_root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            if !filename.starts_with("agentark-exec-") {
                continue;
            }
            let path = entry.path();
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if !metadata.is_dir() {
                continue;
            }
            let modified = match metadata.modified() {
                Ok(modified) => modified,
                Err(_) => continue,
            };
            if modified > cutoff {
                continue;
            }
            tokio::fs::remove_dir_all(&path).await?;
            removed = removed.saturating_add(1);
        }
        Ok(removed)
    }

    async fn prune_stale_code_execute_artifacts(&self) -> Result<u64> {
        let mut removed = 0u64;
        removed = removed.saturating_add(
            self.prune_stale_path_entries(
                &self.data_dir().join("outputs"),
                CODE_EXECUTE_OUTPUT_RETENTION_SECS,
            )
            .await?,
        );
        removed = removed.saturating_add(self.prune_stale_native_code_execute_temp_dirs().await?);
        Ok(removed)
    }

    fn docker_required_error(action_name: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "Docker is required for '{}' execution but is not available",
            action_name
        )
    }

    /// Execute an action in Docker sandbox
    async fn execute_docker(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        if let Some(_executor) = Self::control_plane_executor_client() {
            if action_name == "code_execute" {
                return self.execute_code_remote(arguments, auth_context).await;
            }
            if action_name == "shell" {
                let command = arguments["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing command"))?;
                let shell_arguments = serde_json::json!({
                    "language": "bash",
                    "code": command,
                    "timeout_secs": arguments
                        .get("timeout_secs")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(30),
                    "network_access": false,
                });
                return self
                    .execute_code_remote(&shell_arguments, auth_context)
                    .await;
            }
        }
        #[cfg(feature = "docker")]
        {
            // Check Docker availability first - fall back to native if unavailable
            let docker_available = Self::connect_docker().is_ok();

            if !docker_available {
                tracing::warn!(
                    "Docker not available for '{}'; refusing unsandboxed fallback execution",
                    action_name
                );
                return Err(Self::docker_required_error(action_name));
            }

            match action_name {
                "shell" => {
                    const PUBLIC_SHELL_SANDBOX_IMAGE: &str = "alpine:3.20";
                    self.run_isolated_container(
                        action_name,
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
                "code_execute" => self.execute_code_docker(arguments, auth_context).await,
                _ => Err(anyhow::anyhow!("Unknown docker action: {}", action_name)),
            }
        }

        #[cfg(not(feature = "docker"))]
        {
            let _ = arguments;
            Err(Self::docker_required_error(action_name))
        }
    }

    /// Force-remove a Docker container (stop + remove), ignoring errors.
    /// Guaranteed to not leave containers behind.
    #[cfg(feature = "docker")]
    async fn force_remove_container(docker: &bollard::Docker, id: &str) {
        // Kill first (faster than stop for stuck containers)
        let _ = docker.kill_container(id, None).await;
        // Stop as fallback (handles already-stopped containers)
        let _ = docker
            .stop_container(
                id,
                Some(bollard::query_parameters::StopContainerOptions {
                    t: Some(0),
                    ..Default::default()
                }),
            )
            .await;
        // Force remove - deletes container, volumes, and anonymous volumes
        let _ = docker
            .remove_container(
                id,
                Some(bollard::query_parameters::RemoveContainerOptions {
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
            Some(bollard::query_parameters::CreateImageOptions {
                from_image: Some(image.to_string()),
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
    /// Container is ALWAYS destroyed after execution - no leftovers.
    #[cfg(feature = "docker")]
    async fn run_isolated_container(
        &self,
        action_name: &str,
        image: &str,
        cmd: Vec<String>,
        env: Option<Vec<String>>,
        timeout_secs: u64,
        isolation: ContainerIsolation,
    ) -> Result<String> {
        let docker = Self::connect_docker()?;
        let isolation_label = isolation.label();
        let network_access = isolation.network_access();

        // Auto-pull image if not available
        Self::ensure_image(&docker, image).await?;

        let security_opt = Self::docker_security_opts();
        let host_config = match isolation {
            ContainerIsolation::Strict => bollard::models::HostConfig {
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
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(security_opt.clone()),
                auto_remove: Some(false),
                ..Default::default()
            },
            ContainerIsolation::Standard => bollard::models::HostConfig {
                memory: Some(512 * 1024 * 1024),
                memory_swap: Some(512 * 1024 * 1024),
                cpu_period: Some(100_000),
                cpu_quota: Some(50_000),
                pids_limit: Some(128),
                network_mode: Some("none".to_string()),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(security_opt.clone()),
                auto_remove: Some(false),
                ..Default::default()
            },
            ContainerIsolation::StandardWithNetwork => bollard::models::HostConfig {
                memory: Some(512 * 1024 * 1024),
                memory_swap: Some(512 * 1024 * 1024),
                cpu_period: Some(100_000),
                cpu_quota: Some(50_000),
                pids_limit: Some(128),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(security_opt),
                auto_remove: Some(false),
                ..Default::default()
            },
        };

        let network_disabled = !network_access;

        let container_config = bollard::models::ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            env,
            labels: Some(Self::sandbox_container_labels(action_name, isolation)),
            host_config: Some(host_config),
            network_disabled: Some(network_disabled),
            working_dir: Some(
                if matches!(
                    isolation,
                    ContainerIsolation::Standard | ContainerIsolation::StandardWithNetwork
                ) {
                    CODE_EXECUTE_SANDBOX_DIR
                } else {
                    "/tmp"
                }
                .to_string(),
            ),
            ..Default::default()
        };

        let create_started = std::time::Instant::now();
        let container = docker.create_container(None, container_config).await;
        let container = match container {
            Ok(container) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "create",
                    isolation_label,
                    network_access,
                    "ok",
                    create_started.elapsed(),
                );
                container
            }
            Err(e) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "create",
                    isolation_label,
                    network_access,
                    "error",
                    create_started.elapsed(),
                );
                crate::metrics::observe_container_run(
                    action_name,
                    isolation_label,
                    network_access,
                    "error",
                );
                return Err(anyhow::anyhow!("Failed to create container: {}", e));
            }
        };

        let container_id = container.id.clone();
        self.remember_active_container(&container_id).await;
        tracing::info!(
            "Created isolated container {} for {}",
            &container_id[..12.min(container_id.len())],
            action_name
        );

        // Start container - if this fails, clean up immediately
        let start_started = std::time::Instant::now();
        if let Err(e) = docker.start_container(&container_id, None).await {
            crate::metrics::observe_container_lifecycle(
                action_name,
                "start",
                isolation_label,
                network_access,
                "error",
                start_started.elapsed(),
            );
            let cleanup_started = std::time::Instant::now();
            Self::force_remove_container(&docker, &container_id).await;
            crate::metrics::observe_container_lifecycle(
                action_name,
                "cleanup",
                isolation_label,
                network_access,
                "ok",
                cleanup_started.elapsed(),
            );
            self.forget_active_container(&container_id).await;
            crate::metrics::observe_container_run(
                action_name,
                isolation_label,
                network_access,
                "error",
            );
            return Err(anyhow::anyhow!("Failed to start container: {}", e));
        }
        crate::metrics::observe_container_lifecycle(
            action_name,
            "start",
            isolation_label,
            network_access,
            "ok",
            start_started.elapsed(),
        );

        let wait_started = std::time::Instant::now();
        let exit_code = match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            docker
                .wait_container(
                    &container_id,
                    None::<bollard::query_parameters::WaitContainerOptions>,
                )
                .try_collect::<Vec<_>>(),
        )
        .await
        {
            Ok(Ok(statuses)) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "ok",
                    wait_started.elapsed(),
                );
                let code = statuses
                    .last()
                    .map(|status| status.status_code)
                    .unwrap_or(0);
                tracing::debug!(
                    "Container {} exited with code {}",
                    &container_id[..12.min(container_id.len())],
                    code
                );
                code
            }
            Ok(Err(bollard::errors::Error::DockerContainerWaitError { code, error })) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "error",
                    wait_started.elapsed(),
                );
                if !error.trim().is_empty() {
                    tracing::debug!(
                        "Container {} exited with wait error {}: {}",
                        &container_id[..12.min(container_id.len())],
                        code,
                        error
                    );
                }
                code
            }
            Ok(Err(e)) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "error",
                    wait_started.elapsed(),
                );
                let cleanup_started = std::time::Instant::now();
                Self::force_remove_container(&docker, &container_id).await;
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "cleanup",
                    isolation_label,
                    network_access,
                    "ok",
                    cleanup_started.elapsed(),
                );
                self.forget_active_container(&container_id).await;
                crate::metrics::observe_container_run(
                    action_name,
                    isolation_label,
                    network_access,
                    "error",
                );
                return Err(anyhow::anyhow!("Container wait failed: {}", e));
            }
            Err(_) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "timeout",
                    wait_started.elapsed(),
                );
                let cleanup_started = std::time::Instant::now();
                Self::force_remove_container(&docker, &container_id).await;
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "cleanup",
                    isolation_label,
                    network_access,
                    "ok",
                    cleanup_started.elapsed(),
                );
                self.forget_active_container(&container_id).await;
                crate::metrics::observe_container_run(
                    action_name,
                    isolation_label,
                    network_access,
                    "timeout",
                );
                return Err(anyhow::anyhow!(
                    "Code execution timed out after {} seconds",
                    timeout_secs
                ));
            }
        };

        // Collect stdout and stderr before cleanup
        let logs_started = std::time::Instant::now();
        let logs = docker
            .logs(
                &container_id,
                Some(bollard::query_parameters::LogsOptions {
                    stdout: true,
                    stderr: true,
                    ..Default::default()
                }),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap_or_default();
        crate::metrics::observe_container_lifecycle(
            action_name,
            "logs",
            isolation_label,
            network_access,
            "ok",
            logs_started.elapsed(),
        );

        // Always destroy the container - no leftovers
        let cleanup_started = std::time::Instant::now();
        Self::force_remove_container(&docker, &container_id).await;
        crate::metrics::observe_container_lifecycle(
            action_name,
            "cleanup",
            isolation_label,
            network_access,
            "ok",
            cleanup_started.elapsed(),
        );
        self.forget_active_container(&container_id).await;

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
        crate::metrics::observe_container_run(
            action_name,
            isolation_label,
            network_access,
            if exit_code == 0 { "ok" } else { "error" },
        );

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
        // {file} is replaced with the sandbox source path at runtime.
        // {sandbox_dir} is replaced with the writable execution workspace.
        match lang {
            // Interpreted
            "python" | "python3" | "py" => Some((
                "python:3-slim",
                "py",
                None,
                "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 {file}",
            )),
            "javascript" | "js" | "node" => Some(("node:22-slim", "js", None, "node {file}")),
            "typescript" | "ts" => Some((
                "node:22-slim",
                "ts",
                Some("npm i -g tsx 2>/dev/null"),
                "npx tsx {file}",
            )),
            "bash" | "sh" | "shell" => Some(("bash:latest", "sh", None, "bash {file}")),
            "ruby" | "rb" => Some(("ruby:3-slim", "rb", None, "ruby {file}")),
            "php" => Some(("php:8-cli", "php", None, "php {file}")),
            "perl" | "pl" => Some(("perl:5-slim", "pl", None, "perl {file}")),
            "lua" => Some(("nickblah/lua:5.4", "lua", None, "lua {file}")),
            "r" | "rlang" => Some(("r-base:latest", "R", None, "Rscript {file}")),

            // Compiled
            "java" => Some((
                "eclipse-temurin:21-jdk",
                "java",
                Some("javac {file}"),
                "java -cp {sandbox_dir} Main",
            )),
            "c" => Some((
                "gcc:latest",
                "c",
                Some("gcc {file} -o {sandbox_dir}/a.out -lm"),
                "{sandbox_dir}/a.out",
            )),
            "cpp" | "c++" => Some((
                "gcc:latest",
                "cpp",
                Some("g++ {file} -o {sandbox_dir}/a.out -lm"),
                "{sandbox_dir}/a.out",
            )),
            "go" | "golang" => Some(("golang:1-bookworm", "go", None, "go run {file}")),
            "rust" | "rs" => Some((
                "rust:1-slim-bookworm",
                "rs",
                Some("rustc {file} -o {sandbox_dir}/a.out"),
                "{sandbox_dir}/a.out",
            )),
            "swift" => Some(("swift:latest", "swift", None, "swift {file}")),
            "kotlin" | "kt" => Some((
                "zenika/kotlin:latest",
                "kt",
                Some("kotlinc {file} -include-runtime -d {sandbox_dir}/out.jar 2>/dev/null"),
                "java -jar {sandbox_dir}/out.jar",
            )),

            // Jupyter notebook - execute in-place and output results
            "jupyter" | "notebook" | "ipynb" => Some((
                "python:3-slim",
                "ipynb",
                Some(
                    "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 -m pip install --no-cache-dir -q jupyter nbconvert nbformat matplotlib pandas numpy scikit-learn seaborn 2>/dev/null",
                ),
                "jupyter nbconvert --to notebook --execute --inplace {file} 2>&1 && python3 -c \"import json; nb=json.load(open('{file}')); [print(o.get('text','')) for c in nb['cells'] for o in c.get('outputs',[]) if o.get('output_type')=='stream']\" ",
            )),

            _ => None,
        }
    }

    fn code_execute_contract_phase(arguments: &serde_json::Value) -> Option<&'static str> {
        let phase = arguments
            .get("execution_contract")
            .and_then(|value| value.get("phase"))
            .and_then(|value| value.as_str())?
            .trim()
            .to_ascii_lowercase();
        match phase.as_str() {
            "bootstrap" => Some("bootstrap"),
            "validate" => Some("validate"),
            "poll" => Some("poll"),
            _ => None,
        }
    }

    fn code_execute_contract_flag(arguments: &serde_json::Value, key: &str) -> bool {
        arguments
            .get("execution_contract")
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    fn text_contains_network_endpoint(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        if lower.contains("://") || lower.contains("localhost") || lower.contains("::1") {
            return true;
        }

        for candidate in lower.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
            let octets: Vec<&str> = candidate.split('.').collect();
            if octets.len() == 4
                && octets
                    .iter()
                    .all(|part| !part.is_empty() && part.len() <= 3 && part.parse::<u8>().is_ok())
            {
                return true;
            }
        }

        false
    }

    fn json_value_contains_network_endpoint(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::String(text) => Self::text_contains_network_endpoint(text),
            serde_json::Value::Array(values) => values
                .iter()
                .any(Self::json_value_contains_network_endpoint),
            serde_json::Value::Object(values) => values
                .values()
                .any(Self::json_value_contains_network_endpoint),
            _ => false,
        }
    }

    fn code_execute_effective_network_access(arguments: &serde_json::Value) -> bool {
        arguments
            .get("network_access")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            || Self::code_execute_contract_flag(arguments, "target_connectivity_required")
            || Self::json_value_contains_network_endpoint(arguments)
    }

    fn build_code_execute_execution_metadata(
        arguments: &serde_json::Value,
        success: bool,
        output_file_count: usize,
    ) -> serde_json::Value {
        let phase = Self::code_execute_contract_phase(arguments);
        let target_validated = success
            && Self::code_execute_contract_flag(arguments, "target_validated_when_successful");
        let explicit_ready_for_watch = success
            && Self::code_execute_contract_flag(arguments, "ready_for_watch_when_successful");
        let ready_for_watch = explicit_ready_for_watch
            || (success && phase == Some("poll"))
            || (success && phase == Some("validate") && target_validated);
        let setup_only = success && phase == Some("bootstrap") && !ready_for_watch;

        serde_json::json!({
            "phase": phase,
            "setup_only": setup_only,
            "target_validated": target_validated,
            "ready_for_watch": ready_for_watch,
            "target_connectivity_required": Self::code_execute_contract_flag(
                arguments,
                "target_connectivity_required",
            ),
            "network_access_requested": Self::code_execute_effective_network_access(arguments),
            "output_file_count": output_file_count,
        })
    }

    fn code_execute_high_risk_install_is_explicitly_approved(
        auth_context: &ActionAuthorizationContext,
    ) -> bool {
        auth_context.current_turn_is_explicit_approval
            && auth_context.direct_user_intent
            && auth_context
                .principal
                .as_ref()
                .is_some_and(|principal| principal.trusted)
            && matches!(
                auth_context.surface,
                ActionExecutionSurface::Chat | ActionExecutionSurface::Api
            )
    }

    fn detect_risky_code_execute_install_request(code: &str) -> Option<String> {
        let lower = code.to_ascii_lowercase();
        let patterns = [
            ("apt-get ", "system package manager install (`apt-get`)"),
            ("apt ", "system package manager install (`apt`)"),
            ("yum ", "system package manager install (`yum`)"),
            ("dnf ", "system package manager install (`dnf`)"),
            ("apk add", "system package manager install (`apk add`)"),
            ("pacman -s", "system package manager install (`pacman`)"),
            (
                "brew install",
                "host package manager install (`brew install`)",
            ),
            (
                "choco install",
                "host package manager install (`choco install`)",
            ),
            (
                "winget install",
                "host package manager install (`winget install`)",
            ),
            ("| sh", "piped remote shell installer"),
            ("| bash", "piped remote shell installer"),
            ("| zsh", "piped remote shell installer"),
            ("| iex", "piped remote PowerShell installer"),
            ("git+", "non-registry package source (`git+`)"),
            ("pip install http://", "direct URL package install"),
            ("pip install https://", "direct URL package install"),
            (
                "python -m pip install http://",
                "direct URL package install",
            ),
            (
                "python -m pip install https://",
                "direct URL package install",
            ),
            ("npm install http://", "direct URL package install"),
            ("npm install https://", "direct URL package install"),
            ("npm install git+", "git package install"),
            ("npm install git@", "git package install"),
            ("npm install github:", "GitHub package install"),
        ];
        for (needle, reason) in patterns {
            if lower.contains(needle) {
                return Some(reason.to_string());
            }
        }
        if (lower.contains("curl ") || lower.contains("wget "))
            && (lower.contains("| sh")
                || lower.contains("| bash")
                || lower.contains("| zsh")
                || lower.contains("| iex"))
        {
            return Some("remote script installer".to_string());
        }
        None
    }

    fn build_code_execute_dependency_metadata(
        python_packages: &[String],
        node_packages: &[String],
    ) -> serde_json::Value {
        let mut installers = Vec::new();
        if !python_packages.is_empty() {
            installers.push(serde_json::json!({
                "manager": "pip",
                "sandbox_only": true,
                "auto_allowed": true,
                "packages": python_packages,
            }));
        }
        if !node_packages.is_empty() {
            installers.push(serde_json::json!({
                "manager": "npm",
                "sandbox_only": true,
                "auto_allowed": true,
                "packages": node_packages,
            }));
        }
        serde_json::json!({ "installers": installers })
    }

    fn code_execute_dependency_summary(
        python_packages: &[String],
        node_packages: &[String],
    ) -> Option<String> {
        let mut sections = Vec::new();
        if !python_packages.is_empty() {
            sections.push(format!("pip: {}", python_packages.join(", ")));
        }
        if !node_packages.is_empty() {
            sections.push(format!("npm: {}", node_packages.join(", ")));
        }
        if sections.is_empty() {
            None
        } else {
            Some(format!(
                "AgentArk auto-installed sandbox dependencies: {}.",
                sections.join("; ")
            ))
        }
    }

    /// Detect non-stdlib Python imports and return a pip install command.
    /// Scans `import X` and `from X import` statements, filters out stdlib modules.
    fn detect_python_dep_packages(code: &str) -> Vec<String> {
        // Python stdlib modules (comprehensive but not exhaustive - errs on side of not installing)
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
                "cv2" => "opencv-python-headless".to_string(),
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

        let mut dep_list: Vec<String> = deps.into_iter().collect();
        dep_list.sort();
        dep_list
    }

    /// Detect non-builtin Node.js requires/imports and return an npm install command.
    fn detect_node_dep_packages(code: &str) -> Vec<String> {
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

        let mut dep_list: Vec<String> = deps.into_iter().collect();
        dep_list.sort();
        dep_list
    }

    /// Execute code in an isolated Docker container.
    /// Supports any language with a Docker image - auto-pulls if needed.
    /// Container is ephemeral - fully destroyed after execution.
    /// Output files (images, CSVs, etc.) are extracted before container cleanup.
    #[cfg(feature = "docker")]
    async fn execute_code_docker(
        &self,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        if let Err(error) = self.prune_stale_code_execute_artifacts().await {
            tracing::warn!(
                "Failed to prune stale code execution artifacts before sandbox run: {}",
                error
            );
        }
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code_raw = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;
        if let Some(reason) = Self::detect_risky_code_execute_install_request(code_raw) {
            if Self::code_execute_high_risk_install_is_explicitly_approved(auth_context) {
                tracing::info!(
                    "Allowing high-risk code_execute installer after explicit approval turn: {}",
                    reason
                );
            } else {
                anyhow::bail!(
                    "High-risk installer path detected inside `code_execute`: {}. Ordinary sandbox-local pip/npm installs from standard registries are auto-allowed. I did not run this installer automatically. Reply with approval in this chat if you want me to allow this exact installer path, or rewrite it to use standard registry packages inside the sandbox.",
                    reason
                );
            }
        }

        // Strip Jupyter magic commands (!pip, !apt, !conda, %pip, %conda, etc.)
        // LLMs often generate these in regular Python scripts - our auto-dependency
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
        let file_path = format!("{}/code.{}", CODE_EXECUTE_SANDBOX_DIR, ext);
        // Java needs the file named Main.java
        let file_path = if language == "java" {
            format!("{}/Main.java", CODE_EXECUTE_SANDBOX_DIR)
        } else {
            file_path
        };

        // Build file injection commands for uploaded files.
        // Upload IDs are resolved through storage, validated against the managed uploads root,
        // then base64-encoded and decoded into /data/ inside the container.
        let sandbox_files = self.collect_code_execute_files(arguments).await?;
        let mut file_inject_cmds = String::new();
        if !sandbox_files.is_empty() {
            file_inject_cmds.push_str("mkdir -p /data && ");
            for upload in sandbox_files {
                let data_b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &upload.bytes,
                );
                file_inject_cmds.push_str(&format!(
                    "echo '{}' | base64 -d > /data/{} && ",
                    data_b64, upload.filename
                ));
                tracing::info!(
                    "Injecting uploaded file into container: {} ({} bytes)",
                    upload.filename,
                    upload.bytes.len()
                );
            }
        }

        // Auto-detect dependencies for Python/Node and install them inside the sandbox only.
        let python_packages = if matches!(language.as_str(), "python" | "python3" | "py") {
            Self::detect_python_dep_packages(code)
        } else {
            Vec::new()
        };
        let node_packages = if matches!(
            language.as_str(),
            "javascript" | "js" | "node" | "typescript" | "ts"
        ) {
            Self::detect_node_dep_packages(code)
        } else {
            Vec::new()
        };
        let auto_install_cmd = if !python_packages.is_empty() {
            tracing::info!("Auto-detected Python deps: {:?}", python_packages);
            format!(
                "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 -m pip install --no-cache-dir -q {} && ",
                python_packages.join(" ")
            )
        } else if !node_packages.is_empty() {
            tracing::info!("Auto-detected Node.js deps: {:?}", node_packages);
            format!(
                "npm install --no-fund --no-audit -q {} 2>/dev/null && ",
                node_packages.join(" ")
            )
        } else {
            String::new()
        };

        let run = run_cmd
            .replace("{file}", &file_path)
            .replace("{sandbox_dir}", CODE_EXECUTE_SANDBOX_DIR);
        let workspace_bootstrap = format!(
            "mkdir -p '{sandbox}' '{home}' '{tmp}' '{cache}' '{pip_cache}' '{cache}/npm' && export HOME='{home}' TMPDIR='{tmp}' TMP='{tmp}' TEMP='{tmp}' XDG_CACHE_HOME='{cache}' PIP_CACHE_DIR='{pip_cache}' npm_config_cache='{cache}/npm' NPM_CONFIG_CACHE='{cache}/npm' && cd '{sandbox}' && ",
            sandbox = CODE_EXECUTE_SANDBOX_DIR,
            home = CODE_EXECUTE_HOME_DIR,
            tmp = CODE_EXECUTE_TMP_DIR,
            cache = CODE_EXECUTE_CACHE_DIR,
            pip_cache = CODE_EXECUTE_PIP_CACHE_DIR,
        );
        let main_cmd = if let Some(build) = build_cmd {
            let build = build
                .replace("{file}", &file_path)
                .replace("{sandbox_dir}", CODE_EXECUTE_SANDBOX_DIR);
            format!(
                "{}{}{}echo '{}' | base64 -d > {} && {} && {}",
                workspace_bootstrap,
                file_inject_cmds,
                auto_install_cmd,
                code_b64,
                file_path,
                build,
                run
            )
        } else {
            format!(
                "{}{}{}echo '{}' | base64 -d > {} && {}",
                workspace_bootstrap, file_inject_cmds, auto_install_cmd, code_b64, file_path, run
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
            r#"{}; __AGENTARK_EXIT=$?; echo; echo '__AGENTARK_OUTPUT_FILES__';{} find {sandbox_dir} -maxdepth 3 -type f ! -name 'code.*' ! -name 'a.out' ! -name 'Main.*' ! -name '*.class' ! -name 'out.jar' ! -name '*.ipynb' -newer {} 2>/dev/null | head -20 | while IFS= read -r __f; do __sz=$(stat -c%s "$__f" 2>/dev/null || echo 999999999); if [ "$__sz" -lt 5242880 ]; then echo "FILE:$(basename "$__f"):$(base64 "$__f" | tr -d '\n')"; fi; done; exit $__AGENTARK_EXIT"#,
            main_cmd,
            notebook_extra,
            file_path,
            sandbox_dir = CODE_EXECUTE_SANDBOX_DIR
        );

        // Notebooks get 10 min (install deps + execute all cells + ML training).
        // Compiled languages get 120s (build + run), interpreted get 60s. If
        // runtime auto-installs dependencies, raise the default so the control
        // plane does not time out ordinary sandbox package bootstrap.
        let base_timeout = if is_notebook {
            600
        } else if build_cmd.is_some() {
            120
        } else {
            60
        };
        let dependency_bootstrap = !python_packages.is_empty() || !node_packages.is_empty();
        let default_timeout = if dependency_bootstrap {
            base_timeout.max(180)
        } else {
            base_timeout
        };
        let timeout_limit = if is_notebook { 900 } else { 600 };
        let timeout = arguments
            .get("timeout_secs")
            .and_then(|value| value.as_u64())
            .unwrap_or(default_timeout)
            .clamp(1, timeout_limit);

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
        let network_access = Self::code_execute_effective_network_access(arguments);
        let isolation = if network_access {
            ContainerIsolation::StandardWithNetwork
        } else {
            ContainerIsolation::Standard
        };

        let raw_result = self
            .run_isolated_container(
                "code_execute",
                image,
                vec!["sh".to_string(), "-c".to_string(), shell_cmd],
                env_vec,
                timeout,
                isolation,
            )
            .await?;

        // Parse result and extract output files from stdout
        let parsed: serde_json::Value = serde_json::from_str(&raw_result)?;
        let output = parsed["output"].as_str().unwrap_or("");

        let exec_id = uuid::Uuid::new_v4().to_string();
        let output_dir = self.data_dir().join("outputs").join(&exec_id);
        tokio::fs::create_dir_all(&output_dir)
            .await
            .with_context(|| {
                format!(
                    "Failed to create output directory '{}'",
                    output_dir.display()
                )
            })?;

        let install_summary =
            Self::code_execute_dependency_summary(&python_packages, &node_packages);
        let install_metadata =
            Self::build_code_execute_dependency_metadata(&python_packages, &node_packages);
        let (user_output, saved_files) = if let Some(marker_pos) =
            output.find("__AGENTARK_OUTPUT_FILES__")
        {
            let mut user_output = output[..marker_pos].trim_end().to_string();
            if let Some(summary) = install_summary.as_deref() {
                if user_output.is_empty() {
                    user_output = summary.to_string();
                } else if !user_output.contains(summary) {
                    user_output = format!("{}\n{}", summary, user_output);
                }
            }
            let files_section = &output[marker_pos..];

            let mut saved = Vec::new();

            // Save the code file first so user can download it
            {
                let code_filename = format!("code.{}", ext);
                let code_path = output_dir.join(&code_filename);
                tokio::fs::write(&code_path, code).await.with_context(|| {
                    format!("Failed to save code artifact '{}'", code_path.display())
                })?;
                saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
                tracing::debug!("Saved code file: {}", code_path.display());
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
            // No file marker found - still save the code file
            let mut saved = Vec::new();
            let code_filename = format!("code.{}", ext);
            let code_path = output_dir.join(&code_filename);
            tokio::fs::write(&code_path, code).await.with_context(|| {
                format!("Failed to save code artifact '{}'", code_path.display())
            })?;
            saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
            let mut user_output = output.to_string();
            if let Some(summary) = install_summary.as_deref() {
                if user_output.trim().is_empty() {
                    user_output = summary.to_string();
                } else if !user_output.contains(summary) {
                    user_output = format!("{}\n{}", summary, user_output);
                }
            }
            (user_output, saved)
        };

        // Build final result with file paths
        let mut result = serde_json::json!({
            "output": user_output,
            "error": parsed.get("error").cloned().unwrap_or(serde_json::Value::Null),
            "exit_code": parsed.get("exit_code").cloned().unwrap_or(serde_json::json!(-1)),
            "dependency_installs": install_metadata,
            "agentark_execution": Self::build_code_execute_execution_metadata(
                arguments,
                parsed
                    .get("exit_code")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(-1)
                    == 0,
                saved_files.len(),
            ),
        });

        let exit_code = parsed
            .get("exit_code")
            .and_then(|value| value.as_i64())
            .unwrap_or(-1);
        if exit_code != 0 {
            let combined_failure_text = format!(
                "{}\n{}",
                result
                    .get("output")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                result
                    .get("error")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            if let Some(binary) = Self::detect_missing_binary_from_output(&combined_failure_text) {
                result["missing_capabilities"] = serde_json::json!([{
                    "kind": "binary",
                    "name": binary,
                    "approval_required": true,
                    "route": "host_install_approval",
                    "reason": "The sandbox execution failed because this binary is not available. AgentArk will not install OS/host packages without explicit approval."
                }]);
            }
        }

        if !saved_files.is_empty() {
            result["files"] = serde_json::json!(saved_files);
        }

        Ok(serde_json::to_string(&result)?)
    }

    /// Fallback: execute code natively in an isolated temp directory (no Docker)
    async fn execute_code_native(&self, arguments: &serde_json::Value) -> Result<String> {
        if let Err(error) = self.prune_stale_code_execute_artifacts().await {
            tracing::warn!(
                "Failed to prune stale code execution artifacts before native sandbox run: {}",
                error
            );
        }
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
                ));
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
            "agentark_execution": Self::build_code_execute_execution_metadata(
                arguments,
                exit_code == 0,
                0,
            ),
        });

        Ok(serde_json::to_string(&result)?)
    }

    /// List available actions
    pub async fn list_actions(&self) -> Result<Vec<ActionDef>> {
        let actions = self.actions.read().await;
        Ok(actions.values().map(|s| s.info.clone()).collect())
    }

    pub async fn action_definition(&self, action_name: &str) -> Option<ActionDef> {
        let actions = self.actions.read().await;
        actions.get(action_name).map(|loaded| loaded.info.clone())
    }

    pub async fn list_action_scope_hints(&self) -> Result<HashMap<String, ActionScopeHint>> {
        let actions = self.actions.read().await;
        Ok(actions
            .iter()
            .map(|(name, loaded)| {
                (
                    name.clone(),
                    Self::action_scope_hint_for_loaded_action(name, loaded),
                )
            })
            .collect())
    }

    /// List only actions that are currently executable by the agent.
    /// Non-system actions honor the disabled set; integration-backed system actions honor
    /// the integration enable/disable toggle.
    pub async fn list_enabled_actions(&self) -> Result<Vec<ActionDef>> {
        let disabled = self.disabled_actions.read().await;
        let actions = self
            .actions
            .read()
            .await
            .values()
            .map(|loaded| loaded.info.clone())
            .collect::<Vec<_>>();
        drop(disabled);
        let mut enabled = Vec::new();
        for action in actions {
            if action.source == ActionSource::System {
                if self.is_action_integration_ready(&action).await {
                    enabled.push(action);
                }
                continue;
            }

            if self
                .disabled_actions
                .read()
                .await
                .contains(action.name.as_str())
            {
                continue;
            }

            if let Some(review) = self.refresh_action_review_state(&action.name).await? {
                if !review.visible_in_catalog {
                    continue;
                }
            } else {
                continue;
            }
            enabled.push(action);
        }
        Ok(enabled)
    }

    /// Returns true if an action is enabled (not in the disabled set).
    pub async fn is_action_enabled(&self, name: &str) -> bool {
        let action = {
            let actions = self.actions.read().await;
            actions.get(name).map(|loaded| loaded.info.clone())
        };
        let Some(action) = action else {
            return false;
        };
        if action.source == ActionSource::System {
            return self.is_action_integration_ready(&action).await;
        }

        let disabled = self.disabled_actions.read().await;
        if disabled.contains(name) {
            return false;
        }
        match self.refresh_action_review_state(name).await {
            Ok(Some(review)) => review.allow_execute,
            Ok(None) => false,
            Err(_) => false,
        }
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

        if enabled {
            match self.refresh_action_review_state(name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        anyhow::bail!(
                            "{}",
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to enable.", name)
                            })
                        );
                    }
                }
                None => anyhow::bail!(
                    "Action '{}' has no persisted security review and cannot be enabled.",
                    name
                ),
            }
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

    /// Update action content - for bundled actions, creates a custom copy first
    pub async fn update_action_content(&self, name: &str, content: &str) -> Result<bool> {
        let (source, file_path) = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(name) else {
                return Ok(false);
            };
            if action.info.source == ActionSource::System {
                return Ok(false);
            }
            (action.info.source.clone(), action.info.file_path.clone())
        };

        let (action_dir, action_file, action_source) = if source == ActionSource::Bundled {
            let custom_action_dir = self.actions_dir.join(name);
            tokio::fs::create_dir_all(&custom_action_dir).await?;
            (
                custom_action_dir.clone(),
                Self::preferred_skill_markdown_path(&custom_action_dir),
                ActionSource::Custom,
            )
        } else if let Some(file_path) = file_path {
            let action_file = PathBuf::from(file_path);
            let action_dir = action_file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.actions_dir.join(name));
            (action_dir, action_file, ActionSource::Custom)
        } else {
            return Ok(false);
        };

        tokio::fs::write(&action_file, content).await?;
        if let Some(ref guard) = self.action_guard {
            if let Err(error) = guard.resign_action(&action_dir, name).await {
                tracing::warn!("Failed to re-sign action '{}': {}", name, error);
            }
        }

        let (new_info, new_content, frontmatter) = self
            .parse_action_md(&action_file, action_source.clone())
            .await?;
        let review = self
            .review_markdown_action(&action_dir, &new_info, &new_content, &frontmatter)
            .await?;

        {
            let mut actions = self.actions.write().await;
            if let Some(action) = actions.get_mut(name) {
                action.info = new_info;
                action.info.source = action_source;
                action.info.file_path = Some(action_file.to_string_lossy().to_string());
                action.workflow_content = Some(new_content);
            }
        }

        self.upsert_action_review(review.clone()).await?;
        if review.allow_execute {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.remove(name) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        } else {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.insert(name.to_string()) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        }

        tracing::info!(
            "Updated action '{}' and refreshed security review state",
            name
        );
        Ok(true)
    }

    pub async fn apply_semantically_reviewed_skill_evolution_candidate(
        &self,
        action: &str,
        name: &str,
        content: &str,
        evidence_markdown: &str,
        semantic_review: &crate::security::skill_review::SemanticSkillReview,
    ) -> Result<SkillEvolutionApplyResult> {
        let action = action.trim().to_ascii_lowercase();
        let name = name.trim();
        let content = content.trim();
        if name.is_empty() {
            anyhow::bail!("skill evolution candidate is missing a skill name");
        }
        if content.is_empty() {
            anyhow::bail!("skill evolution candidate is missing skill content");
        }
        match action.as_str() {
            "create_skill" | "improve_skill" | "optimize_description" => {}
            other => anyhow::bail!("unsupported skill evolution action '{}'", other),
        }

        let existing = self.get_action_content(name).await?;
        if action == "create_skill" && existing.is_none() {
            let review = self
                .install_semantically_reviewed_action(name, content, semantic_review, false)
                .await?;
            if !review.allow_load {
                anyhow::bail!("skill creation was blocked by semantic security policy");
            }
            let history_dir = self.actions_dir.join(name).join("history");
            tokio::fs::create_dir_all(&history_dir).await?;
            tokio::fs::write(history_dir.join("v0_evidence.md"), evidence_markdown).await?;
            return Ok(SkillEvolutionApplyResult {
                skill_name: name.to_string(),
                approved_ref: format!("skill:{}:v1", name),
                history_version: 1,
            });
        }

        let Some((info, before_content)) = existing else {
            anyhow::bail!("skill '{}' does not exist in the current runtime", name);
        };
        if info.source == ActionSource::System {
            anyhow::bail!("system actions cannot be modified by skill evolution");
        }

        let history_dir = if info.source == ActionSource::Bundled {
            self.actions_dir.join(name).join("history")
        } else {
            info.file_path
                .as_deref()
                .and_then(|value| Path::new(value).parent().map(|path| path.join("history")))
                .unwrap_or_else(|| self.actions_dir.join(name).join("history"))
        };
        tokio::fs::create_dir_all(&history_dir).await?;
        let history_version =
            crate::core::self_evolve::skill_evolution::next_skill_history_version(&history_dir)?;
        tokio::fs::write(
            history_dir.join(format!("v{}.md", history_version)),
            before_content,
        )
        .await?;
        tokio::fs::write(
            history_dir.join(format!("v{}_evidence.md", history_version)),
            evidence_markdown,
        )
        .await?;
        let review = self
            .update_semantically_reviewed_action(name, content, semantic_review, false)
            .await?;
        if review.is_none() {
            anyhow::bail!(
                "failed to update skill '{}' after snapshotting history",
                name
            );
        }

        Ok(SkillEvolutionApplyResult {
            skill_name: name.to_string(),
            approved_ref: format!("skill:{}:v{}", name, history_version + 1),
            history_version: history_version + 1,
        })
    }

    /// Create a new custom action with security verification
    /// Returns the security verdict so the caller can present it to the user.
    /// `force` can keep non-blocking warnings visible, but semantic/security
    /// blocks are never overridden.
    pub async fn create_action(
        &self,
        name: &str,
        content: &str,
        _force: bool,
    ) -> Result<Option<crate::security::action_guard::ActionSecurityVerdict>> {
        let guard = self.action_guard.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Action security is unavailable, so importing new skills is disabled.")
        })?;
        let action_dir = self.actions_dir.join(name);
        tokio::fs::create_dir_all(&action_dir).await?;

        let action_file = Self::preferred_skill_markdown_path(&action_dir);
        tokio::fs::write(&action_file, content).await?;

        // Sign the new action manifest
        if let Err(e) = guard.resign_action(&action_dir, name).await {
            tracing::warn!("Failed to sign new action '{}': {}", name, e);
        }

        let (info, workflow_content, frontmatter) = self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to parse action: {}", error))?;
        let verdict = guard
            .evaluate_action(&action_dir, name, &workflow_content, &frontmatter)
            .await?;
        let required_env = Self::extract_required_envs_from_frontmatter(&frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(&action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[&workflow_content, &frontmatter]));
        let review = Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: Self::action_source_label(&info.source),
            fingerprint,
            verdict: &verdict,
            required_env,
            missing_env,
            requires_auth: false,
            auth_configured: true,
            notes: Vec::new(),
        });
        let blocked = !verdict.allow_load;

        if blocked {
            tracing::warn!(
                "New action '{}' BLOCKED by security guard: {:?}",
                name,
                verdict.warnings
            );
            let _ = tokio::fs::remove_dir_all(&action_dir).await;
            self.remove_action_review(name).await?;
            return Ok(Some(verdict));
        }

        for warning in &verdict.warnings {
            tracing::warn!("Action '{}': {}", name, warning);
        }
        self.register_workflow_action(info, workflow_content).await;
        self.upsert_action_review(review).await?;
        tracing::info!(
            "Created and registered action '{}' at {:?}",
            name,
            action_file
        );
        Ok(Some(verdict))
    }

    /// Create a custom action after the import path has completed semantic
    /// capability review with the configured model. This signs and persists the
    /// skill, then stores the deterministic policy verdict without reclassifying
    /// the content through wording-based checks.
    pub async fn install_semantically_reviewed_action(
        &self,
        name: &str,
        content: &str,
        semantic_review: &crate::security::skill_review::SemanticSkillReview,
        _force: bool,
    ) -> Result<ActionReviewSnapshot> {
        if semantic_review.policy.blocked {
            anyhow::bail!("Skill '{}' blocked by semantic security policy", name);
        }

        let action_dir = self.actions_dir.join(name);
        tokio::fs::create_dir_all(&action_dir).await?;
        let action_file = Self::preferred_skill_markdown_path(&action_dir);
        tokio::fs::write(&action_file, content).await?;

        let (info, workflow_content, frontmatter) = self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to parse action: {}", error))?;
        let Some(ref guard) = self.action_guard else {
            anyhow::bail!("Action security is unavailable, so importing new skills is disabled.");
        };
        guard
            .resign_action(&action_dir, name)
            .await
            .with_context(|| format!("Failed to sign semantically reviewed skill '{}'", name))?;
        let integrity_ok = true;

        let required_env = Self::extract_required_envs_from_frontmatter(&frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(&action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[&workflow_content, &frontmatter]));

        let mut notes = Vec::new();
        notes.push(format!(
            "Semantic capability review used configured model '{}'.",
            semantic_review.model
        ));
        if !semantic_review.summary.trim().is_empty() {
            notes.push(format!(
                "Semantic summary: {}",
                semantic_review.summary.trim()
            ));
        }
        let capabilities = semantic_review
            .capabilities
            .iter()
            .map(|capability| {
                if let Some(target) = capability.target.as_deref() {
                    format!("{}:{}", capability.normalized_kind(), target)
                } else {
                    capability.normalized_kind()
                }
            })
            .collect::<Vec<_>>();
        if !capabilities.is_empty() {
            notes.push(format!("Capabilities: {}.", capabilities.join(", ")));
        }
        for rule in &semantic_review.policy.matched_rules {
            notes.push(format!("Policy rule '{}': {}", rule.id, rule.message));
        }

        let warnings = semantic_review.policy.warnings.clone();
        let allow_load = !semantic_review.policy.blocked;
        let status = if !missing_env.is_empty() {
            ActionReviewStatus::NeedsSecrets
        } else if !warnings.is_empty()
            || semantic_review.policy.risk_band == "review"
            || semantic_review.policy.risk_band == "risky"
        {
            ActionReviewStatus::Warning
        } else {
            ActionReviewStatus::Ready
        };

        let allow_execute = matches!(
            status,
            ActionReviewStatus::Ready | ActionReviewStatus::Warning
        );
        let blocked_reason = if semantic_review.policy.blocked {
            semantic_review
                .policy
                .warnings
                .first()
                .cloned()
                .or_else(|| Some("Blocked by semantic skill security policy.".to_string()))
        } else if !missing_env.is_empty() {
            Some(format!(
                "Required secrets missing: {}",
                missing_env.join(", ")
            ))
        } else {
            None
        };

        let review = ActionReviewSnapshot {
            action_name: info.name.clone(),
            source_kind: Self::action_source_label(&info.source).to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status,
            ready: allow_execute,
            allow_load,
            allow_execute,
            visible_in_catalog: allow_execute,
            integrity_ok,
            threat_level: format!("{:?}", semantic_review.policy.threat_level),
            total_severity: semantic_review.policy.total_severity,
            total_findings: semantic_review.policy.findings.len(),
            risk_score_10: semantic_review.policy.risk_score_10,
            risk_band: semantic_review.policy.risk_band.clone(),
            warnings,
            findings: semantic_review.policy.findings.clone(),
            required_env,
            missing_env,
            permissions_needed: capabilities,
            requires_auth: false,
            auth_configured: true,
            notes,
            blocked_reason,
        };

        self.register_workflow_action(info, workflow_content).await;
        self.upsert_action_review(review.clone()).await?;
        self.record_action_review_event(&review).await;
        if review.allow_execute {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.remove(name) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        } else {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.insert(name.to_string()) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        }
        Ok(review)
    }

    pub async fn update_semantically_reviewed_action(
        &self,
        name: &str,
        content: &str,
        semantic_review: &crate::security::skill_review::SemanticSkillReview,
        force: bool,
    ) -> Result<Option<ActionReviewSnapshot>> {
        let editable = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(name) else {
                return Ok(None);
            };
            action.info.source != ActionSource::System
        };
        if !editable {
            return Ok(None);
        }
        self.install_semantically_reviewed_action(name, content, semantic_review, force)
            .await
            .map(Some)
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

    /// Delete/disable an action.
    /// - Custom actions: deleted from disk and runtime.
    /// - Bundled actions: deleted from runtime-owned bundled directories for this install.
    /// - System actions: cannot be deleted/disabled.
    pub async fn delete_action(&self, name: &str) -> Result<bool> {
        let (source, file_path) = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => (action.info.source.clone(), action.info.file_path.clone()),
                None => return Ok(false),
            }
        };
        tracing::info!(
            action = name,
            source = ?source,
            has_file_path = file_path.is_some(),
            "Runtime delete_action resolved action"
        );

        match source {
            ActionSource::System => {
                tracing::info!(action = name, "Runtime delete_action refused system action");
                Ok(false)
            }
            ActionSource::Bundled => {
                tracing::info!(
                    action = name,
                    "Runtime delete_action deleting bundled action"
                );
                self.delete_runtime_owned_bundled_skill_dir(name).await?;
                {
                    let mut removed = self.removed_bundled_actions.write().await;
                    removed.insert(name.to_string());
                }
                {
                    let mut disabled = self.disabled_actions.write().await;
                    disabled.remove(name);
                }
                self.save_removed_bundled_actions().await?;
                self.save_disabled_actions().await?;
                self.clear_action_secret_bindings(name).await?;
                self.remove_action_review(name).await?;
                let mut actions = self.actions.write().await;
                actions.remove(name);
                tracing::info!("Deleted bundled action '{}' for this install", name);
                Ok(true)
            }
            ActionSource::Custom => {
                tracing::info!(
                    action = name,
                    "Runtime delete_action deleting custom action"
                );
                if let Some(fp) = file_path {
                    let action_path = std::path::Path::new(&fp);
                    if let Some(action_dir) = action_path.parent() {
                        let dir_path = action_dir.to_path_buf();
                        if dir_path.exists() {
                            tracing::info!(
                                action = name,
                                path = %dir_path.display(),
                                "Runtime delete_action removing custom action directory"
                            );
                            tokio::fs::remove_dir_all(&dir_path).await?;
                        }
                    }
                }
                {
                    let mut disabled = self.disabled_actions.write().await;
                    disabled.remove(name);
                }
                self.save_disabled_actions().await?;
                self.clear_action_secret_bindings(name).await?;
                self.remove_action_review(name).await?;
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
        let search_config = build_search_config(&self.config_dir, self.storage.as_ref()).await;

        for query in &search_queries {
            tracing::debug!("Searching: {}", query);
            let args = crate::actions::search::SearchArgs {
                query: query.clone(),
                num_results: 5,
                backend: None,
                time_scope: None,
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
            if user_query.is_empty() {
                "none"
            } else {
                user_query
            }
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

    fn collect_sensitive_required_fields_from_schema(schema: &serde_json::Value) -> Vec<String> {
        let required = Self::collect_required_fields_from_schema(schema);
        let properties = schema.get("properties").and_then(|value| value.as_object());
        required
            .into_iter()
            .filter(|key| {
                properties
                    .and_then(|items| items.get(key))
                    .and_then(|value| value.as_object())
                    .is_some_and(|property| {
                        property.get("sensitive").and_then(|value| value.as_bool()) == Some(true)
                            || property.get("writeOnly").and_then(|value| value.as_bool())
                                == Some(true)
                            || property
                                .get("format")
                                .and_then(|value| value.as_str())
                                .is_some_and(|value| value.eq_ignore_ascii_case("password"))
                    })
            })
            .collect()
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
                "sensitive_missing": payload.sensitive_missing,
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

    fn slugify_name(value: &str) -> String {
        let mut slug = String::new();
        let mut last_was_separator = false;
        for ch in value.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                last_was_separator = false;
            } else if !last_was_separator {
                slug.push('-');
                last_was_separator = true;
            }
        }
        while slug.ends_with('-') {
            slug.pop();
        }
        slug
    }

    fn parse_required_fields_from_frontmatter(frontmatter: &str) -> Vec<String> {
        let mut required = Vec::new();
        let lines: Vec<&str> = frontmatter.lines().collect();
        let mut i = 0usize;

        while i < lines.len() {
            let raw = lines[i];
            let line = raw.trim();
            let is_required_key = line
                .split_once(':')
                .map(|(key, _)| Self::slugify_name(key).replace('-', "_"))
                .is_some_and(|key| {
                    matches!(
                        key.as_str(),
                        "required" | "required_inputs" | "requiredinputs" | "required_fields"
                    )
                });
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
                let heading = Self::slugify_name(line.trim_start_matches('#').trim());
                in_required_section = matches!(
                    heading.as_str(),
                    "required-inputs"
                        | "inputs-required"
                        | "required-fields"
                        | "required"
                        | "input-contract"
                );
                continue;
            }

            if line
                .split_once(':')
                .map(|(key, _)| Self::slugify_name(key))
                .is_some_and(|key| matches!(key.as_str(), "required-inputs" | "inputs-required"))
            {
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
    /// Looks for SKILL.md files in subdirectories.
    /// These are registered as workflow actions for LLM-driven execution
    pub async fn load_markdown_actions(&self, dir: &Path, source: ActionSource) -> Result<()> {
        let dir_exists = tokio::fs::metadata(dir)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if !dir_exists {
            return Ok(());
        }

        // Read directory entries
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Could not read skills directory {:?}: {}", dir, e);
                return Ok(());
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let is_dir = entry
                .file_type()
                .await
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            let path = entry.path();
            let md_file = path.join("SKILL.md");
            let md_exists = tokio::fs::metadata(&md_file)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            if !md_exists {
                continue;
            }

            match self.parse_action_md(&md_file, source.clone()).await {
                Ok((info, workflow_content, frontmatter)) => {
                    if source == ActionSource::Bundled {
                        let removed = self.removed_bundled_actions.read().await;
                        if removed.contains(&info.name) {
                            tracing::info!(
                                "Skipped deleted bundled action '{}' from {:?}",
                                info.name,
                                md_file
                            );
                            continue;
                        }
                        let disabled = self.disabled_actions.read().await;
                        if disabled.contains(&info.name) {
                            tracing::info!(
                                "Loaded bundled action '{}' as disabled from {:?}",
                                info.name,
                                md_file
                            );
                        }
                    }

                    let review = if source == ActionSource::Custom {
                        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(&path)
                            .unwrap_or_else(|_| {
                                Self::fingerprint_text(&[&workflow_content, &frontmatter])
                            });
                        match self.get_action_review(&info.name).await {
                            Some(mut stored) if stored.fingerprint == fingerprint => {
                                if Self::has_semantic_skill_review_marker(&stored) {
                                    stored.source_kind =
                                        Self::action_source_label(&info.source).to_string();
                                    stored
                                } else {
                                    Self::build_blocked_review(
                                        &info.name,
                                        Self::action_source_label(&info.source),
                                        fingerprint,
                                        "Custom skill review predates the semantic security layer. Re-import or update the skill before it can run.",
                                    )
                                }
                            }
                            Some(_) => Self::build_blocked_review(
                                &info.name,
                                Self::action_source_label(&info.source),
                                fingerprint,
                                "Skill files changed on disk outside the reviewed API path; re-import or update the skill to run semantic review again.",
                            ),
                            None => Self::build_blocked_review(
                                &info.name,
                                Self::action_source_label(&info.source),
                                fingerprint,
                                "Custom skill has no semantic security review. Re-import or update the skill before it can run.",
                            ),
                        }
                    } else {
                        self.review_markdown_action(&path, &info, &workflow_content, &frontmatter)
                            .await?
                    };
                    for warning in &review.warnings {
                        tracing::warn!("Action '{}': {}", info.name, warning);
                    }
                    if !review.allow_execute {
                        tracing::warn!(
                            "Loaded action '{}' in blocked/unready state: {:?}",
                            info.name,
                            review.blocked_reason
                        );
                    }

                    tracing::info!("Loaded workflow action '{}' from {:?}", info.name, md_file);
                    self.register_workflow_action(info.clone(), workflow_content.clone())
                        .await;
                    self.upsert_action_review(review).await?;
                    continue;

                    /* Legacy duplicate security-evaluation path removed.
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
                                    "Security check failed for '{}': {} - loading anyway",
                                    info.name,
                                    e
                                );
                            }
                        }
                    }

                    tracing::info!("Loaded workflow action '{}' from {:?}", info.name, md_file);
                    self.register_workflow_action(info, workflow_content).await;
                    */
                }
                Err(e) => {
                    tracing::warn!("Failed to load action from {:?}: {}", md_file, e);
                }
            }
        }

        Ok(())
    }

    /// Parse a SKILL.md file to extract action information and full content.
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

        if let Some((frontmatter, _rest)) = Self::split_frontmatter_block(&content) {
            frontmatter_text = frontmatter.to_string();
            if let Some(root) = Self::parse_frontmatter_yaml(frontmatter)
                .and_then(|value| value.as_mapping().cloned())
            {
                if let Some(value) = root
                    .get(serde_yaml::Value::String("name".to_string()))
                    .and_then(|value| value.as_str())
                {
                    name = value.trim().to_string();
                }
                if let Some(value) = root
                    .get(serde_yaml::Value::String("description".to_string()))
                    .and_then(|value| value.as_str())
                {
                    description = value.trim().to_string();
                }
                if let Some(value) = root
                    .get(serde_yaml::Value::String("version".to_string()))
                    .and_then(|value| value.as_str())
                {
                    version = value.trim().to_string();
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
            authorization: Default::default(),
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

        let engine = self.sandbox.engine();

        // Create a basic store without WASI for simple modules, but enforce configured limits.
        let mut store = self.sandbox.new_store();

        // Compile the module
        let module = Module::new(engine, wasm_bytes)?;

        // Create a linker and instantiate
        let linker = Linker::new(engine);
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

pub(crate) fn load_persisted_search_config(
    config_dir: &Path,
    data_dir: Option<&Path>,
) -> crate::actions::SearchConfig {
    if let Ok(manager) = SecureConfigManager::new_with_data_dir(config_dir, data_dir) {
        if manager.uses_storage_backend() {
            match manager.load_encrypted_json::<crate::actions::SearchConfig>(
                crate::core::config::SETTINGS_SEARCH_KEY,
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
            return manager.save_encrypted_json(crate::core::config::SETTINGS_SEARCH_KEY, config);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::integration_enabled_key;

    #[test]
    fn parse_schedule_task_completion_accepts_structured_marker() {
        let structured = format!(
            "{}{}",
            TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "schedule_task",
                "status": "completed",
                "detail": "Task scheduled"
            })
        );
        let structured = parse_schedule_task_completion(&structured)
            .expect("structured schedule marker should parse");
        assert_eq!(structured.tool, "schedule_task");
        assert_eq!(structured.status, "completed");
    }

    #[test]
    fn parse_watch_completion_accepts_structured_marker() {
        let structured = format!(
            "{}{}",
            TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "watch",
                "status": "completed",
                "detail": "Watch created"
            })
        );
        let structured =
            parse_watch_completion(&structured).expect("structured watch marker should parse");
        assert_eq!(structured.tool, "watch");
        assert_eq!(structured.status, "completed");
    }

    #[test]
    fn parse_delegate_completion_accepts_structured_marker() {
        let structured = format!(
            "{}{}",
            TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "delegate",
                "status": "completed",
                "detail": "Delegation prepared"
            })
        );
        let structured = parse_delegate_completion(&structured)
            .expect("structured delegate marker should parse");
        assert_eq!(structured.tool, "delegate");
        assert_eq!(structured.status, "completed");
    }

    #[test]
    fn parse_tool_completion_accepts_marker_line_before_human_text() {
        let output = format!(
            "{}{}\nHuman readable status follows.",
            TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "watch",
                "status": "completed",
                "detail": "Polling configured"
            })
        );
        let structured = parse_watch_completion(&output)
            .expect("structured marker line should parse before human text");

        assert_eq!(structured.tool, "watch");
        assert_eq!(structured.status, "completed");
    }

    #[test]
    fn parse_tool_completion_accepts_raw_json_envelope() {
        let structured = parse_schedule_task_completion(
            &serde_json::json!({
                "tool": "schedule_task",
                "status": "completed",
                "detail": "Task scheduled"
            })
            .to_string(),
        )
        .expect("raw JSON completion should parse");

        assert_eq!(structured.tool, "schedule_task");
        assert_eq!(structured.status, "completed");
    }

    #[test]
    fn parse_workflow_inputs_accepts_structured_json_without_marker() {
        let payload = serde_json::json!({
            "action": "lookup_customer",
            "missing": ["customer_id"],
            "required": ["customer_id"],
            "provided": [],
            "query": "lookup customer"
        })
        .to_string();

        let parsed = parse_workflow_missing_inputs_marker(&payload)
            .expect("raw JSON missing-input payload should parse");

        assert_eq!(parsed.action, "lookup_customer");
        assert_eq!(parsed.missing, vec!["customer_id".to_string()]);
    }

    #[test]
    fn required_input_parsing_accepts_canonicalized_metadata_keys_and_headings() {
        let frontmatter = "Required Fields:\n  - customer_id\n  - account_id";
        assert_eq!(
            ActionRuntime::parse_required_fields_from_frontmatter(frontmatter),
            vec!["customer_id".to_string(), "account_id".to_string()]
        );

        let workflow = "## Inputs Required\n- `customer_id`: stable customer id\n- account_id";
        assert_eq!(
            ActionRuntime::parse_required_fields_from_workflow(workflow),
            vec!["customer_id".to_string(), "account_id".to_string()]
        );
    }

    #[test]
    fn docker_host_socket_transport_detection_handles_unix_and_tcp_hosts() {
        assert!(ActionRuntime::docker_host_uses_socket_transport(
            "unix:///var/run/docker.sock"
        ));
        assert!(ActionRuntime::docker_host_uses_socket_transport(
            "npipe:////./pipe/docker_engine"
        ));
        assert!(!ActionRuntime::docker_host_uses_socket_transport(
            "tcp://127.0.0.1:2375"
        ));
        assert!(!ActionRuntime::docker_host_uses_socket_transport(
            "http://docker.internal:2375"
        ));
    }

    #[test]
    fn control_plane_without_local_docker_skips_local_docker_management() {
        assert!(!ActionRuntime::should_manage_local_sandbox_containers_for(
            Some("control"),
            false,
        ));
        assert!(!ActionRuntime::should_manage_local_sandbox_containers_for(
            Some("control-plane"),
            false,
        ));
        assert!(ActionRuntime::should_manage_local_sandbox_containers_for(
            Some("control"),
            true,
        ));
        assert!(ActionRuntime::should_manage_local_sandbox_containers_for(
            Some("executor"),
            false,
        ));
        assert!(ActionRuntime::should_manage_local_sandbox_containers_for(
            None, false,
        ));
    }

    #[test]
    fn code_execute_execution_metadata_marks_bootstrap_setup_only() {
        let metadata = ActionRuntime::build_code_execute_execution_metadata(
            &serde_json::json!({
                "network_access": false,
                "execution_contract": {
                    "phase": "bootstrap"
                }
            }),
            true,
            0,
        );

        assert_eq!(
            metadata.get("phase").and_then(|value| value.as_str()),
            Some("bootstrap")
        );
        assert_eq!(
            metadata.get("setup_only").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            metadata
                .get("ready_for_watch")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn upload_signature_detects_opus_by_bytes_without_extension() {
        let mut bytes = b"OggS".to_vec();
        bytes.extend_from_slice(&[0; 24]);
        bytes.extend_from_slice(b"OpusHead");
        let detected = ActionRuntime::upload_signature("voice", None, &bytes);

        assert_eq!(
            detected.get("input_type").and_then(|value| value.as_str()),
            Some("audio")
        );
        assert_eq!(
            detected.get("extension").and_then(|value| value.as_str()),
            Some("opus")
        );
    }

    #[test]
    fn upload_signature_keeps_unknown_unresolved_instead_of_guessing_text() {
        let detected = ActionRuntime::upload_signature(
            "payload",
            Some("application/octet-stream"),
            b"plain utf8 but no durable type evidence",
        );

        assert_eq!(
            detected.get("input_type").and_then(|value| value.as_str()),
            Some("unknown")
        );
        assert_eq!(
            detected
                .get("needs_deeper_inspection")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(detected.get("mime").is_some_and(|value| value.is_null()));
    }

    #[test]
    fn missing_binary_detector_reads_structured_marker() {
        assert_eq!(
            ActionRuntime::detect_missing_binary_from_output("AGENTARK_MISSING_BINARY: ffmpeg\n"),
            Some("ffmpeg".to_string())
        );
    }

    #[test]
    fn missing_binary_detector_extracts_generic_shell_errors() {
        assert_eq!(
            ActionRuntime::detect_missing_binary_from_output(
                "bash: custom-tool: command not found"
            ),
            Some("custom-tool".to_string())
        );
        assert_eq!(
            ActionRuntime::detect_missing_binary_from_output(
                "FileNotFoundError: [Errno 2] No such file or directory: 'media-helper'"
            ),
            Some("media-helper".to_string())
        );
    }

    #[test]
    fn code_execute_execution_metadata_marks_validated_poller_ready_for_watch() {
        let metadata = ActionRuntime::build_code_execute_execution_metadata(
            &serde_json::json!({
                "network_access": true,
                "execution_contract": {
                    "phase": "validate",
                    "target_validated_when_successful": true,
                    "ready_for_watch_when_successful": true
                }
            }),
            true,
            1,
        );

        assert_eq!(
            metadata.get("phase").and_then(|value| value.as_str()),
            Some("validate")
        );
        assert_eq!(
            metadata
                .get("target_validated")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            metadata
                .get("ready_for_watch")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn target_connectivity_contract_enables_effective_network_access() {
        assert!(ActionRuntime::code_execute_effective_network_access(
            &serde_json::json!({
                "execution_contract": {
                    "phase": "validate",
                    "target_connectivity_required": true
                }
            })
        ));
        assert!(!ActionRuntime::code_execute_effective_network_access(
            &serde_json::json!({
                "execution_contract": {
                    "phase": "bootstrap"
                }
            })
        ));
    }

    #[test]
    fn code_execute_infers_network_access_from_endpoint_values() {
        assert!(ActionRuntime::code_execute_effective_network_access(
            &serde_json::json!({
                "language": "python",
                "code": "print('polling a device')",
                "env": {
                    "TARGET_URL": "customproto://192.168.29.61:554/stream"
                }
            })
        ));
        assert!(ActionRuntime::code_execute_effective_network_access(
            &serde_json::json!({
                "language": "python",
                "code": "open('output.txt', 'w').write('http://example.com')"
            })
        ));
        assert!(!ActionRuntime::code_execute_effective_network_access(
            &serde_json::json!({
                "language": "python",
                "code": "print('local-only calculation')"
            })
        ));
    }

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
    async fn capability_resolve_is_builtin_and_inventory_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let action = action_def_by_name(&runtime, "capability_resolve").await;

        assert_eq!(action.source, ActionSource::System);
        assert!(action.capabilities.iter().any(|cap| cap == "file_read"));
        assert!(action
            .capabilities
            .iter()
            .any(|cap| cap == "capability_inventory"));
    }

    #[tokio::test]
    async fn manage_actions_declares_skill_management_capability() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let action = action_def_by_name(&runtime, "manage_actions").await;

        assert_eq!(action.source, ActionSource::System);
        assert!(action
            .capabilities
            .iter()
            .any(|cap| cap == "skill_management"));
    }

    #[tokio::test]
    async fn delegate_is_builtin_multi_agent_capability() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let action = action_def_by_name(&runtime, "delegate").await;

        assert_eq!(action.source, ActionSource::System);
        assert!(action.capabilities.iter().any(|cap| cap == "multi_agent"));
        assert!(action.capabilities.iter().any(|cap| cap == "swarm"));
        assert_eq!(action.input_schema["required"], serde_json::json!(["task"]));
        let metadata = crate::actions::planner_metadata_for_action(&action);
        assert_eq!(
            metadata.role,
            crate::actions::PlannerActionRole::Orchestration
        );
        assert_eq!(
            metadata.side_effect_level,
            crate::actions::PlannerSideEffectLevel::Write
        );
    }

    #[tokio::test]
    async fn capability_acquire_requires_agent_permission() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let action = action_def_by_name(&runtime, "capability_acquire").await;

        assert_eq!(action.source, ActionSource::System);
        assert!(ActionRuntime::action_required_agent_permission_ids(&action)
            .iter()
            .any(|permission| permission == "capability_acquire"));
    }

    #[tokio::test]
    async fn system_action_review_visibility_does_not_hide_builtin_actions() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        runtime
            .upsert_action_review(ActionReviewSnapshot {
                action_name: "file_write".to_string(),
                status: ActionReviewStatus::Blocked,
                ready: false,
                allow_load: false,
                allow_execute: false,
                visible_in_catalog: false,
                blocked_reason: Some("stale persisted review".to_string()),
                ..ActionReviewSnapshot::default()
            })
            .await
            .unwrap();

        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(enabled.iter().any(|action| action.name == "file_write"));
        assert!(runtime.is_action_enabled("file_write").await);
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

    #[tokio::test]
    async fn list_enabled_actions_hides_disconnected_workspace_tools() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(!enabled.iter().any(|action| action.name == "gmail_scan"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "calendar_create"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_drive_search"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_docs_read"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_sheets_read"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_chat_list_spaces"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_admin_list_users"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_workspace_gws_command"));
    }

    #[tokio::test]
    async fn list_enabled_actions_exposes_granted_gmail_without_calendar() {
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
                            "https://www.googleapis.com/auth/gmail.send"
                        ],
                        "granted_bundles": ["gmail"]
                    })
                    .to_string(),
                ),
            )
            .unwrap();
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
                Some(serde_json::json!(["gmail"]).to_string()),
            )
            .unwrap();

        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
        assert!(enabled.iter().any(|action| action.name == "gmail_reply"));
        assert!(!enabled.iter().any(|action| action.name == "calendar_today"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "calendar_create"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_drive_search"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_docs_read"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_sheets_read"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_chat_list_spaces"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_admin_list_users"));
    }

    #[tokio::test]
    async fn list_enabled_actions_exposes_only_granted_workspace_bundle_tools() {
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
                            "https://www.googleapis.com/auth/drive.metadata.readonly"
                        ],
                        "granted_bundles": ["gmail", "drive"]
                    })
                    .to_string(),
                ),
            )
            .unwrap();
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
                Some(serde_json::json!(["gmail", "drive"]).to_string()),
            )
            .unwrap();

        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
        assert!(enabled
            .iter()
            .any(|action| action.name == "google_drive_search"));
        assert!(!enabled.iter().any(|action| action.name == "calendar_today"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_docs_read"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_sheets_read"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_chat_list_spaces"));
        assert!(!enabled
            .iter()
            .any(|action| action.name == "google_admin_list_users"));
    }

    #[tokio::test]
    async fn list_enabled_actions_hides_unconfigured_external_connector_tools() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(!enabled.iter().any(|action| action.name == "places"));
        assert!(!enabled.iter().any(|action| action.name == "twilio"));
        assert!(!enabled.iter().any(|action| action.name == "github"));
        assert!(enabled.iter().any(|action| action.name == "moltbook"));
        let status = runtime
            .execute_action("moltbook", &serde_json::json!({ "action": "status" }))
            .await
            .unwrap();
        assert!(status.contains("not_configured"));
    }

    #[tokio::test]
    async fn list_enabled_actions_exposes_ready_external_connector_tools() {
        let temp = tempfile::tempdir().unwrap();
        let manager = crate::core::config::SecureConfigManager::new(temp.path()).unwrap();
        manager
            .set_custom_secret("google_places_api_key", Some("test-key".to_string()))
            .unwrap();

        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let enabled = runtime.list_enabled_actions().await.unwrap();

        assert!(enabled.iter().any(|action| action.name == "places"));
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

    #[tokio::test]
    async fn connector_request_rejects_local_and_private_targets() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

        let localhost = runtime
            .validate_connector_request_url("http://127.0.0.1:8990/health")
            .await
            .unwrap_err();
        assert!(
            localhost.to_string().contains("localhost")
                || localhost.to_string().contains("loopback")
        );

        let private_ip = runtime
            .validate_connector_request_url("http://10.0.0.8/internal")
            .await
            .unwrap_err();
        assert!(private_ip.to_string().contains("private"));
    }

    #[test]
    fn docker_unavailable_error_fails_closed() {
        let shell_error = ActionRuntime::docker_required_error("shell").to_string();
        let code_error = ActionRuntime::docker_required_error("code_execute").to_string();

        assert!(shell_error.contains("Docker is required"));
        assert!(shell_error.contains("shell"));
        assert!(code_error.contains("code_execute"));
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
    fn workspace_alias_paths_remap_to_workspace_root() {
        let runtime = ActionRuntime {
            config: RuntimeConfig::default(),
            sandbox: ActionSandbox::new(&RuntimeConfig::default()).unwrap(),
            transactions: tokio::sync::Mutex::new(TransactionManager::new(PathBuf::from(
                "snapshots",
            ))),
            actions: tokio::sync::RwLock::new(HashMap::new()),
            disabled_actions: tokio::sync::RwLock::new(HashSet::new()),
            disabled_actions_file: PathBuf::from("./disabled_actions.json"),
            action_reviews: tokio::sync::RwLock::new(HashMap::new()),
            action_reviews_file: PathBuf::from("./action_reviews.json"),
            capability_run_contexts: tokio::sync::RwLock::new(HashMap::new()),
            removed_bundled_actions: tokio::sync::RwLock::new(HashSet::new()),
            removed_bundled_actions_file: PathBuf::from("./removed_bundled_actions.json"),
            actions_dir: PathBuf::from("./skills"),
            cli_skills_dir: PathBuf::from("./cli_skills"),
            config_dir: PathBuf::from("."),
            auto_approved_actions: std::sync::RwLock::new(HashSet::new()),
            tool_args_guard_config: std::sync::RwLock::new(Default::default()),
            task_queue: None,
            action_guard: None,
            storage: None,
            embedding_client: None,
            current_user_id: None,
            mcp_registry: None,
            plugin_registry: None,
            extension_pack_registry: None,
            #[cfg(feature = "docker")]
            active_sandbox_containers: tokio::sync::RwLock::new(HashSet::new()),
            #[cfg(feature = "docker")]
            container_reaper_status: tokio::sync::RwLock::new(ContainerReaperStatus::default()),
        };
        let workspace_root = runtime.workspace_root();
        assert_eq!(
            runtime
                .absolutize_tool_path("/workspace/demo/index.html")
                .unwrap(),
            workspace_root.join("demo").join("index.html")
        );
    }

    #[test]
    fn find_project_root_from_path_walks_up_to_cargo_toml() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let nested = root.join("target").join("release");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let exe_path = nested.join(if cfg!(windows) {
            "agentark.exe"
        } else {
            "agentark"
        });
        std::fs::write(&exe_path, "").unwrap();

        let detected = ActionRuntime::find_project_root_from_path(&exe_path)
            .expect("project root should be detected");
        assert_eq!(detected, root);
    }

    #[test]
    fn runtime_owned_bundled_dirs_are_disabled() {
        assert!(!ActionRuntime::is_runtime_owned_bundled_dir(Path::new(
            "/app/repo-skills"
        )));
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
                std::env::var("ComSpec")
                    .or_else(|_| std::env::var("COMSPEC"))
                    .unwrap_or_else(|_| "C:\\WINDOWS\\system32\\cmd.exe".to_string())
            } else {
                "sh".to_string()
            },
            verify_args: vec![],
            source_url: None,
        };

        runtime
            .install_cli_skill_action(
                manifest.clone(),
                "---\nname: echo-cli\ndescription: Echo CLI\n---\n# echo-cli\n",
            )
            .await
            .unwrap();

        let args = if cfg!(windows) {
            serde_json::json!({ "args": ["/C", "echo", "ready"] })
        } else {
            serde_json::json!({ "args": ["-lc", "printf ready"] })
        };
        let output = runtime
            .execute_cli_action(
                "echo-cli",
                CliToolBinding {
                    executable_path: manifest.executable_path.clone(),
                    verify_args: manifest.verify_args.clone(),
                    auth_profile_id: None,
                    auth_env_exports: BTreeMap::new(),
                },
                &args,
            )
            .await
            .unwrap();
        assert!(output.contains("ready"));
    }

    async fn runtime_for_authorization_tests() -> ActionRuntime {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        runtime
    }

    async fn runtime_for_permission_gate_tests() -> ActionRuntime {
        let temp = tempfile::tempdir().unwrap();
        let mut runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        let guard = crate::security::ActionGuard::new(
            &ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]),
            "did:key:test",
            temp.path(),
            temp.path(),
        )
        .await
        .unwrap();
        runtime.set_action_guard(std::sync::Arc::new(guard));
        runtime.load_builtin_actions().await.unwrap();
        runtime
    }

    async fn action_def_by_name(runtime: &ActionRuntime, name: &str) -> ActionDef {
        runtime
            .list_actions()
            .await
            .unwrap()
            .into_iter()
            .find(|action| action.name == name)
            .expect("action should exist")
    }

    #[tokio::test]
    async fn vision_ocr_is_read_only_and_not_media_generation_gated() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "vision_ocr").await;

        assert!(action.capabilities.contains(&"vision_ocr".to_string()));
        assert!(!action
            .capabilities
            .contains(&"image_generation".to_string()));
        assert!(action.authorization.access.integration_ids.is_empty());
    }

    fn vision_test_slot(
        id: &str,
        role: crate::core::config::ModelRole,
        provider: crate::core::LlmProvider,
    ) -> crate::core::config::ModelSlot {
        crate::core::config::ModelSlot {
            id: id.to_string(),
            label: id.to_string(),
            role,
            provider,
            enabled: true,
            capability_tier: crate::core::config::ModelCapabilityTier::Balanced,
            cost_tier: crate::core::config::ModelCostTier::Medium,
            auto_escalate: true,
            escalation_rank: 0,
            health_scope: crate::core::config::ModelHealthScope::Provider,
        }
    }

    #[test]
    fn configured_chat_vision_candidates_prefer_model_pool_primary() {
        let mut config = crate::core::config::AgentConfig::default();
        config.llm = crate::core::LlmProvider::OpenAI {
            api_key: "legacy-key".to_string(),
            model: "legacy-model".to_string(),
            base_url: None,
        };
        config.model_pool.slots = vec![
            vision_test_slot(
                "fast",
                crate::core::config::ModelRole::Fast,
                crate::core::LlmProvider::OpenAI {
                    api_key: "fast-key".to_string(),
                    model: "fast-model".to_string(),
                    base_url: Some(crate::core::llm_provider::OPENROUTER_API_BASE_URL.to_string()),
                },
            ),
            vision_test_slot(
                "primary",
                crate::core::config::ModelRole::Primary,
                crate::core::LlmProvider::OpenAI {
                    api_key: "primary-key".to_string(),
                    model: "primary-model".to_string(),
                    base_url: Some(crate::core::llm_provider::OPENROUTER_API_BASE_URL.to_string()),
                },
            ),
        ];

        let candidates = ActionRuntime::openai_compatible_chat_vision_candidates(&config);

        assert_eq!(candidates[0].model, "primary-model");
        assert_eq!(candidates[0].provider_label(), "openrouter");
        assert!(candidates
            .iter()
            .any(|candidate| candidate.model == "legacy-model"));
    }

    #[test]
    fn configured_chat_vision_candidates_skip_missing_managed_provider_keys() {
        let mut config = crate::core::config::AgentConfig::default();
        config.llm = crate::core::LlmProvider::Anthropic {
            api_key: "anthropic-key".to_string(),
            model: "text-model".to_string(),
        };
        config.model_pool.slots = vec![
            vision_test_slot(
                "missing-openrouter",
                crate::core::config::ModelRole::Primary,
                crate::core::LlmProvider::OpenAI {
                    api_key: String::new(),
                    model: "openrouter-model".to_string(),
                    base_url: Some(crate::core::llm_provider::OPENROUTER_API_BASE_URL.to_string()),
                },
            ),
            vision_test_slot(
                "local-compatible",
                crate::core::config::ModelRole::Fallback,
                crate::core::LlmProvider::OpenAI {
                    api_key: String::new(),
                    model: "local-vision-model".to_string(),
                    base_url: Some("http://127.0.0.1:11434/v1".to_string()),
                },
            ),
        ];

        let candidates = ActionRuntime::openai_compatible_chat_vision_candidates(&config);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].model, "local-vision-model");
        assert_eq!(candidates[0].provider_label(), "openai-compatible");
    }

    #[test]
    fn configured_chat_vision_candidates_dedupe_legacy_primary_copy() {
        let mut config = crate::core::config::AgentConfig::default();
        let provider = crate::core::LlmProvider::OpenAI {
            api_key: "shared-key".to_string(),
            model: "same-model".to_string(),
            base_url: Some(crate::core::llm_provider::OPENROUTER_API_BASE_URL.to_string()),
        };
        config.llm = provider.clone();
        config.model_pool.slots = vec![vision_test_slot(
            "primary",
            crate::core::config::ModelRole::Primary,
            provider,
        )];

        let candidates = ActionRuntime::openai_compatible_chat_vision_candidates(&config);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].model, "same-model");
    }

    #[tokio::test]
    async fn document_lookup_has_native_executor() {
        let runtime = runtime_for_authorization_tests().await;
        let error = runtime
            .execute_action_with_context(
                "document_lookup",
                &serde_json::json!({"query": "uploaded document"}),
                &trusted_chat_context("native-document-lookup", false),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(!error.contains("Unknown native action"));
        assert!(
            error.contains("storage") || error.contains("not available"),
            "unexpected document_lookup error: {error}"
        );
    }

    #[tokio::test]
    async fn list_watchers_has_native_executor() {
        let runtime = runtime_for_authorization_tests().await;
        let error = runtime
            .execute_action_with_context(
                "list_watchers",
                &serde_json::json!({"filter": "all"}),
                &trusted_chat_context("native-list-watchers", false),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(!error.contains("Unknown native action"));
        assert!(
            error.contains("Storage not available") || error.contains("not available"),
            "unexpected list_watchers error: {error}"
        );
    }

    #[tokio::test]
    async fn app_and_automation_action_contracts_separate_cadence_ownership() {
        let runtime = runtime_for_authorization_tests().await;
        let app_deploy = action_def_by_name(&runtime, "app_deploy").await;
        let schedule_task = action_def_by_name(&runtime, "schedule_task").await;
        let watch = action_def_by_name(&runtime, "watch").await;
        let background_session_manage =
            action_def_by_name(&runtime, "background_session_manage").await;

        assert!(app_deploy
            .description
            .contains("implement that behavior inside the artifact"));
        assert!(schedule_task
            .description
            .contains("cadence that belongs inside a generated app"));
        assert!(watch
            .description
            .contains("inside a generated app, dashboard, page, or tool's own UI"));
        assert!(background_session_manage
            .description
            .contains("durable AgentArk background session"));
        assert!(background_session_manage
            .description
            .contains("not app-internal refresh/poll cadence"));
    }

    fn trusted_chat_context(
        capability_context_id: &str,
        current_turn_is_explicit_approval: bool,
    ) -> ActionAuthorizationContext {
        ActionAuthorizationContext {
            principal: Some(ActionCallerPrincipal::local_admin("test")),
            surface: ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: Some(capability_context_id.to_string()),
        }
    }

    #[tokio::test]
    async fn trusted_chat_allows_high_risk_builtin_actions() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "code_execute").await;
        let decision = runtime
            .authorize_action_invocation(
                "code_execute",
                Some(&action),
                &serde_json::json!({}),
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn lan_discover_requires_explicit_approval_for_trusted_chat() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "lan_discover").await;
        assert!(action
            .capabilities
            .iter()
            .any(|cap| cap == "local_network_discovery"));

        let decision = runtime
            .authorize_action_invocation(
                "lan_discover",
                Some(&action),
                &serde_json::json!({ "target": "sonos" }),
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(!decision.allowed);
        assert!(decision.reason.contains("explicit user approval"));
    }

    #[tokio::test]
    async fn lan_discover_allows_explicit_approval_turn() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "lan_discover").await;
        let decision = runtime
            .authorize_action_invocation(
                "lan_discover",
                Some(&action),
                &serde_json::json!({ "target": "sonos" }),
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: true,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn trusted_api_allows_high_risk_builtin_actions() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "shell").await;
        let decision = runtime
            .authorize_action_invocation(
                "shell",
                Some(&action),
                &serde_json::json!({ "command": "pwd" }),
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Api,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn background_blocks_high_risk_builtin_actions() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "app_deploy").await;
        let decision = runtime
            .authorize_action_invocation(
                "app_deploy",
                Some(&action),
                &serde_json::json!({}),
                &ActionAuthorizationContext {
                    principal: None,
                    surface: ActionExecutionSurface::Background,
                    direct_user_intent: false,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(!decision.allowed);
        assert!(decision.reason.contains("background or automation"));
    }

    #[tokio::test]
    async fn read_only_background_actions_still_work() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "file_read").await;
        let decision = runtime
            .authorize_action_invocation(
                "file_read",
                Some(&action),
                &serde_json::json!({ "path": "README.md" }),
                &ActionAuthorizationContext {
                    principal: None,
                    surface: ActionExecutionSurface::Background,
                    direct_user_intent: false,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn api_without_principal_blocks_high_risk_actions() {
        let runtime = runtime_for_authorization_tests().await;
        let action = action_def_by_name(&runtime, "shell").await;
        let decision = runtime
            .authorize_action_invocation(
                "shell",
                Some(&action),
                &serde_json::json!({ "command": "pwd" }),
                &ActionAuthorizationContext {
                    principal: None,
                    surface: ActionExecutionSurface::Api,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(!decision.allowed);
        assert!(decision.reason.contains("trusted local session"));
    }

    #[tokio::test]
    async fn trusted_chat_bypasses_permission_gate_for_code_execute() {
        let runtime = runtime_for_permission_gate_tests().await;
        let action = action_def_by_name(&runtime, "code_execute").await;
        let unapproved = runtime
            .unapproved_permissions_for_action(
                &action,
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: None,
                },
            )
            .await;

        assert!(unapproved.is_empty());
    }

    #[tokio::test]
    async fn runtime_correlation_requires_approval_for_sensitive_read_then_external_send() {
        let runtime = runtime_for_authorization_tests().await;
        let memory = action_def_by_name(&runtime, "memory_lookup").await;
        let schedule = action_def_by_name(&runtime, "schedule_task").await;
        let context = trusted_chat_context("test-sensitive-send", false);

        let read_decision = runtime
            .authorize_action_invocation(
                "memory_lookup",
                Some(&memory),
                &serde_json::json!({ "query": "saved user context" }),
                &context,
            )
            .await
            .unwrap();
        assert!(read_decision.allowed);

        let send_decision = runtime
            .authorize_action_invocation(
                "schedule_task",
                Some(&schedule),
                &serde_json::json!({
                    "task": "Send a summary",
                    "at": "2026-04-18T12:00:00+05:30",
                    "report_to": "ext.custom.ops"
                }),
                &context,
            )
            .await
            .unwrap();

        assert!(!send_decision.allowed);
        assert!(send_decision.requires_explicit_approval);
    }

    #[tokio::test]
    async fn runtime_correlation_allows_sensitive_send_after_explicit_approval() {
        let runtime = runtime_for_authorization_tests().await;
        let memory = action_def_by_name(&runtime, "memory_lookup").await;
        let schedule = action_def_by_name(&runtime, "schedule_task").await;
        let context = trusted_chat_context("test-sensitive-send-approved", false);

        assert!(
            runtime
                .authorize_action_invocation(
                    "memory_lookup",
                    Some(&memory),
                    &serde_json::json!({ "query": "saved user context" }),
                    &context,
                )
                .await
                .unwrap()
                .allowed
        );

        let approved_context = trusted_chat_context("test-sensitive-send-approved", true);
        let send_decision = runtime
            .authorize_action_invocation(
                "schedule_task",
                Some(&schedule),
                &serde_json::json!({
                    "task": "Send a summary",
                    "at": "2026-04-18T12:00:00+05:30",
                    "report_to": "ext.custom.ops"
                }),
                &approved_context,
            )
            .await
            .unwrap();

        assert!(send_decision.allowed);
    }

    #[tokio::test]
    async fn anonymous_context_still_requires_code_execute_permission() {
        let runtime = runtime_for_permission_gate_tests().await;
        let action = action_def_by_name(&runtime, "code_execute").await;
        let unapproved = runtime
            .unapproved_permissions_for_action(&action, &ActionAuthorizationContext::default())
            .await;

        assert!(unapproved
            .iter()
            .any(|perm| matches!(perm, crate::security::action_guard::Permission::CodeExecute)));
    }

    #[tokio::test]
    async fn scoped_agent_blocks_unattached_channel_targets() {
        let runtime = runtime_for_authorization_tests().await;
        let decision = runtime
            .authorize_action_invocation(
                "schedule_task",
                None,
                &serde_json::json!({
                    "task": "Send me a daily summary",
                    "cron": "0 9 * * *",
                    "report_to": "slack"
                }),
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: Some("Ops Bot".to_string()),
                    agent_access_scope: Some(crate::core::swarm::AgentAccessScope {
                        channel_ids: vec!["teams".to_string()],
                        ..Default::default()
                    }),
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(!decision.allowed);
        assert!(decision.reason.contains("slack"));
    }

    #[cfg(feature = "ssh")]
    #[tokio::test]
    async fn scoped_agent_blocks_unattached_ssh_connection_names() {
        let runtime = runtime_for_authorization_tests().await;
        let decision = runtime
            .authorize_action_invocation(
                "ssh",
                None,
                &serde_json::json!({
                    "connection": "staging-box",
                    "command": "pwd"
                }),
                &ActionAuthorizationContext {
                    principal: Some(ActionCallerPrincipal::local_admin("test")),
                    surface: ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: Some("Infra Bot".to_string()),
                    agent_access_scope: Some(crate::core::swarm::AgentAccessScope {
                        ssh_connection_names: vec!["prod-box".to_string()],
                        ..Default::default()
                    }),
                    capability_context_id: None,
                },
            )
            .await
            .unwrap();

        assert!(!decision.allowed);
        assert!(decision.reason.contains("staging-box"));
    }

    #[test]
    fn outbound_gate_respects_explicit_action_metadata() {
        let mut action = ActionDef::default();
        action.authorization.outbound.read_only = true;
        assert!(!ActionRuntime::action_def_requires_outbound_gate(&action));

        action.authorization.outbound.read_only = false;
        action.authorization.outbound.outbound_write = true;
        assert!(ActionRuntime::action_def_requires_outbound_gate(&action));

        action.authorization.outbound.outbound_write = false;
        action.authorization.outbound.public_publish = true;
        assert!(ActionRuntime::action_def_requires_outbound_gate(&action));
    }
}
