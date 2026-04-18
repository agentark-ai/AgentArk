use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

use super::{InternalServiceKind, load_internal_service_token_from_default_config_dir};

#[derive(Debug, Clone)]
pub struct ExecutorClientConfig {
    pub base_url: String,
    pub token: Option<String>,
    pub timeout_secs: u64,
}

impl ExecutorClientConfig {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("AGENTARK_EXECUTOR_URL")
                .or_else(|_| std::env::var("AGENTARK_EXECUTOR_BASE_URL"))
                .unwrap_or_else(|_| "http://127.0.0.1:8991".to_string()),
            token: load_internal_service_token_from_default_config_dir(
                InternalServiceKind::Executor,
            ),
            timeout_secs: std::env::var("AGENTARK_EXECUTOR_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(600),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutorClient {
    config: ExecutorClientConfig,
    client: reqwest::Client,
}

impl ExecutorClient {
    pub fn new(config: ExecutorClientConfig) -> Result<Self> {
        super::validate_internal_service_base_url(&config.base_url, "Executor service")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs.max(1)))
            .build()
            .context("Failed to build executor client")?;
        Ok(Self { config, client })
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    pub fn bearer_token(&self) -> Option<&str> {
        self.config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("change-me"))
    }

    pub(crate) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let builder = self.client.request(method, url);
        if let Some(token) = self
            .config
            .token
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            builder.bearer_auth(token)
        } else {
            builder
        }
    }

    pub async fn execute_code(&self, request: &CodeExecuteRequest) -> Result<CodeExecuteResponse> {
        Ok(self
            .request(reqwest::Method::POST, "/internal/v1/code/execute")
            .json(request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn app_status(&self, app_id: &str) -> Result<AppStatusResponse> {
        let path = format!("/internal/v1/apps/{}/status", app_id);
        Ok(self
            .request(reqwest::Method::GET, &path)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn app_logs(&self, app_id: &str, tail: usize) -> Result<AppLogsResponse> {
        let path = format!("/internal/v1/apps/{}/logs?tail={}", app_id, tail.max(256));
        Ok(self
            .request(reqwest::Method::GET, &path)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
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
    pub execution_contract: Option<Value>,
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
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLifecycleRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatusResponse {
    pub status: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub runtime_mode: Option<String>,
    #[serde(default)]
    pub is_isolated_runtime: bool,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLogsResponse {
    pub status: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub logs: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub raw: Value,
}
