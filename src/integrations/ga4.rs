//! GA4 Integration

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct Ga4Connector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl Ga4Connector {
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
        Self::load_secret(&self.config_dir, "GA4_ACCESS_TOKEN", "ga4_access_token")
    }

    fn property_id(&self) -> Option<String> {
        Self::load_secret(&self.config_dir, "GA4_PROPERTY_ID", "ga4_property_id")
    }

    async fn run_report(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self
            .token()
            .ok_or_else(|| anyhow!("GA4 token not configured"))?;
        let property_id = params
            .get("property_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.property_id())
            .ok_or_else(|| anyhow!("Missing GA4 property_id"))?;

        let dims = params
            .get("dimensions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| vec![serde_json::json!("date")]);
        let mets = params
            .get("metrics")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| {
                vec![
                    serde_json::json!("sessions"),
                    serde_json::json!("activeUsers"),
                ]
            });
        let date_ranges = params
            .get("date_ranges")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| vec![serde_json::json!({"startDate":"7daysAgo","endDate":"today"})]);
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000)
            .min(10000)
            .to_string();

        let payload = serde_json::json!({
            "dimensions": dims.into_iter().filter_map(|d| d.as_str().map(|s| serde_json::json!({"name": s}))).collect::<Vec<_>>(),
            "metrics": mets.into_iter().filter_map(|m| m.as_str().map(|s| serde_json::json!({"name": s}))).collect::<Vec<_>>(),
            "dateRanges": date_ranges,
            "limit": limit
        });
        let url = format!(
            "https://analyticsdata.googleapis.com/v1beta/properties/{}:runReport",
            property_id
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
            return Err(anyhow!("GA4 run_report failed ({}): {}", status, text));
        }
        Ok(serde_json::json!({
            "property_id": property_id,
            "report": resp.json::<serde_json::Value>().await?
        }))
    }
}

#[async_trait]
impl Integration for Ga4Connector {
    fn id(&self) -> &str {
        "ga4"
    }
    fn name(&self) -> &str {
        "Google Analytics 4"
    }
    fn description(&self) -> &str {
        "GA4 Data API reports"
    }
    fn icon(&self) -> &str {
        "ga4"
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
            "run_report" => self.run_report(params).await,
            _ => Err(anyhow!("Unknown GA4 action: {}", action)),
        }
    }
}
