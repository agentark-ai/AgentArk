//! Mem0 Memory Layer Integration
//!
//! HTTP client for the embedded mem0-bridge. Provides intelligent memory
//! extraction (add), semantic search with decay scoring (search), and
//! periodic cleanup of decayed ephemeral memories.
//!
//! Memory tiers:
//!   - Core facts: persist forever, updated on contradiction (Mem0 built-in)
//!   - Context: exponential decay over time, pruned by cleanup

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::LlmProvider;

/// A memory returned by Mem0 search (with decay metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mem0Memory {
    pub id: String,
    pub memory: String,
    pub score: f32,
    #[serde(default)]
    pub is_core: bool,
    #[serde(default)]
    pub decay: f32,
}

/// Result of a cleanup operation
#[derive(Debug, Clone, Deserialize)]
pub struct CleanupResult {
    pub deleted: usize,
    pub remaining: usize,
    pub core_facts: usize,
}

/// HTTP client for the embedded mem0-bridge
pub struct Mem0Client {
    client: reqwest::Client,
    bridge_url: String,
    configured: AtomicBool,
}

#[derive(Serialize)]
struct ConfigureRequest {
    provider: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

#[derive(Serialize)]
struct AddRequest {
    messages: Vec<MessagePayload>,
    user_id: String,
}

#[derive(Serialize)]
struct MessagePayload {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct SearchRequest {
    query: String,
    user_id: String,
    limit: usize,
}

#[derive(Serialize)]
struct CleanupRequest {
    user_id: String,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    memories: Vec<Mem0Memory>,
}

impl Mem0Client {
    pub fn new(bridge_url: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            bridge_url: bridge_url.trim_end_matches('/').to_string(),
            configured: AtomicBool::new(false),
        }
    }

    /// Whether Mem0 has been configured with an LLM provider
    pub fn is_available(&self) -> bool {
        self.configured.load(Ordering::Relaxed)
    }

    /// Push LLM configuration from model pool to the bridge
    pub async fn configure(&self, provider: &LlmProvider) -> Result<()> {
        let req = match provider {
            LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            } => ConfigureRequest {
                provider: "openai".to_string(),
                model: model.clone(),
                api_key: Some(api_key.clone()),
                base_url: base_url.clone(),
            },
            LlmProvider::Anthropic { api_key, model } => ConfigureRequest {
                provider: "anthropic".to_string(),
                model: model.clone(),
                api_key: Some(api_key.clone()),
                base_url: None,
            },
            LlmProvider::Ollama { base_url, model } => ConfigureRequest {
                provider: "ollama".to_string(),
                model: model.clone(),
                api_key: None,
                base_url: Some(base_url.clone()),
            },
        };

        let resp = self
            .client
            .post(format!("{}/configure", self.bridge_url))
            .json(&req)
            .send()
            .await?;

        if resp.status().is_success() {
            self.configured.store(true, Ordering::Relaxed);
            let display_provider = if req.provider == "openai" {
                if let Some(ref url) = req.base_url {
                    if url.contains("openrouter") {
                        "openrouter"
                    } else {
                        "openai-compatible"
                    }
                } else {
                    "openai"
                }
            } else {
                &req.provider
            };
            tracing::info!(
                "Mem0 configured: provider={}, model={}",
                display_provider,
                req.model
            );
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 configure failed ({}): {}", status, body)
        }
    }

    /// Warm up embedding model + vector stack so first real request is faster.
    /// This is best-effort and should not block normal startup.
    pub async fn warmup(&self) -> Result<()> {
        if !self.is_available() {
            return Ok(());
        }

        let req = SearchRequest {
            query: "startup warmup".to_string(),
            user_id: "system:warmup".to_string(),
            limit: 1,
        };

        let resp = self
            .client
            .post(format!("{}/memories/search", self.bridge_url))
            .json(&req)
            .send()
            .await?;

        if resp.status().is_success() {
            tracing::info!("Mem0 warmup completed");
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 warmup failed ({}): {}", status, body)
        }
    }

    /// Send a user+assistant exchange to Mem0 for intelligent memory extraction
    pub async fn add_memory(
        &self,
        user_msg: &str,
        assistant_msg: &str,
        user_id: &str,
    ) -> Result<()> {
        // Truncate very long messages to avoid overwhelming the LLM fact-extraction step.
        // Code blocks and execution output are not useful for memory anyway.
        const MAX_CHARS: usize = 4000;
        let truncate = |s: &str| -> String {
            if s.len() <= MAX_CHARS {
                return s.to_string();
            }
            // Try to cut before code blocks if possible
            if let Some(pos) = s[..MAX_CHARS].rfind("```") {
                if pos > MAX_CHARS / 2 {
                    return format!("{}...[truncated]", &s[..pos]);
                }
            }
            format!("{}...[truncated]", &s[..MAX_CHARS])
        };

        let req = AddRequest {
            messages: vec![
                MessagePayload {
                    role: "user".to_string(),
                    content: truncate(user_msg),
                },
                MessagePayload {
                    role: "assistant".to_string(),
                    content: truncate(assistant_msg),
                },
            ],
            user_id: user_id.to_string(),
        };

        let resp = self
            .client
            .post(format!("{}/memories", self.bridge_url))
            .json(&req)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 add failed ({}): {}", status, body)
        }
    }

    /// Search memories by semantic similarity (with decay re-ranking)
    pub async fn search(
        &self,
        query: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<Mem0Memory>> {
        let req = SearchRequest {
            query: query.to_string(),
            user_id: user_id.to_string(),
            limit,
        };

        let resp = self
            .client
            .post(format!("{}/memories/search", self.bridge_url))
            .json(&req)
            .send()
            .await?;

        if resp.status().is_success() {
            let search_resp: SearchResponse = resp.json().await?;
            Ok(search_resp.memories)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 search failed ({}): {}", status, body)
        }
    }

    /// Prune decayed ephemeral memories. Core facts are never deleted.
    pub async fn cleanup(&self, user_id: &str) -> Result<CleanupResult> {
        let req = CleanupRequest {
            user_id: user_id.to_string(),
        };

        let resp = self
            .client
            .post(format!("{}/cleanup", self.bridge_url))
            .json(&req)
            .send()
            .await?;

        if resp.status().is_success() {
            let result: CleanupResult = resp.json().await?;
            tracing::info!(
                "Mem0 cleanup: deleted={}, remaining={}, core={}",
                result.deleted,
                result.remaining,
                result.core_facts
            );
            Ok(result)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 cleanup failed ({}): {}", status, body)
        }
    }
}
