//! Twitter/X Integration
//!
//! Provides access to the Twitter/X API v2 for reading tweets, searching,
//! viewing bookmarks, and retrieving user profile information.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Twitter/X API connector
pub struct TwitterConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl TwitterConnector {
    const API_BASE: &'static str = "https://api.twitter.com/2";

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: crate::core::net::default_outgoing_http_client(),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_config_dir(config_dir)
    }

    /// Load bearer token from environment variable or secure config
    fn load_token_from(config_dir: &Path) -> Option<String> {
        if let Ok(token) = std::env::var("TWITTER_BEARER_TOKEN") {
            if !token.is_empty() {
                return Some(token);
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("twitter_bearer_token")
                .ok()
                .flatten(),
            Err(_) => None,
        }
    }

    /// Get the bearer token or return an error
    fn token(&self) -> Result<String> {
        Self::load_token_from(&self.config_dir).ok_or_else(|| {
            anyhow!("Twitter bearer token not configured. Set TWITTER_BEARER_TOKEN or store via secure config.")
        })
    }

    /// GET /users/me - Retrieve the authenticated user's profile
    async fn get_user(&self, _params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        let url = format!(
            "{}/users/me?user.fields=id,name,username,description,public_metrics",
            Self::API_BASE
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
            tracing::warn!("Twitter get_user failed ({}): {}", status, error_text);
            return Err(anyhow!("Twitter API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;
        let data = body.get("data").cloned().unwrap_or(serde_json::Value::Null);

        Ok(serde_json::json!({
            "id": data.get("id"),
            "name": data.get("name"),
            "username": data.get("username"),
            "description": data.get("description"),
            "public_metrics": data.get("public_metrics"),
        }))
    }

    /// GET /users/{user_id}/bookmarks - Retrieve the authenticated user's bookmarks
    async fn bookmarks(&self, _params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        // First retrieve the authenticated user's ID
        let user = self.get_user(&serde_json::json!({})).await?;
        let user_id = user
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Could not determine authenticated user ID"))?;

        let url = format!(
            "{}/users/{}/bookmarks?tweet.fields=created_at,author_id,text&max_results=20",
            Self::API_BASE,
            user_id
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
            tracing::warn!("Twitter bookmarks failed ({}): {}", status, error_text);
            return Err(anyhow!("Twitter API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;
        let tweets = body
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let bookmarks: Vec<serde_json::Value> = tweets
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.get("id"),
                    "text": t.get("text"),
                    "author_id": t.get("author_id"),
                    "created_at": t.get("created_at"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "bookmarks": bookmarks,
            "count": bookmarks.len(),
        }))
    }

    /// GET /users/{user_id}/tweets - List tweets by the authenticated user
    async fn list_tweets(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        // Allow an explicit user_id, otherwise fetch the authenticated user
        let user_id = if let Some(uid) = params.get("user_id").and_then(|v| v.as_str()) {
            uid.to_string()
        } else {
            let user = self.get_user(&serde_json::json!({})).await?;
            user.get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Could not determine user ID"))?
                .to_string()
        };

        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(100);

        let url = format!(
            "{}/users/{}/tweets?tweet.fields=created_at,text,public_metrics&max_results={}",
            Self::API_BASE,
            user_id,
            max_results
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
            tracing::warn!("Twitter list_tweets failed ({}): {}", status, error_text);
            return Err(anyhow!("Twitter API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;
        let tweets = body
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let results: Vec<serde_json::Value> = tweets
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.get("id"),
                    "text": t.get("text"),
                    "created_at": t.get("created_at"),
                    "public_metrics": t.get("public_metrics"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "tweets": results,
            "count": results.len(),
        }))
    }

    /// GET /tweets/search/recent - Search recent tweets
    async fn search(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let token = self.token()?;

        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'query' parameter"))?;

        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(100);

        let url = format!(
            "{}/tweets/search/recent?query={}&tweet.fields=created_at,author_id,text,public_metrics&max_results={}",
            Self::API_BASE,
            urlencoding::encode(query),
            max_results
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
            tracing::warn!("Twitter search failed ({}): {}", status, error_text);
            return Err(anyhow!("Twitter API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response.json().await?;
        let tweets = body
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let results: Vec<serde_json::Value> = tweets
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.get("id"),
                    "text": t.get("text"),
                    "author_id": t.get("author_id"),
                    "created_at": t.get("created_at"),
                    "public_metrics": t.get("public_metrics"),
                })
            })
            .collect();

        let meta = body.get("meta").cloned().unwrap_or(serde_json::Value::Null);

        Ok(serde_json::json!({
            "tweets": results,
            "count": results.len(),
            "meta": meta,
        }))
    }
}

#[async_trait]
impl Integration for TwitterConnector {
    fn id(&self) -> &str {
        "twitter"
    }

    fn name(&self) -> &str {
        "Twitter / X"
    }

    fn description(&self) -> &str {
        "Access Twitter/X - search tweets, view bookmarks, list user tweets, and profile info"
    }

    fn icon(&self) -> &str {
        "🐦"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if Self::load_token_from(&self.config_dir).is_some() {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "get_user" => self.get_user(params).await,
            "bookmarks" => self.bookmarks(params).await,
            "list_tweets" => self.list_tweets(params).await,
            "search" => self.search(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for TwitterConnector {
    fn default() -> Self {
        Self::new()
    }
}
