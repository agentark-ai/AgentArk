use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalServiceHealth {
    pub service: String,
    pub mode: String,
    pub ok: bool,
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorStatusResponse {
    pub service: String,
    pub mode: String,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    #[serde(default)]
    pub workspace_base_url: Option<String>,
    #[serde(default)]
    pub token_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackMemoryContainerStats {
    pub name: String,
    pub memory_used_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackMemoryStatsResponse {
    pub status: String,
    pub source: String,
    pub memory_used_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_pressure_percent: Option<f64>,
    pub container_count: usize,
    #[serde(default)]
    pub containers: Vec<StackMemoryContainerStats>,
    pub sampled_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackUpdateRequest {
    pub release_tag: String,
    pub release_version: String,
    pub release_repo: String,
    pub image_repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackUpdateResponse {
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub release_tag: Option<String>,
    #[serde(default)]
    pub release_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeExecuteFilePayload {
    pub filename: String,
    pub bytes_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeExecuteRequest {
    pub language: String,
    pub code: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub file_payloads: Vec<CodeExecuteFilePayload>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_contract: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_context: Option<crate::actions::ActionAuthorizationContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeExecuteResponse {
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub exec_id: Option<String>,
    #[serde(default)]
    pub output_files: Vec<String>,
    #[serde(default)]
    pub output_text: Option<String>,
    #[serde(default)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDeployRequest {
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub file_patches: Vec<serde_json::Value>,
    #[serde(default)]
    pub delete_paths: Vec<String>,
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub repo_ref: Option<String>,
    #[serde(default)]
    pub repo_subdir: Option<String>,
    #[serde(default)]
    pub service_mode: Option<String>,
    #[serde(default)]
    pub deploy_target: Option<String>,
    #[serde(default)]
    pub production: Option<bool>,
    #[serde(default)]
    pub vercel_project_mode: Option<String>,
    #[serde(default)]
    pub vercel_project_id: Option<String>,
    #[serde(default)]
    pub vercel_team_id: Option<String>,
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub runtime_image: Option<String>,
    #[serde(default)]
    pub runtime_preference: Option<String>,
    #[serde(default)]
    pub runtime_required: Option<bool>,
    #[serde(default)]
    pub expose_public: Option<bool>,
    #[serde(default)]
    pub access_guard: Option<bool>,
    #[serde(default)]
    pub access_password: Option<String>,
    #[serde(default)]
    pub required_inputs: Vec<serde_json::Value>,
    #[serde(default)]
    pub config_values: BTreeMap<String, String>,
    #[serde(default)]
    pub install_command: Option<String>,
    #[serde(default)]
    pub entry_command: Option<String>,
    #[serde(default)]
    pub start_command: Option<String>,
    #[serde(default)]
    pub stop_command: Option<String>,
    #[serde(default)]
    pub commands: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDeployResponse {
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLifecycleRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppActionResponse {
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub raw: serde_json::Value,
}
