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
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::actions::ActionCallerPrincipal;
use crate::actions::{
    ActionAuthorization, ActionAuthorizationContext, ActionAuthorizationDecision, ActionDef,
    ActionExecutionSurface, ActionRiskLevel, ActionSource,
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
        return serde_json::from_str::<StructuredToolCompletion>(payload).ok();
    }
    serde_json::from_str::<StructuredToolCompletion>(output.trim()).ok()
}

fn parse_legacy_tool_completion(
    output: &str,
    tool: &str,
    accepted_prefixes: &[&str],
) -> Option<StructuredToolCompletion> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    let detail = accepted_prefixes.iter().find_map(|prefix| {
        if lower.starts_with(prefix) {
            Some(
                trimmed[prefix.len()..]
                    .trim()
                    .trim_start_matches(':')
                    .trim(),
            )
        } else {
            None
        }
    })?;

    Some(StructuredToolCompletion {
        tool: tool.to_string(),
        status: "completed".to_string(),
        detail: if detail.is_empty() {
            None
        } else {
            Some(detail.to_string())
        },
    })
}

pub fn parse_schedule_task_completion(output: &str) -> Option<StructuredToolCompletion> {
    parse_structured_tool_completion(output)
        .filter(|completion| completion.tool == "schedule_task")
        .or_else(|| {
            parse_legacy_tool_completion(
                output,
                "schedule_task",
                &[
                    "task scheduled:",
                    "scheduled task:",
                    "schedule created:",
                    "schedule added:",
                ],
            )
        })
}

