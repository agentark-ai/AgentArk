//! Home Assistant integration.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct HomeAssistantConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl HomeAssistantConnector {
    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| crate::core::runtime::net::build_outgoing_http_client(15)),
            config_dir,
        }
    }

    fn configured_value(config_dir: &Path, user_key: &str, custom_key: &str) -> Option<String> {
        if let Ok(value) = std::env::var(user_key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        let manager = crate::core::runtime::config::SecureConfigManager::new(config_dir).ok()?;
        for key in crate::core::runtime::secrets::storage_keys_for_user_key(user_key) {
            if let Ok(Some(value)) = manager.get_custom_secret(&key) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        manager
            .get_custom_secret(custom_key)
            .ok()
            .flatten()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn base_url(&self) -> Result<reqwest::Url> {
        let Some(raw) =
            Self::configured_value(&self.config_dir, "HOME_ASSISTANT_URL", "home_assistant_url")
        else {
            anyhow::bail!(
                "Home Assistant URL is not configured. Set HOME_ASSISTANT_URL or store home_assistant_url."
            );
        };
        let parsed = reqwest::Url::parse(raw.trim().trim_end_matches('/'))
            .with_context(|| "Home Assistant URL is invalid")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("Home Assistant URL must use http:// or https://");
        }
        Ok(parsed)
    }

    fn token(&self) -> Result<String> {
        Self::configured_value(
            &self.config_dir,
            "HOME_ASSISTANT_TOKEN",
            "home_assistant_token",
        )
        .ok_or_else(|| {
            anyhow!(
                "Home Assistant token is not configured. Set HOME_ASSISTANT_TOKEN or store home_assistant_token."
            )
        })
    }

    fn build_url(&self, path_segments: &[&str]) -> Result<reqwest::Url> {
        let mut url = self.base_url()?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Failed to build Home Assistant API URL"))?;
            segments.push("api");
            for segment in path_segments {
                segments.push(segment);
            }
        }
        Ok(url)
    }

    fn authed_request(
        &self,
        method: reqwest::Method,
        path_segments: &[&str],
    ) -> Result<reqwest::RequestBuilder> {
        let url = self.build_url(path_segments)?;
        Ok(self
            .http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.token()?))
            .header("Accept", "application/json"))
    }

    async fn send_json(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Home Assistant API returned {}: {}", status, text);
        }
        if text.trim().is_empty() {
            return Ok(serde_json::json!({ "ok": true }));
        }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "body": text })))
    }

    fn terms(query: Option<&str>) -> Vec<String> {
        query
            .unwrap_or_default()
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    fn score_entity(entity: &Value, terms: &[String]) -> usize {
        if terms.is_empty() {
            return 1;
        }
        let entity_id = entity
            .get("entity_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let friendly = entity
            .pointer("/attributes/friendly_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let state = entity
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let haystack = format!("{entity_id} {friendly} {state}").to_ascii_lowercase();
        terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count()
    }

    fn compact_entity(entity: Value) -> Value {
        let attributes = entity.get("attributes").cloned().unwrap_or(Value::Null);
        serde_json::json!({
            "entity_id": entity.get("entity_id"),
            "state": entity.get("state"),
            "friendly_name": attributes.get("friendly_name"),
            "unit_of_measurement": attributes.get("unit_of_measurement"),
            "device_class": attributes.get("device_class"),
            "last_changed": entity.get("last_changed"),
            "last_updated": entity.get("last_updated"),
        })
    }

    async fn list_entities(&self, params: &Value) -> Result<Value> {
        let states = self
            .send_json(self.authed_request(reqwest::Method::GET, &["states"])?)
            .await?;
        let domain = params
            .get("domain")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let terms = Self::terms(params.get("query").and_then(Value::as_str));
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 200) as usize;
        let Some(items) = states.as_array() else {
            return Ok(states);
        };

        let mut scored = items
            .iter()
            .filter(|entity| {
                let Some(domain) = domain else {
                    return true;
                };
                entity
                    .get("entity_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with(&format!("{domain}.")))
            })
            .map(|entity| (Self::score_entity(entity, &terms), entity.clone()))
            .collect::<Vec<_>>();
        scored.sort_by_key(|item| std::cmp::Reverse(item.0));

        Ok(serde_json::json!({
            "count": scored.len(),
            "entities": scored
                .into_iter()
                .take(limit)
                .map(|(_, entity)| Self::compact_entity(entity))
                .collect::<Vec<_>>()
        }))
    }

    async fn get_state(&self, params: &Value) -> Result<Value> {
        let entity_id = params
            .get("entity_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("home_assistant get_state requires entity_id"))?;
        self.send_json(self.authed_request(reqwest::Method::GET, &["states", entity_id])?)
            .await
    }

    async fn get_services(&self) -> Result<Value> {
        self.send_json(self.authed_request(reqwest::Method::GET, &["services"])?)
            .await
    }

    async fn call_service(&self, params: &Value) -> Result<Value> {
        let domain = params
            .get("domain")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("home_assistant_call_service requires domain"))?;
        let service = params
            .get("service")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("home_assistant_call_service requires service"))?;
        let mut body = params
            .get("service_data")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if !body.is_object() {
            anyhow::bail!("service_data must be a JSON object");
        }
        if let Some(target) = params.get("target") {
            body["target"] = target.clone();
        }
        if let Some(entity_id) = params.get("entity_id").and_then(Value::as_str) {
            body["entity_id"] = Value::String(entity_id.to_string());
        }
        self.send_json(
            self.authed_request(reqwest::Method::POST, &["services", domain, service])?
                .json(&body),
        )
        .await
    }
}

#[async_trait]
impl Integration for HomeAssistantConnector {
    fn id(&self) -> &str {
        "home_assistant"
    }

    fn name(&self) -> &str {
        "Home Assistant"
    }

    fn description(&self) -> &str {
        "Read Home Assistant state and call services on configured devices."
    }

    fn icon(&self) -> &str {
        "HA"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Search, Capability::Write]
    }

    async fn status(&self) -> IntegrationStatus {
        if self.base_url().is_err() || self.token().is_err() {
            return IntegrationStatus::NotConfigured;
        }
        match self.authed_request(reqwest::Method::GET, &[]) {
            Ok(request) => match request.send().await {
                Ok(response) if response.status().is_success() => IntegrationStatus::Connected,
                Ok(response) => {
                    IntegrationStatus::Error(format!("API returned {}", response.status()))
                }
                Err(error) => IntegrationStatus::Error(format!("Connection failed: {}", error)),
            },
            Err(error) => IntegrationStatus::Error(error.to_string()),
        }
    }

    async fn execute(&self, action: &str, params: &Value) -> Result<Value> {
        match action {
            "list_entities" | "search_entities" => self.list_entities(params).await,
            "get_state" => self.get_state(params).await,
            "get_services" => self.get_services().await,
            "call_service" => self.call_service(params).await,
            _ => Err(anyhow!("Unknown Home Assistant action: {}", action)),
        }
    }
}
