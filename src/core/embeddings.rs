use anyhow::{anyhow, Result};
use futures::StreamExt;
use sea_orm::entity::prelude::PgVector;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use super::config::{AgentConfig, EmbeddingsProviderKind};
use super::llm_provider::effective_openai_base_url;

const MAX_EMBED_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PROVIDER_EMBED_BATCH_TEXTS: usize = 8;
const MAX_PROVIDER_EMBED_BATCH_TEXTS: usize = 64;
const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "BAAI/bge-small-en-v1.5";
const DEFAULT_LOCAL_EMBEDDINGS_DOCKER_URL: &str = "http://agentark-embeddings:8993";
const DEFAULT_LOCAL_EMBEDDINGS_LOCAL_URL: &str = "http://127.0.0.1:8993";
const DEFAULT_OPENAI_EMBEDDINGS_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Clone)]
enum EmbeddingProvider {
    LocalHf {
        base_url: String,
    },
    OpenAICompatible {
        api_key: String,
        base_url: Option<String>,
    },
    Ollama {
        base_url: String,
    },
}

#[derive(Clone)]
pub struct EmbeddingClient {
    provider: EmbeddingProvider,
    model: String,
    client: reqwest::Client,
    prepared: Arc<AtomicBool>,
    prepare_lock: Arc<tokio::sync::Mutex<()>>,
}