pub fn parse_watch_completion(output: &str) -> Option<StructuredToolCompletion> {
    parse_structured_tool_completion(output)
        .filter(|completion| completion.tool == "watch")
        .or_else(|| {
            parse_legacy_tool_completion(
                output,
                "watch",
                &["watch created:", "watch added:", "watcher created:"],
            )
        })
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
        let uploads_dir = self.data_dir().join("uploads");
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

    pub fn storage(&self) -> Option<crate::storage::Storage> {
        self.storage.clone()
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

    pub fn default_sandbox_mode(&self) -> SandboxMode {
        self.config.default_sandbox.clone()
    }

    pub fn wasm_memory_limit_bytes(&self) -> u64 {
        self.config.wasm_memory_limit
    }

    pub fn docker_image(&self) -> &str {
        &self.config.docker_image
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
            authorization: Default::default(),
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
            authorization: Default::default(),
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
            authorization: Default::default(),
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
            description: "Schedule or update a recurring/one-time AgentArk task. Use this for date/time reminders and notifications that do not require writing to an external calendar. Use `task_id` when the user is changing an existing task from `list_tasks`; otherwise matching tasks are updated/reused unless allow_duplicate=true. Use 'cron' for recurring minute-or-lower-frequency schedules (e.g., daily at 9am = '0 9 * * *') or 'at' for one-time (ISO timestamp). For monitoring intervals below 60 seconds, use `watch` with `interval_secs` instead.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Task description - what to do" },
                    "task_id": { "type": "string", "description": "Optional existing task ID to update. Use this after `list_tasks` or when the user explicitly references an existing routine/task." },
                    "cron": { "type": "string", "description": "Cron expression for recurring tasks. Minute granularity only for schedule_task. Format: 'minute hour day month weekday'. Examples: '0 9 * * *' = daily at 9am, '0 9 * * 1' = every Monday 9am, '*/30 * * * *' = every 30 minutes" },
                    "at": { "type": "string", "description": "ISO 8601 timestamp for one-time task. Example: '2026-02-06T09:00:00+05:30'" },
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
                    { "required": ["task_id", "at"] }
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

        // Background watcher â€” poll an action until a condition is met, then act
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
            description: "Spawn or update a background watcher that polls an action at regular intervals until a condition is met, then executes follow-up instructions. Use `watcher_id` when the user is changing an existing watcher from `list_watchers`; otherwise matching watchers are updated/reused unless allow_duplicate=true. Use when asked to 'watch for', 'wait for', 'monitor', 'let me know when', or 'poll until'. The watcher runs autonomously and notifies the user when triggered or timed out. It supports sub-minute polling via `interval_secs`. Default duration is 24 hours; users can extend it with timeout_hours, timeout_days, timeout_secs, or until_stopped=true.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this watcher does (shown in UI)" },
                    "watcher_id": { "type": "string", "description": "Optional existing watcher ID to update. Use this after `list_watchers` or when the user explicitly references an existing watcher." },
                    "poll_action": { "type": "string", "description": "Action to poll (e.g. 'gmail_scan', 'web_search', 'http_get')" },
                    "poll_arguments": { "type": "object", "description": "Arguments for the poll action" },
                    "condition": {
                        "type": "object",
                        "description": "Structured trigger condition authored by the model. Include a human-readable `description` and an explicit matcher. Prefer `json_predicate` or `json_logic` for structured poll outputs; use `llm` only when the trigger cannot be expressed safely as a deterministic contract.",
                        "properties": {
                            "description": { "type": "string", "description": "Human-readable summary of what counts as a match" },
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
                    "on_trigger": { "type": "string", "description": "What to do when condition is met â€” natural language instructions for the agent" },
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
                    { "required": ["watcher_id"] }
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
            name: "capability_acquire".to_string(),
            description: "Scaffold a reusable integration/action when the needed capability does not already exist. Generates a reviewable custom SKILL.md backed by connector_request and/or browser_auto, registers it immediately, and returns the new action plus any remaining auth/config requirements. Do not use this for extension-pack integrations or connector installs; use the extension_pack_* actions for those.".to_string(),
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
            capabilities: vec!["file_read".to_string()],
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
                    "thread_id": { "type": "string", "description": "Gmail thread ID to reply to (from gmail_scan results)" }
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
            description: "Search the web for current information. Use when asked about news, facts, prices, weather, or anything that needs up-to-date data. Keep the query topic-focused and do not invent specific year filters unless the user explicitly asked for them.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query. Preserve the user's topic, but do not inject arbitrary years or date ranges unless the user explicitly requested them." },
                    "num_results": { "type": "integer", "description": "Number of results (default 5)" },
                    "backend": { "type": "string", "description": "Search backend: lightpanda, duckduckgo, playwright, brave, brave_api, serper" }
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
            description: "Conduct deep research on a topic by gathering diverse source sets, fetching and comparing evidence, surfacing contradictions and open questions, and returning a citation-backed synthesis. Use for complex questions that need thorough investigation beyond a simple web search.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Research topic or question" },
                    "max_sources": { "type": "integer", "description": "Maximum sources to examine (default 5, or 12 when depth='deep')" },
                    "backend": { "type": "string", "description": "Optional search backend override: lightpanda, duckduckgo, playwright, brave, brave_api, serper" },
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
            description: "Execute code in an isolated Docker sandbox. Supports Python, JavaScript, TypeScript, Bash, Ruby, PHP, Perl, Lua, R, Java, C, C++, Go, Rust, Swift, Kotlin, and Jupyter notebooks (.ipynb). Use when the user asks to run, execute, test, or debug code, install dependencies, set up custom local automation, bootstrap monitoring scripts, validate a repo/runtime, or perform scripted device/network/media/computer-vision checks. Ordinary sandbox-local pip/npm installs from standard registries are auto-allowed when needed; higher-risk installer paths such as host package managers, remote installer scripts, and non-registry package sources require explicit approval. For ML/data science and EDA, use language='jupyter' to create executable notebooks with visualizations; they get executed and returned as downloadable .ipynb files. When the user has attached files through the upload API, pass their upload IDs in the 'files' array; they'll be validated and available at /data/<filename> inside the sandbox.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": { "type": "string", "description": "Programming language: python, javascript, typescript, bash, ruby, php, perl, lua, r, java, c, cpp, go, rust, swift, kotlin, jupyter. Use 'jupyter' for EDA/ML notebooks with visualizations." },
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
            capabilities: vec![],
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
            name: "agentark_inspect".to_string(),
            description: "Inspect live AgentArk internal surfaces such as overview, gateway_ops, arkpulse, sentinel, evolution, moltbook, and trace. Use when the user asks about current AgentArk state, recent internal runs, anomalies, learning status, or what changed.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "surface": {
                        "type": "string",
                        "enum": ["overview", "gateway_ops", "arkpulse", "sentinel", "evolution", "moltbook", "trace"],
                        "description": "Internal AgentArk surface to inspect. Default: overview."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum recent records to include per section (default: 6)."
                    },
                    "trace_id": {
                        "type": "string",
                        "description": "Optional execution trace id when surface=trace."
                    }
                }
            }),
            capabilities: vec!["platform_observability".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
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

        // Action management â€” create/update/delete/list custom actions via chat
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
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
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
        authorization: Default::default(),
        }).await;

        // PDF generation â€” creates PDF documents from content
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
        authorization: Default::default(),
        }).await;

        // Expense tracking â€” add, list, summarize, delete expenses
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

        // Security logs â€” query security events from DB
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

        // Browser automation â€” full headless browser control with human-in-the-loop
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
        authorization: integration_authorization("moltbook"),
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

        // Google Calendar â€” list, create, find free time
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

        // SSH â€” remote server execution (behind feature flag)
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
        authorization: Default::default(),
        }).await;

        // App deployment â€” write files, start servers, return live URL
        self.register_builtin_action(ActionDef {
            name: "app_deploy".to_string(),
            description: format!(
                "Deploy a web app or server and return a live URL. Supports either generated files OR a repository source. Use when asked to build a dashboard, create a tool, make a website, build an app, or deploy/run a repo locally for the user. For file-based apps, provide a `files` object. For repo-based apps, provide `repo_url` (and optionally `repo_ref`, `repo_subdir`, `service_mode`) so {} can clone the repo, inspect the README/manifests, stand up the detected frontend/backend services, and return managed endpoints. For generated file bundles, only request a dynamic runtime when it is genuinely needed by setting `runtime_required=true` and providing an `entry_command` (optionally `runtime_reason`). Otherwise the bundle is treated as a static/local app. Repo-based deploys default to container runtime unless overridden. Dynamic app containers default to the installed {} image unless `runtime_image` or a runner-image env override is provided. Public exposure stays off unless explicitly requested (`expose_public=true`). Declare required inputs via required_inputs and mark each item sensitive=true/false. Access guard follows the current app-hosting default when omitted for local/private apps. Public exposure requires `access_password`, and providing `access_password` enables App Guard.",
                crate::branding::PRODUCT_NAME,
                crate::branding::PRODUCT_NAME
            ),
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
                    "title": { "type": "string", "description": "App name/title (default: App)" },
                    "entry_command": {
                        "type": "string",
                        "description": "Command to start the server process (omit for static HTML apps). Use {PORT} placeholder or PORT env var for the port. Python apps auto-activate their venv. Examples: 'python3 app.py', 'node server.js', 'uvicorn app:app --host 0.0.0.0 --port {PORT}'"
                    },
                    "install_command": {
                        "type": "string",
                        "description": "Command to install dependencies before starting (optional). Omit for Python apps with requirements.txt â€” a venv is auto-created. Each app runs in its own isolated environment (Python venv or local node_modules). Examples: 'pip install -r requirements.txt', 'npm install'"
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
                        "description": "Set true only when the generated bundle genuinely needs a long-lived server/runtime. Default: false."
                    },
                    "runtime_reason": {
                        "type": "string",
                        "description": "Optional short explanation of why a dynamic runtime is needed for this generated bundle."
                    },
                    "expose_public": {
                        "type": "boolean",
                        "description": "Whether to expose the app on the configured remote-access provider when available. Default: false."
                    },
                    "access_guard": {
                        "type": "boolean",
                        "description": "Enable access-password guard for the shared app URL. Providing `access_password` enables this automatically. Public exposure requires it."
                    },
                    "access_password": {
                        "type": "string",
                        "description": "Operator-chosen or UI-generated access password. Required when `expose_public=true` and enables App Guard."
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
        authorization: Default::default(),
        }).await;

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
            matches!(
                crate::security::action_guard::ActionGuard::permission_risk(
                    &crate::security::action_guard::ActionGuard::parse_permission(cap)
                ),
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
                .ok_or_else(|| anyhow::anyhow!("Unknown action: {}", action_name))?
        };

        let authorization_decision = self
            .authorize_action_invocation(action_name, Some(&info), arguments, auth_context)
            .await?;
        if !authorization_decision.allowed {
            anyhow::bail!("{}", authorization_decision.reason);
        }
        let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

        if !chat_override {
            match self.refresh_action_review_state(action_name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        anyhow::bail!(
                            "{}",
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to execute.", action_name)
                            })
                        );
                    }
                }
                None if info.source != ActionSource::System => {
                    anyhow::bail!(
                        "Action '{}' has no persisted security review and cannot execute.",
                        action_name
                    );
                }
                None => {}
            }
        }

        if !chat_override {
            if info.source != ActionSource::System {
                let disabled = self.disabled_actions.read().await;
                if disabled.contains(action_name) {
                    anyhow::bail!(
                        "Action '{}' is disabled. Re-enable it in the UI before running.",
                        action_name
                    );
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
                anyhow::bail!(
                    "Action '{}' is unavailable because required integration '{}' is not ready.",
                    action_name,
                    integration_id
                );
            }
        }

        match action_name {
            "http_get" => {
                let url = arguments
                    .get("url")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing URL"))?;
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
                action.extension_pack_binding.clone(),
                action.info.source.clone(),
                action.info.clone(),
            )
        };

        let authorization_decision = self
            .authorize_action_invocation(action_name, Some(&info), arguments, auth_context)
            .await?;
        if !authorization_decision.allowed {
            anyhow::bail!("{}", authorization_decision.reason);
        }
        let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

        if !chat_override {
            match self.refresh_action_review_state(action_name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        anyhow::bail!(
                            "{}",
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to execute.", action_name)
                            })
                        );
                    }
                }
                None if source != ActionSource::System => {
                    anyhow::bail!(
                        "Action '{}' has no persisted security review and cannot execute.",
                        action_name
                    );
                }
                None => {}
            }
        }

        if !chat_override {
            if source != ActionSource::System {
                let disabled = self.disabled_actions.read().await;
                if disabled.contains(action_name) {
                    return Err(anyhow::anyhow!(
                        "Action '{}' is disabled. Re-enable it in the UI before running.",
                        action_name
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
                return Err(anyhow::anyhow!(
                    "Action '{}' is unavailable because required integration '{}' is not ready.",
                    action_name,
                    integration_id
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
                // Return a marker â€” actual watcher creation is handled by Agent::handle_watch
                let desc = arguments
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("watcher");
                Ok(format!("Watch created: {}", desc))
            }
            "manage_actions" => self.execute_manage_actions(arguments).await,
            "agentark_inspect" => self.execute_agentark_inspect(arguments).await,
            "list_integrations" => self.execute_list_integrations(arguments).await,
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
                crate::actions::search::execute_search(&args, &config).await
            }
            "research" => {
                let args: crate::actions::research::ResearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid research arguments: {}", e))?;

                let config = build_search_config(&self.config_dir, self.storage.as_ref()).await;
                crate::actions::research::execute_research(&args, &config).await
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
                                "- [{}] {} {} â€” {} ({}){}\n",
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

    async fn execute_list_integrations(&self, arguments: &serde_json::Value) -> Result<String> {
        let query = arguments.get("query").and_then(|value| value.as_str());
        let kind = arguments.get("kind").and_then(|value| value.as_str());
        let packs = if let Some(registry) = self.extension_pack_registry.clone() {
            let guard = registry.read().await;
            Some(guard.search_packs(query, kind).await?)
        } else {
            None
        };
        let plugins = if let Some(registry) = self.plugin_registry.clone() {
            let guard = registry.read().await;
            Some(guard.list_plugins().await?)
        } else {
            None
        };
        let mcp_servers = if let Some(registry) = self.mcp_registry.clone() {
            let guard = registry.read().await;
            Some(guard.list_servers(false).await?)
        } else {
            None
        };
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "extension_packs": packs,
            "plugins": plugins,
            "mcp_servers": mcp_servers,
        }))?)
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

    async fn load_storage_utf8_value(
        storage: &crate::storage::Storage,
        key: &str,
    ) -> Option<String> {
        storage
            .get(key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| String::from_utf8(raw).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
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

    async fn inspect_moltbook_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let manager = self.settings_manager()?;
        let has_api_key = std::env::var("MOLTBOOK_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_some()
            || manager
                .get_custom_secret("moltbook_api_key")
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .is_some();
        Ok(serde_json::json!({
            "surface": "moltbook",
            "has_api_key": has_api_key,
            "settings": Self::load_storage_json_value(storage, "moltbook_settings_v1").await,
            "last_run": Self::load_storage_utf8_value(storage, "moltbook_last_run_v1").await,
            "last_status": Self::load_storage_utf8_value(storage, "moltbook_last_status_v1").await,
            "next_run": Self::load_storage_utf8_value(storage, "moltbook_next_run_v1").await,
            "last_post": Self::load_storage_utf8_value(storage, "moltbook_last_post_v1").await,
            "last_comment": Self::load_storage_utf8_value(storage, "moltbook_last_comment_v1").await,
            "last_upvote": Self::load_storage_utf8_value(storage, "moltbook_last_upvote_v1").await,
            "last_engagement": Self::load_storage_utf8_value(storage, "moltbook_last_engagement_v1").await,
            "last_run_stats": Self::load_storage_json_value(storage, "moltbook_last_run_stats_v1").await,
            "recent_activity": Self::preview_json_array(
                Self::load_storage_json_value(storage, "moltbook_activity_log_v1").await,
                limit as usize,
            ),
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

    async fn execute_agentark_inspect(&self, arguments: &serde_json::Value) -> Result<String> {
        let storage = self.runtime_storage()?;
        let surface = arguments
            .get("surface")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("overview");
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(6)
            .clamp(1, 24);
        let trace_id = arguments.get("trace_id").and_then(|value| value.as_str());
        let payload = match surface {
            "overview" => serde_json::json!({
                "surface": "overview",
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "gateway_ops": self.inspect_gateway_ops_json(&storage, limit).await?,
                "arkpulse": self.inspect_arkpulse_json(&storage, limit).await?,
                "sentinel": self.inspect_sentinel_json(&storage, limit).await?,
                "evolution": self.inspect_evolution_json(&storage, limit).await?,
                "moltbook": self.inspect_moltbook_json(&storage, limit).await?,
                "trace": self.inspect_trace_json(&storage, None, limit).await?,
            }),
            "gateway_ops" => self.inspect_gateway_ops_json(&storage, limit).await?,
            "arkpulse" => self.inspect_arkpulse_json(&storage, limit).await?,
            "sentinel" => self.inspect_sentinel_json(&storage, limit).await?,
            "evolution" => self.inspect_evolution_json(&storage, limit).await?,
            "moltbook" => self.inspect_moltbook_json(&storage, limit).await?,
            "trace" => self.inspect_trace_json(&storage, trace_id, limit).await?,
            other => {
                anyhow::bail!(
                    "Unknown AgentArk surface '{}'. Use overview, gateway_ops, arkpulse, sentinel, evolution, moltbook, or trace",
                    other
                )
            }
        };
        Ok(serde_json::to_string_pretty(&payload)?)
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
            use rand::Rng;
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
                                    "\nâš ï¸ Security warnings: {}",
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
            // Check Docker availability first â€” fall back to native if unavailable
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
        // Force remove â€” deletes container, volumes, and anonymous volumes
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
    /// Container is ALWAYS destroyed after execution â€” no leftovers.
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

        // Start container â€” if this fails, clean up immediately
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

        // Always destroy the container â€” no leftovers
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

            // Jupyter notebook â€” execute in-place and output results
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
        // Python stdlib modules (comprehensive but not exhaustive â€” errs on side of not installing)
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
    /// Supports any language with a Docker image â€” auto-pulls if needed.
    /// Container is ephemeral â€” fully destroyed after execution.
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
        // LLMs often generate these in regular Python scripts â€” our auto-dependency
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
            "mkdir -p '{sandbox}' '{home}' '{tmp}' '{cache}' '{pip_cache}' '{cache}/npm' /data && export HOME='{home}' TMPDIR='{tmp}' TMP='{tmp}' TEMP='{tmp}' XDG_CACHE_HOME='{cache}' PIP_CACHE_DIR='{pip_cache}' npm_config_cache='{cache}/npm' NPM_CONFIG_CACHE='{cache}/npm' && cd '{sandbox}' && ",
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
            // No file marker found â€” still save the code file
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
            if action.source == ActionSource::System
                && !self.is_action_integration_ready(&action).await
            {
                continue;
            }
            if action.source != ActionSource::System
                && self
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
            } else if action.source != ActionSource::System {
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
        if action.source == ActionSource::System && !self.is_action_integration_ready(&action).await
        {
            return false;
        }
        if action.source != ActionSource::System {
            let disabled = self.disabled_actions.read().await;
            if disabled.contains(name) {
                return false;
            }
        }
        match self.refresh_action_review_state(name).await {
            Ok(Some(review)) => review.allow_execute,
            Ok(None) => action.source == ActionSource::System,
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

    fn resolve_skill_markdown_path(dir: &Path) -> Option<std::path::PathBuf> {
        let skill_md = dir.join("SKILL.md");
        if skill_md.exists() {
            return Some(skill_md);
        }
        None
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

    #[allow(dead_code)]
    pub async fn apply_skill_evolution_candidate(
        &self,
        action: &str,
        name: &str,
        content: &str,
        evidence_markdown: &str,
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
            let verdict = self.create_action(name, content, false).await?;
            if verdict.as_ref().is_some_and(|value| !value.allow_load) {
                anyhow::bail!("skill creation was blocked by the action security guard");
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
        if !self.update_action_content(name, content).await? {
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

        match source {
            ActionSource::System => Ok(false),
            ActionSource::Bundled => {
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
                if let Some(fp) = file_path {
                    let action_path = std::path::Path::new(&fp);
                    if let Some(action_dir) = action_path.parent() {
                        let dir_path = action_dir.to_path_buf();
                        if dir_path.exists() {
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
        let search_config = build_search_config(&self.config_dir, self.storage.as_ref()).await;

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
                                    "Security check failed for '{}': {} â€” loading anyway",
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

/// Build search config: loads user settings from persistent settings storage,
/// injects API-backed secrets, auto-detects runtime-provided builtins such as
/// Lightpanda and the Playwright bridge, and applies the default free fallback
/// chain only when no chain is saved.
pub(crate) async fn build_search_config(
    config_dir: &Path,
    storage: Option<&crate::storage::Storage>,
) -> crate::actions::SearchConfig {
    let mut config = load_persisted_search_config(config_dir, None);

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
    fn parse_schedule_task_completion_accepts_legacy_and_structured_markers() {
        let legacy =
            parse_schedule_task_completion("Task scheduled: backup | Schedule: cron:0 9 * * *")
                .expect("legacy schedule text should parse");
        assert_eq!(legacy.tool, "schedule_task");
        assert_eq!(legacy.status, "completed");
        assert!(legacy
            .detail
            .as_deref()
            .expect("legacy schedule detail")
            .contains("backup"));

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
    fn parse_watch_completion_accepts_legacy_and_structured_markers() {
        let legacy = parse_watch_completion("Watch created: inbox changes")
            .expect("legacy watch text should parse");
        assert_eq!(legacy.tool, "watch");
        assert_eq!(legacy.status, "completed");
        assert!(legacy
            .detail
            .as_deref()
            .expect("legacy watch detail")
            .contains("inbox"));

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
    async fn capability_resolve_is_builtin_and_file_read_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
        runtime.load_builtin_actions().await.unwrap();
        let action = action_def_by_name(&runtime, "capability_resolve").await;

        assert_eq!(action.source, ActionSource::System);
        assert_eq!(action.capabilities, vec!["file_read".to_string()]);
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
            task_queue: None,
            action_guard: None,
            storage: None,
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
