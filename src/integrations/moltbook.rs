//! Moltbook Integration
//!
//! Social network integration for AI agents with strict outbound privacy guards.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

static EMAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b").unwrap());
static PHONE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?x)\b(?:\+?\d[\d\-\s().]{7,}\d)\b").unwrap());
static SECRET_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(api[_-]?key|access[_-]?token|bearer|password|private\s*key|ssh\s*key|session\s*token|secret)\b",
    )
    .unwrap()
});
static TOKENISH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(sk-[A-Za-z0-9]{12,}|ghp_[A-Za-z0-9]{12,})\b").unwrap());
static USER_BOUND_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(my|the)\s*(user|client|customer)\b.*\b(name|email|phone|address|account)\b")
        .unwrap()
});

pub struct MoltbookConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl MoltbookConnector {
    const API_BASE: &'static str = "https://www.moltbook.com/api/v1";

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: reqwest::Client::new(),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self::new_with_config_dir(config_dir)
    }

    pub fn has_configured_api_key(&self) -> bool {
        Self::load_api_key_from(&self.config_dir).is_some()
    }

    fn checked_url(path: &str) -> Result<String> {
        let url = format!("{}{}", Self::API_BASE, path);
        let parsed = url::Url::parse(&url)?;
        if parsed.scheme() != "https" {
            return Err(anyhow!("Moltbook URL must be https"));
        }
        if parsed.host_str() != Some("www.moltbook.com") {
            return Err(anyhow!(
                "Refusing non-www Moltbook host (auth header protection)"
            ));
        }
        Ok(url)
    }

    fn load_api_key_from(config_dir: &Path) -> Option<String> {
        if let Ok(key) = std::env::var("MOLTBOOK_API_KEY") {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(m) => m.get_custom_secret("moltbook_api_key").ok().flatten(),
            Err(_) => None,
        }
    }

    fn set_api_key_for(config_dir: &Path, api_key: &str) -> Result<()> {
        let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
        manager.set_custom_secret("moltbook_api_key", Some(api_key.to_string()))?;
        Ok(())
    }

    fn api_key(&self) -> Result<String> {
        Self::load_api_key_from(&self.config_dir)
            .ok_or_else(|| anyhow!("Moltbook API key not configured"))
    }

    fn require_safe_outbound(text: &str) -> Result<()> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Content cannot be empty"));
        }
        if trimmed.len() > 5000 {
            return Err(anyhow!("Content too long"));
        }
        if EMAIL_RE.is_match(trimmed) {
            return Err(anyhow!(
                "Blocked: outbound text appears to contain an email"
            ));
        }
        if PHONE_RE.is_match(trimmed) {
            return Err(anyhow!(
                "Blocked: outbound text appears to contain a phone number"
            ));
        }
        if SECRET_RE.is_match(trimmed) {
            return Err(anyhow!(
                "Blocked: outbound text appears to contain secret-like material"
            ));
        }
        if TOKENISH_RE.is_match(trimmed) {
            return Err(anyhow!(
                "Blocked: outbound text appears to contain token-like material"
            ));
        }
        if USER_BOUND_RE.is_match(trimmed) {
            return Err(anyhow!("Blocked: outbound text appears user-bound"));
        }
        Ok(())
    }

    async fn authed_get(&self, path: &str) -> Result<serde_json::Value> {
        let url = Self::checked_url(path)?;
        let key = self.api_key()?;
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", key))
            .send()
            .await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            return Err(anyhow!("Moltbook API error {}: {}", status, body));
        }
        Ok(body)
    }

    async fn authed_post(
        &self,
        path: &str,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = Self::checked_url(path)?;
        let key = self.api_key()?;
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", key))
            .json(payload)
            .send()
            .await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            return Err(anyhow!("Moltbook API error {}: {}", status, body));
        }
        Ok(body)
    }

    async fn register_agent(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Missing 'name'"))?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("AI agent on Moltbook");
        Self::require_safe_outbound(description)?;

        let url = Self::checked_url("/agents/register")?;
        let payload = serde_json::json!({
            "name": name,
            "description": description
        });
        let resp = self.http.post(url).json(&payload).send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            return Err(anyhow!("Moltbook register failed {}: {}", status, body));
        }

        let api_key = body
            .get("agent")
            .and_then(|a| a.get("api_key"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if api_key.is_empty() {
            return Err(anyhow!("Registration response missing api_key"));
        }
        Self::set_api_key_for(&self.config_dir, &api_key)?;
        let masked = if api_key.len() > 8 {
            format!("{}...{}", &api_key[..4], &api_key[api_key.len() - 4..])
        } else {
            "***".to_string()
        };

        Ok(serde_json::json!({
            "success": true,
            "stored_api_key": true,
            "api_key_masked": masked,
            "claim_url": body.get("agent").and_then(|a| a.get("claim_url")).and_then(|v| v.as_str()),
            "verification_code": body.get("agent").and_then(|a| a.get("verification_code")).and_then(|v| v.as_str()),
        }))
    }

    async fn status_check(&self) -> Result<serde_json::Value> {
        if Self::load_api_key_from(&self.config_dir).is_none() {
            return Ok(serde_json::json!({
                "connected": false,
                "status": "not_configured"
            }));
        }
        match self.authed_get("/agents/status").await {
            Ok(v) => Ok(serde_json::json!({
                "connected": true,
                "status": v.get("status").and_then(|s| s.as_str()).unwrap_or("connected"),
                "raw": v
            })),
            Err(e) => Ok(serde_json::json!({
                "connected": false,
                "status": "error",
                "error": e.to_string()
            })),
        }
    }

    async fn feed(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let sort = params.get("sort").and_then(|v| v.as_str()).unwrap_or("new");
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(25);
        self.authed_get(&format!("/feed?sort={}&limit={}", sort, limit))
            .await
    }

    async fn search(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let q = params
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Missing 'query'"))?;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(25);
        let q_enc = urlencoding::encode(q);
        self.authed_get(&format!("/search?q={}&limit={}", q_enc, limit))
            .await
    }

    async fn create_post(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let submolt = params
            .get("submolt")
            .and_then(|v| v.as_str())
            .unwrap_or("general");
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Missing 'title'"))?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Missing 'content'"))?;
        Self::require_safe_outbound(title)?;
        Self::require_safe_outbound(content)?;

        let payload = serde_json::json!({
            "submolt": submolt,
            "title": title,
            "content": content
        });
        self.authed_post("/posts", &payload).await
    }

    async fn comment(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let post_id = params
            .get("post_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'post_id'"))?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Missing 'content'"))?;
        Self::require_safe_outbound(content)?;
        let payload = serde_json::json!({
            "content": content,
            "parent_id": params.get("parent_id").and_then(|v| v.as_str())
        });
        self.authed_post(&format!("/posts/{}/comments", post_id), &payload)
            .await
    }

    async fn upvote_post(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let post_id = params
            .get("post_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'post_id'"))?;
        self.authed_post(
            &format!("/posts/{}/upvote", post_id),
            &serde_json::json!({}),
        )
        .await
    }
}

#[async_trait]
impl Integration for MoltbookConnector {
    fn id(&self) -> &str {
        "moltbook"
    }

    fn name(&self) -> &str {
        "Moltbook"
    }

    fn description(&self) -> &str {
        "Social network for AI agents (safe mode with outbound privacy guard)"
    }

    fn icon(&self) -> &str {
        "M"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if Self::load_api_key_from(&self.config_dir).is_some() {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "register" => self.register_agent(params).await,
            "status" => self.status_check().await,
            "me" => self.authed_get("/agents/me").await,
            "feed" => self.feed(params).await,
            "search" => self.search(params).await,
            "create_post" => self.create_post(params).await,
            "comment" => self.comment(params).await,
            "upvote_post" => self.upvote_post(params).await,
            _ => Err(anyhow!("Unknown Moltbook action: {}", action)),
        }
    }
}

impl Default for MoltbookConnector {
    fn default() -> Self {
        Self::new()
    }
}
