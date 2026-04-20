use anyhow::{Context, Result};
use std::time::Duration;

use super::{load_internal_service_token_from_default_config_dir, InternalServiceKind};

#[derive(Debug, Clone)]
pub struct WorkspaceClientConfig {
    pub base_url: String,
    pub token: Option<String>,
    pub timeout_secs: u64,
}

impl WorkspaceClientConfig {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("AGENTARK_WORKSPACE_URL")
                .or_else(|_| std::env::var("AGENTARK_WORKSPACE_BASE_URL"))
                .unwrap_or_else(|_| "http://127.0.0.1:8992".to_string()),
            token: load_internal_service_token_from_default_config_dir(
                InternalServiceKind::Workspace,
            ),
            timeout_secs: std::env::var("AGENTARK_WORKSPACE_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(30),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceClient {
    config: WorkspaceClientConfig,
    client: reqwest::Client,
}

impl WorkspaceClient {
    pub fn new(config: WorkspaceClientConfig) -> Result<Self> {
        super::validate_internal_service_base_url(&config.base_url, "Workspace service")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("Failed to build workspace client")?;
        Ok(Self { config, client })
    }

    pub fn bearer_token(&self) -> Option<&str> {
        self.config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("change-me"))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
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

    pub async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.request(
            reqwest::Method::PUT,
            &format!("/internal/v1/blobs/{}", urlencoding::encode(path)),
        )
        .body(bytes.to_vec())
        .send()
        .await?
        .error_for_status()?;
        Ok(())
    }

    pub async fn get_blob(&self, path: &str) -> Result<Vec<u8>> {
        Ok(self
            .request(
                reqwest::Method::GET,
                &format!("/internal/v1/blobs/{}", urlencoding::encode(path)),
            )
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec())
    }
}
