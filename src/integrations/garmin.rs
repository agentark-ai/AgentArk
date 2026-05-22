//! Garmin Integration

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct GarminConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl GarminConnector {
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
        Self::load_secret(&self.config_dir, "GARMIN_TOKEN", "garmin_token")
    }

    fn base_url(&self) -> String {
        Self::load_secret(&self.config_dir, "GARMIN_API_BASE", "garmin_api_base")
            .unwrap_or_else(|| "https://apis.garmin.com/wellness-api/rest".to_string())
    }

    fn authed_request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder> {
        let token = self
            .token()
            .ok_or_else(|| anyhow!("Garmin token not configured"))?;
        let url = format!(
            "{}/{}",
            self.base_url().trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        Ok(self
            .http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", token)))
    }

    async fn daily_summary(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let date = params.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let path = if date.is_empty() {
            "daily-summary".to_string()
        } else {
            format!("daily-summary?date={}", urlencoding::encode(date))
        };
        let resp = self
            .authed_request(reqwest::Method::GET, &path)?
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Garmin daily_summary failed ({}): {}",
                status,
                text
            ));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(serde_json::json!({ "summary": body }))
    }

    async fn activities(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let start_date = params
            .get("start_date")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let end_date = params
            .get("end_date")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .min(200);
        let mut query = vec![format!("limit={}", limit)];
        if !start_date.is_empty() {
            query.push(format!("start_date={}", urlencoding::encode(start_date)));
        }
        if !end_date.is_empty() {
            query.push(format!("end_date={}", urlencoding::encode(end_date)));
        }
        let path = format!("activities?{}", query.join("&"));
        let resp = self
            .authed_request(reqwest::Method::GET, &path)?
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Garmin activities failed ({}): {}", status, text));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(serde_json::json!({ "activities": body }))
    }
}

#[async_trait]
impl Integration for GarminConnector {
    fn id(&self) -> &str {
        "garmin"
    }
    fn name(&self) -> &str {
        "Garmin"
    }
    fn description(&self) -> &str {
        "Garmin fitness data connector (summary + activities)"
    }
    fn icon(&self) -> &str {
        "fitness"
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
        match action {
            "daily_summary" => self.daily_summary(params).await,
            "activities" => self.activities(params).await,
            _ => Err(anyhow!("Unknown Garmin action: {}", action)),
        }
    }
}