fn effective_embeddings_base_url(base_url: Option<&str>) -> &str {
    match base_url.map(str::trim).filter(|value| !value.is_empty()) {
        Some(url) => effective_openai_base_url(Some(url)),
        None => DEFAULT_OPENAI_EMBEDDINGS_BASE_URL,
    }
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn provider_embed_batch_texts() -> usize {
    std::env::var("AGENTARK_EMBEDDINGS_BATCH_TEXTS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_PROVIDER_EMBED_BATCH_TEXTS)
        .clamp(1, MAX_PROVIDER_EMBED_BATCH_TEXTS)
}

fn local_embeddings_base_url() -> String {
    std::env::var("AGENTARK_LOCAL_EMBEDDINGS_URL")
        .ok()
        .map(|value| normalize_base_url(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if Path::new("/.dockerenv").exists() {
                DEFAULT_LOCAL_EMBEDDINGS_DOCKER_URL.to_string()
            } else {
                DEFAULT_LOCAL_EMBEDDINGS_LOCAL_URL.to_string()
            }
        })
}

fn normalize_embedding(values: Vec<f32>) -> Result<PgVector> {
    if values.is_empty() {
        return Err(anyhow!("embedding vector was empty"));
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err(anyhow!("embedding vector had zero norm"));
    }
    Ok(PgVector::from(
        values
            .into_iter()
            .map(|value| value / norm)
            .collect::<Vec<_>>(),
    ))
}

async fn read_response_bytes_limited(
    response: reqwest::Response,
    provider: &str,
) -> Result<Vec<u8>> {
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_EMBED_RESPONSE_BYTES as u64 {
            return Err(anyhow!(
                "{} response exceeded {} byte limit (content-length={})",
                provider,
                MAX_EMBED_RESPONSE_BYTES,
                content_length
            ));
        }
    }

    let mut total = 0usize;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total = total.saturating_add(chunk.len());
        if total > MAX_EMBED_RESPONSE_BYTES {
            return Err(anyhow!(
                "{} response exceeded {} byte limit",
                provider,
                MAX_EMBED_RESPONSE_BYTES
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn read_response_text_limited(response: reqwest::Response, provider: &str) -> Result<String> {
    let bytes = read_response_bytes_limited(response, provider).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn read_response_json_limited<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    provider: &str,
) -> Result<T> {
    let bytes = read_response_bytes_limited(response, provider).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

impl EmbeddingProvider {
    fn provider_name(&self) -> &'static str {
        match self {
            Self::LocalHf { .. } => "local-hf",
            Self::OpenAICompatible { base_url, .. }
                if base_url.as_deref().unwrap_or("").trim().is_empty() =>
            {
                "openai"
            }
            Self::OpenAICompatible { .. } => "openai-compatible",
            Self::Ollama { .. } => "ollama",
        }
    }
}

impl EmbeddingClient {
    pub fn from_config(config: &AgentConfig, _data_dir: &Path) -> Result<Option<Self>> {
        let embeddings = config.embeddings_config();
        let model = if embeddings.model.trim().is_empty() {
            DEFAULT_LOCAL_EMBEDDING_MODEL.to_string()
        } else {
            embeddings.model.trim().to_string()
        };

        let provider = match embeddings.provider {
            EmbeddingsProviderKind::Disabled => return Ok(None),
            EmbeddingsProviderKind::LocalHf => EmbeddingProvider::LocalHf {
                base_url: local_embeddings_base_url(),
            },
            EmbeddingsProviderKind::Ollama => {
                let Some(base_url) = embeddings
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(normalize_base_url)
                else {
                    return Ok(None);
                };
                EmbeddingProvider::Ollama { base_url }
            }
            EmbeddingsProviderKind::OpenaiCompatible => EmbeddingProvider::OpenAICompatible {
                api_key: embeddings.api_key.clone(),
                base_url: embeddings
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(normalize_base_url),
            },
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;

        Ok(Some(Self {
            provider,
            model,
            client,
            prepared: Arc::new(AtomicBool::new(false)),
            prepare_lock: Arc::new(tokio::sync::Mutex::new(())),
        }))
    }

    pub fn provider_name(&self) -> &'static str {
        self.provider.provider_name()
    }

    pub fn describe_backend(&self) -> String {
        match &self.provider {
            EmbeddingProvider::LocalHf { base_url } => {
                format!("{} ({}) @ {}", self.provider_name(), self.model, base_url)
            }
            EmbeddingProvider::OpenAICompatible { base_url, .. } => format!(
                "{} ({}) @ {}",
                self.provider_name(),
                self.model,
                effective_embeddings_base_url(base_url.as_deref())
            ),
            EmbeddingProvider::Ollama { base_url } => {
                format!("{} ({}) @ {}", self.provider_name(), self.model, base_url)
            }
        }
    }

    pub async fn prepare(&self) -> Result<()> {
        if self.prepared.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _guard = self.prepare_lock.lock().await;
        if self.prepared.load(Ordering::Relaxed) {
            return Ok(());
        }

        self.prepared.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub async fn health_check(&self) -> Result<String> {
        match &self.provider {
            EmbeddingProvider::LocalHf { base_url } => {
                #[derive(Deserialize, Default)]
                struct LocalHfHealthResponse {
                    #[serde(default)]
                    ok: bool,
                    #[serde(default)]
                    ready: bool,
                    #[serde(default)]
                    model: Option<String>,
                    #[serde(default)]
                    error: Option<String>,
                }

                let response = self
                    .client
                    .get(format!("{}/health", base_url.trim_end_matches('/')))
                    .send()
                    .await?;
                if !response.status().is_success() {
                    let error =
                        read_response_text_limited(response, "Local embeddings health").await?;
                    return Err(anyhow!("local embeddings sidecar health error: {}", error));
                }

                let health: LocalHfHealthResponse =
                    read_response_json_limited(response, "Local embeddings health").await?;
                if !health.ok {
                    return Err(anyhow!(
                        "local embeddings sidecar unavailable: {}",
                        health.error.unwrap_or_else(|| "unknown error".to_string())
                    ));
                }

                let model = health.model.as_deref().unwrap_or(self.model.as_str());
                if health.ready {
                    Ok(format!("Local embeddings sidecar ready ({})", model))
                } else {
                    Ok(format!(
                        "Local embeddings sidecar reachable ({}) and initializes on first dense retrieval use",
                        model
                    ))
                }
            }
            EmbeddingProvider::OpenAICompatible { base_url, .. } => Ok(format!(
                "External embeddings configured ({}) @ {}",
                self.model,
                effective_embeddings_base_url(base_url.as_deref())
            )),
            EmbeddingProvider::Ollama { base_url } => {
                let tags = self.fetch_ollama_tags(base_url).await?;
                let model_ready = tags.iter().any(|name| {
                    name.trim()
                        .trim_end_matches(":latest")
                        .eq_ignore_ascii_case(self.model.trim().trim_end_matches(":latest"))
                });
                Ok(if model_ready {
                    format!("External Ollama embeddings ready ({})", self.model)
                } else {
                    format!(
                        "External Ollama reachable, but model is not loaded yet ({})",
                        self.model
                    )
                })
            }
        }
    }

    pub async fn embed_texts(&self, texts: &[String]) -> Result<Vec<PgVector>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        self.prepare().await?;

        let mut embeddings = Vec::with_capacity(texts.len());
        let batch_texts = provider_embed_batch_texts();
        for chunk in texts.chunks(batch_texts) {
            let chunk_embeddings = match &self.provider {
                EmbeddingProvider::LocalHf { base_url } => {
                    self.embed_local_hf(base_url, chunk).await
                }
                EmbeddingProvider::OpenAICompatible { api_key, base_url } => {
                    self.embed_openai(api_key, base_url.as_deref(), chunk).await
                }
                EmbeddingProvider::Ollama { base_url } => self.embed_ollama(base_url, chunk).await,
            }?;
            embeddings.extend(chunk_embeddings);
        }
        Ok(embeddings)
    }

    async fn fetch_ollama_tags(&self, base_url: &str) -> Result<Vec<String>> {
        #[derive(Deserialize, Default)]
        struct OllamaTagEntry {
            #[serde(default)]
            name: String,
            #[serde(default)]
            model: String,
        }

        #[derive(Deserialize, Default)]
        struct OllamaTagsResponse {
            #[serde(default)]
            models: Vec<OllamaTagEntry>,
        }

        let response = self
            .client
            .get(format!("{}/api/tags", base_url.trim_end_matches('/')))
            .send()
            .await?;
        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Ollama tags").await?;
            return Err(anyhow!("ollama tags error: {}", error));
        }

        let response: OllamaTagsResponse =
            read_response_json_limited(response, "Ollama tags").await?;
        Ok(response
            .models
            .into_iter()
            .map(|entry| {
                if entry.name.trim().is_empty() {
                    entry.model
                } else {
                    entry.name
                }
            })
            .filter(|name| !name.trim().is_empty())
            .collect())
    }

    async fn embed_local_hf(&self, base_url: &str, texts: &[String]) -> Result<Vec<PgVector>> {
        #[derive(Serialize)]
        struct LocalHfEmbedRequest<'a> {
            model: &'a str,
            texts: &'a [String],
        }

        #[derive(Deserialize)]
        struct LocalHfEmbedResponse {
            embeddings: Vec<Vec<f32>>,
        }

        let response = self
            .client
            .post(format!("{}/embed", base_url.trim_end_matches('/')))
            .json(&LocalHfEmbedRequest {
                model: &self.model,
                texts,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Local embeddings").await?;
            return Err(anyhow!("local embeddings sidecar error: {}", error));
        }

        let response: LocalHfEmbedResponse =
            read_response_json_limited(response, "Local embeddings").await?;
        if response.embeddings.len() != texts.len() {
            return Err(anyhow!(
                "local embedding response size mismatch: expected {}, got {}",
                texts.len(),
                response.embeddings.len()
            ));
        }

        response
            .embeddings
            .into_iter()
            .map(normalize_embedding)
            .collect::<Result<Vec<_>>>()
    }

    async fn embed_openai(
        &self,
        api_key: &str,
        base_url: Option<&str>,
        texts: &[String],
    ) -> Result<Vec<PgVector>> {
        #[derive(Serialize)]
        struct EmbeddingRequest<'a> {
            model: &'a str,
            input: &'a [String],
            encoding_format: &'static str,
        }

        #[derive(Deserialize)]
        struct EmbeddingItem {
            embedding: Vec<f32>,
        }

        #[derive(Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingItem>,
        }

        let mut request = self
            .client
            .post(format!(
                "{}/embeddings",
                effective_embeddings_base_url(base_url).trim_end_matches('/')
            ))
            .json(&EmbeddingRequest {
                model: &self.model,
                input: texts,
                encoding_format: "float",
            });

        if !api_key.trim().is_empty() {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Embedding API").await?;
            return Err(anyhow!("embedding API error: {}", error));
        }

        let response: EmbeddingResponse =
            read_response_json_limited(response, "Embedding API").await?;
        if response.data.len() != texts.len() {
            return Err(anyhow!(
                "embedding response size mismatch: expected {}, got {}",
                texts.len(),
                response.data.len()
            ));
        }

        response
            .data
            .into_iter()
            .map(|item| normalize_embedding(item.embedding))
            .collect()
    }

    async fn embed_ollama(&self, base_url: &str, texts: &[String]) -> Result<Vec<PgVector>> {
        #[derive(Serialize)]
        struct OllamaEmbedRequest<'a> {
            model: &'a str,
            input: &'a [String],
        }

        #[derive(Deserialize)]
        struct OllamaEmbedResponse {
            #[serde(default)]
            embeddings: Vec<Vec<f32>>,
            #[serde(default)]
            embedding: Option<Vec<f32>>,
        }

        let response = self
            .client
            .post(format!("{}/api/embed", base_url.trim_end_matches('/')))
            .json(&OllamaEmbedRequest {
                model: &self.model,
                input: texts,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Ollama embeddings").await?;
            return Err(anyhow!("ollama embeddings error: {}", error));
        }

        let response: OllamaEmbedResponse =
            read_response_json_limited(response, "Ollama embeddings").await?;

        let embeddings = if !response.embeddings.is_empty() {
            response.embeddings
        } else if let Some(single) = response.embedding {
            vec![single]
        } else {
            Vec::new()
        };

        if embeddings.len() != texts.len() {
            return Err(anyhow!(
                "ollama embedding response size mismatch: expected {}, got {}",
                texts.len(),
                embeddings.len()
            ));
        }

        embeddings
            .into_iter()
            .map(normalize_embedding)
            .collect::<Result<Vec<_>>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_embedding_rejects_empty_vectors() {
        assert!(normalize_embedding(Vec::new()).is_err());
    }
}
