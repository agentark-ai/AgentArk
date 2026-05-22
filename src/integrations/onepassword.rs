//! 1Password Integration
//!
//! Provides access to 1Password vaults and items via the Connect API.
//! Supports listing vaults, searching items, retrieving item metadata,
//! and creating new items. For security, secret/password field values
//! are NEVER returned -- only metadata is exposed.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// 1Password Connect API connector
pub struct OnePasswordConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl OnePasswordConnector {
    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_config_dir(config_dir)
    }

    /// API base URL derived from the configured server URL
    fn api_base(&self) -> String {
        let base = Self::load_server_url_from(&self.config_dir);
        format!("{}/v1", base.trim_end_matches('/'))
    }

    /// Load Connect token from environment variable or secure config
    fn load_token_from(config_dir: &Path) -> Option<String> {
        // Backward compatible alias (README / older configs)
        if let Ok(token) = std::env::var("ONEPASSWORD_TOKEN") {
            if !token.is_empty() {
                return Some(token);
            }
        }
        if let Ok(token) = std::env::var("OP_CONNECT_TOKEN") {
            if !token.is_empty() {
                return Some(token);
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("onepassword_token")
                .ok()
                .flatten(),
            Err(_) => None,
        }
    }

    /// Load Connect server URL from environment variable, secure config, or default
    fn load_server_url_from(config_dir: &Path) -> String {
        if let Ok(host) = std::env::var("OP_CONNECT_HOST") {
            if !host.is_empty() {
                return host;
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("onepassword_host")
                .ok()
                .flatten()
                .unwrap_or_else(|| "http://localhost:8080".to_string()),
            Err(_) => "http://localhost:8080".to_string(),
        }
    }

    /// Get the bearer token or return an error
    fn token(&self) -> Result<String> {
        Self::load_token_from(&self.config_dir).ok_or_else(|| {
            anyhow!("1Password Connect token not configured. Set OP_CONNECT_TOKEN or store via secure config.")
        })
    }

    /// GET /vaults - List all vaults accessible to the Connect token
    async fn list_vaults(&self, _params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        let url = format!("{}/vaults", self.api_base());

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("1Password list_vaults failed ({}): {}", status, error_text);
            return Err(anyhow!("1Password API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;

        let vaults: Vec<serde_json::Value> = body
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|v| {
                serde_json::json!({
                    "id": v.get("id"),
                    "name": v.get("name"),
                    "description": v.get("description"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "vaults": vaults,
            "count": vaults.len(),
        }))
    }

    /// GET /vaults/{vault_id}/items/{item_id} - Get item METADATA only (never secrets)
    async fn get_item(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        let vault_id = params
            .get("vault_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'vault_id' parameter"))?;

        let item_id = params
            .get("item_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'item_id' parameter"))?;

        let url = format!("{}/vaults/{}/items/{}", self.api_base(), vault_id, item_id);

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("1Password get_item failed ({}): {}", status, error_text);
            return Err(anyhow!("1Password API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;

        // Extract URLs array if present
        let urls: Vec<serde_json::Value> = body
            .get("urls")
            .and_then(|u| u.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|u| {
                serde_json::json!({
                    "href": u.get("href"),
                    "primary": u.get("primary"),
                })
            })
            .collect();

        // SECURITY: Return ONLY metadata -- never password/secret field values
        Ok(serde_json::json!({
            "id": body.get("id"),
            "title": body.get("title"),
            "category": body.get("category"),
            "tags": body.get("tags"),
            "urls": urls,
            "created_at": body.get("createdAt"),
            "updated_at": body.get("updatedAt"),
            "note": "Secret/password field values are intentionally omitted for security.",
        }))
    }

    /// GET /vaults/{vault_id}/items?filter=title co "{query}" - Search items by title
    async fn search(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        let vault_id = params
            .get("vault_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'vault_id' parameter"))?;

        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'query' parameter"))?;

        let filter = format!("title co \"{}\"", query);

        let url = format!(
            "{}/vaults/{}/items?filter={}",
            self.api_base(),
            vault_id,
            urlencoding::encode(&filter)
        );

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("1Password search failed ({}): {}", status, error_text);
            return Err(anyhow!("1Password API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;

        // Return only metadata for each item
        let items: Vec<serde_json::Value> = body
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|item| {
                serde_json::json!({
                    "id": item.get("id"),
                    "title": item.get("title"),
                    "category": item.get("category"),
                    "tags": item.get("tags"),
                    "vault_id": vault_id,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "items": items,
            "count": items.len(),
            "vault_id": vault_id,
            "query": query,
        }))
    }

    /// POST /vaults/{vault_id}/items - Create a new item in a vault
    async fn create_item(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        let vault_id = params
            .get("vault_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'vault_id' parameter"))?;

        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'title' parameter"))?;

        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("LOGIN");

        // Build fields array from params
        let fields: Vec<serde_json::Value> = params
            .get("fields")
            .and_then(|f| f.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|field| {
                let field_type = field
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("STRING");
                let purpose = match field.get("label").and_then(|v| v.as_str()).unwrap_or("") {
                    "username" => Some("USERNAME"),
                    "password" => Some("PASSWORD"),
                    _ => None,
                };
                let mut f = serde_json::json!({
                    "label": field.get("label"),
                    "value": field.get("value"),
                    "type": field_type,
                });
                if let Some(p) = purpose {
                    f["purpose"] = serde_json::json!(p);
                }
                f
            })
            .collect();

        let body = serde_json::json!({
            "vault": {
                "id": vault_id,
            },
            "title": title,
            "category": category,
            "fields": fields,
        });

        let url = format!("{}/vaults/{}/items", self.api_base(), vault_id);

        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("1Password create_item failed ({}): {}", status, error_text);
            return Err(anyhow!("1Password API error ({}): {}", status, error_text));
        }

        let result: serde_json::Value = response.json().await?;

        // Return metadata only (no secret values)
        Ok(serde_json::json!({
            "id": result.get("id"),
            "title": result.get("title"),
            "category": result.get("category"),
            "vault_id": vault_id,
            "created": true,
        }))
    }
}

#[async_trait]
impl Integration for OnePasswordConnector {
    fn id(&self) -> &str {
        "onepassword"
    }

    fn name(&self) -> &str {
        "1Password"
    }

    fn description(&self) -> &str {
        "Access 1Password vaults and items via Connect API - list, search, and create items (secrets never exposed)"
    }

    fn icon(&self) -> &str {
        "🔐"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if Self::load_token_from(&self.config_dir).is_none() {
            return IntegrationStatus::NotConfigured;
        }
        // Optionally verify connectivity with a lightweight call
        let server_url = Self::load_server_url_from(&self.config_dir);
        let url = format!("{}/heartbeat", server_url.trim_end_matches('/'));
        match self.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => IntegrationStatus::Connected,
            Ok(resp) => {
                tracing::warn!("1Password Connect heartbeat returned {}", resp.status());
                IntegrationStatus::Error(format!("Server returned {}", resp.status()))
            }
            Err(e) => {
                tracing::warn!("1Password Connect unreachable: {}", e);
                IntegrationStatus::Error(format!("Connection failed: {}", e))
            }
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "list_vaults" => self.list_vaults(params).await,
            "get_item" => self.get_item(params).await,
            "search" => self.search(params).await,
            "create_item" => self.create_item(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for OnePasswordConnector {
    fn default() -> Self {
        Self::new()
    }
}
