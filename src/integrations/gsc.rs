//! Google Search Console Integration

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct GscConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl GscConnector {
    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: crate::core::net::default_outgoing_http_client(),
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
        Self::load_secret(&self.config_dir, "GSC_ACCESS_TOKEN", "gsc_access_token")
    }

    fn site_url(&self) -> Option<String> {
        Self::load_secret(&self.config_dir, "GSC_SITE_URL", "gsc_site_url")
    }

    async fn query(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self
            .token()
            .ok_or_else(|| anyhow!("GSC token not configured"))?;
        let site = params
            .get("site_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.site_url())
            .ok_or_else(|| anyhow!("Missing GSC site_url"))?;
        let start_date = params
            .get("start_date")
            .and_then(|v| v.as_str())
            .unwrap_or("7daysAgo");
        let end_date = params
            .get("end_date")
            .and_then(|v| v.as_str())
            .unwrap_or("today");
        let row_limit = params
            .get("row_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000)
            .min(25000);
        let dimensions = params
            .get("dimensions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| vec![serde_json::json!("query"), serde_json::json!("page")])
            .into_iter()
            .filter_map(|d| d.as_str().map(|s| serde_json::Value::String(s.to_string())))
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "startDate": start_date,
            "endDate": end_date,
            "dimensions": dimensions,
            "rowLimit": row_limit
        });
        let url = format!(
            "https://searchconsole.googleapis.com/webmasters/v3/sites/{}/searchAnalytics/query",
            urlencoding::encode(&site)
        );
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("GSC query failed ({}): {}", status, text));
        }
        Ok(serde_json::json!({
            "site_url": site,
            "result": resp.json::<serde_json::Value>().await?
        }))
    }
}

#[async_trait]
impl Integration for GscConnector {
    fn id(&self) -> &str {
        "gsc"
    }
    fn name(&self) -> &str {
        "Google Search Console"
    }
    fn description(&self) -> &str {
        "Search Console query analytics"
    }
    fn icon(&self) -> &str {
        "gsc"
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Search]
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
            "query" => self.query(params).await,
            _ => Err(anyhow!("Unknown GSC action: {}", action)),
        }
    }
}
