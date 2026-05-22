//! WHOOP Integration

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct WhoopConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl WhoopConnector {
    const API_BASE: &'static str = "https://api.prod.whoop.com/developer/v1";

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: reqwest::Client::new(),
            config_dir,
        }
    }

    fn load_secret(config_dir: &Path, env_key: &str, key: &str) -> Option<String> {
        if let Ok(v) = std::env::var(env_key) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
        crate::core::config::SecureConfigManager::new(config_dir)
            .ok()
            .and_then(|mgr| mgr.get_custom_secret(key).ok().flatten())
    }

    fn token(&self) -> Option<String> {
        Self::load_secret(&self.config_dir, "WHOOP_TOKEN", "whoop_token")
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<serde_json::Value> {
        let token = self
            .token()
            .ok_or_else(|| anyhow!("WHOOP token not configured"))?;
        let mut req = self
            .http
            .get(format!(
                "{}/{}",
                Self::API_BASE,
                path.trim_start_matches('/')
            ))
            .header("Authorization", format!("Bearer {}", token));
        if !query.is_empty() {
            req = req.query(query);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("WHOOP API failed ({}): {}", status, body));
        }
        Ok(resp.json().await?)
    }
}

#[async_trait]
impl Integration for WhoopConnector {
    fn id(&self) -> &str {
        "whoop"
    }
    fn name(&self) -> &str {
        "WHOOP"
    }
    fn description(&self) -> &str {
        "WHOOP performance data connector"
    }
    fn icon(&self) -> &str {
        "whoop"
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read]
    }
    async fn status(&self) -> IntegrationStatus {
        if self.token().is_some() {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }
    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(25)
            .min(100);
        match action {
            "profile" => self.get("/user/profile/basic", &[]).await,
            "recovery" => self.get("/recovery", &[("limit", limit.to_string())]).await,
            "sleep" => {
                self.get("/activity/sleep", &[("limit", limit.to_string())])
                    .await
            }
            "workouts" => {
                self.get("/activity/workout", &[("limit", limit.to_string())])
                    .await
            }
            _ => Err(anyhow!("Unknown WHOOP action: {}", action)),
        }
    }
}
