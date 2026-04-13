use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use sea_orm::entity::prelude::PgVector;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[cfg(not(test))]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
#[cfg(not(test))]
use std::str::FromStr;

use super::config::{AgentConfig, EmbeddingsProviderKind};
use super::llm_provider::effective_openai_base_url;

const MAX_EMBED_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "BAAI/bge-small-en-v1.5";
const DEFAULT_OPENAI_EMBEDDINGS_BASE_URL: &str = "https://api.openai.com/v1";
const LOCAL_EMBEDDING_RETRY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug)]
enum LocalEmbeddingStatus {
    Idle,
    Preparing,
    Ready,
    Failed {
        error: String,
        retry_after: Instant,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LocalEmbeddingModel {
    AllMiniLML6V2,
    AllMiniLML12V2,
    BGESmallENV15,
    NomicEmbedTextV15,
    #[cfg(not(test))]
    Custom(String),
}

#[cfg(not(test))]
type LocalEmbeddingRuntime = TextEmbedding;

#[cfg(test)]
#[derive(Clone, Default)]
struct LocalEmbeddingRuntime;

#[derive(Clone)]
enum EmbeddingProvider {
    LocalHf {
        cache_dir: PathBuf,
        runtime: Arc<Mutex<Option<LocalEmbeddingRuntime>>>,
        status: Arc<Mutex<LocalEmbeddingStatus>>,
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

fn normalize_local_model_alias(value: &str) -> Option<LocalEmbeddingModel> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "sentence-transformers/all-minilm-l6-v2" | "all-minilm-l6-v2" => {
            Some(LocalEmbeddingModel::AllMiniLML6V2)
        }
        "sentence-transformers/all-minilm-l12-v2" | "all-minilm-l12-v2" => {
            Some(LocalEmbeddingModel::AllMiniLML12V2)
        }
        "baai/bge-small-en-v1.5" | "bge-small-en-v1.5" => Some(LocalEmbeddingModel::BGESmallENV15),
        "nomic-ai/nomic-embed-text-v1.5" | "nomic-embed-text-v1.5" | "nomic-embed-text" => {
            Some(LocalEmbeddingModel::NomicEmbedTextV15)
        }
        _ => {
            let compact = normalized
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>();
            match compact.as_str() {
                "allminilml6v2" => Some(LocalEmbeddingModel::AllMiniLML6V2),
                "allminilml12v2" => Some(LocalEmbeddingModel::AllMiniLML12V2),
                "bgesmallenv15" => Some(LocalEmbeddingModel::BGESmallENV15),
                "nomicembedtextv15" | "nomicembedtext" => {
                    Some(LocalEmbeddingModel::NomicEmbedTextV15)
                }
                _ => None,
            }
        }
    }
}

fn resolve_local_embedding_model(value: &str) -> Result<LocalEmbeddingModel> {
    if let Some(model) = normalize_local_model_alias(value) {
        return Ok(model);
    }

    #[cfg(not(test))]
    {
        return EmbeddingModel::from_str(value.trim())
            .map(|_| LocalEmbeddingModel::Custom(value.trim().to_string()))
            .map_err(|_| {
                anyhow!(
                    "Unsupported local embeddings model '{}'. Try {} or BAAI/bge-small-en-v1.5.",
                    value.trim(),
                    DEFAULT_LOCAL_EMBEDDING_MODEL
                )
            });
    }

    #[cfg(test)]
    {
        return Err(anyhow!(
            "Unsupported local embeddings model '{}'. Try {} or BAAI/bge-small-en-v1.5.",
            value.trim(),
            DEFAULT_LOCAL_EMBEDDING_MODEL
        ));
    }
}

#[cfg(not(test))]
fn to_fastembed_model(model: LocalEmbeddingModel) -> EmbeddingModel {
    match model {
        LocalEmbeddingModel::AllMiniLML6V2 => EmbeddingModel::AllMiniLML6V2,
        LocalEmbeddingModel::AllMiniLML12V2 => EmbeddingModel::AllMiniLML12V2,
        LocalEmbeddingModel::BGESmallENV15 => EmbeddingModel::BGESmallENV15,
        LocalEmbeddingModel::NomicEmbedTextV15 => EmbeddingModel::NomicEmbedTextV15,
        #[cfg(not(test))]
        LocalEmbeddingModel::Custom(name) => {
            EmbeddingModel::from_str(&name).expect("validated custom local embedding model")
        }
    }
}

#[cfg(not(test))]
fn initialize_local_runtime(
    cache_dir: &Path,
    model: LocalEmbeddingModel,
) -> Result<LocalEmbeddingRuntime> {
    let options = InitOptions::new(to_fastembed_model(model))
        .with_cache_dir(cache_dir.to_path_buf())
        .with_show_download_progress(true);
    Ok(TextEmbedding::try_new(options)?)
}

#[cfg(test)]
fn initialize_local_runtime(
    _cache_dir: &Path,
    _model: LocalEmbeddingModel,
) -> Result<LocalEmbeddingRuntime> {
    Ok(LocalEmbeddingRuntime)
}

#[cfg(not(test))]
fn warmup_local_runtime(runtime: &mut LocalEmbeddingRuntime) -> Result<()> {
    let warmup = vec!["agentark embedding warmup"];
    let _ = runtime.embed(warmup, None)?;
    Ok(())
}

#[cfg(test)]
fn warmup_local_runtime(_runtime: &mut LocalEmbeddingRuntime) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn embed_with_local_runtime(
    runtime: &mut LocalEmbeddingRuntime,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>> {
    Ok(runtime.embed(texts, None)?)
}

#[cfg(test)]
fn embed_with_local_runtime(
    _runtime: &mut LocalEmbeddingRuntime,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>> {
    Ok(texts
        .into_iter()
        .map(|text| deterministic_test_embedding(&text))
        .collect())
}

#[cfg(test)]
fn deterministic_test_embedding(text: &str) -> Vec<f32> {
    let mut values = vec![0.0f32; 16];
    if text.trim().is_empty() {
        values[0] = 1.0;
        return values;
    }

    for (index, byte) in text.bytes().enumerate() {
        let slot = index % values.len();
        let weight = 1.0 + (index % 5) as f32 * 0.1;
        values[slot] += ((byte as f32) / 255.0 + 0.01) * weight;
    }

    if values.iter().all(|value| value.abs() <= f32::EPSILON) {
        values[0] = 1.0;
    }

    values
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

fn local_embeddings_cache_dir(data_dir: &Path) -> PathBuf {
    std::env::var_os("AGENTARK_LOCAL_EMBEDDINGS_CACHE_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            let bundled = PathBuf::from("/app/prebuilt-embeddings-cache");
            bundled.exists().then_some(bundled)
        })
        .unwrap_or_else(|| data_dir.join("embeddings-cache"))
}

impl EmbeddingClient {
    pub fn from_config(config: &AgentConfig, data_dir: &Path) -> Result<Option<Self>> {
        let embeddings = config.embeddings_config();
        let model = if embeddings.model.trim().is_empty() {
            DEFAULT_LOCAL_EMBEDDING_MODEL.to_string()
        } else {
            embeddings.model.trim().to_string()
        };

        let provider = match embeddings.provider {
            EmbeddingsProviderKind::LocalHf => EmbeddingProvider::LocalHf {
                cache_dir: local_embeddings_cache_dir(data_dir),
                runtime: Arc::new(Mutex::new(None)),
                status: Arc::new(Mutex::new(LocalEmbeddingStatus::Idle)),
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
            EmbeddingProvider::LocalHf { .. } => {
                format!("{} ({})", self.provider_name(), self.model)
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

    fn set_local_status(status: &Arc<Mutex<LocalEmbeddingStatus>>, next: LocalEmbeddingStatus) {
        if let Ok(mut guard) = status.lock() {
            *guard = next;
        }
    }

    async fn prepare_local_model(
        &self,
        cache_dir: PathBuf,
        runtime: Arc<Mutex<Option<LocalEmbeddingRuntime>>>,
        status: Arc<Mutex<LocalEmbeddingStatus>>,
    ) -> Result<()> {
        Self::set_local_status(&status, LocalEmbeddingStatus::Preparing);
        let model_name = self.model.clone();
        let outcome = tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&cache_dir).with_context(|| {
                format!("failed to create embeddings cache dir {:?}", cache_dir)
            })?;
            let model = resolve_local_embedding_model(&model_name)?;
            let mut guard = runtime
                .lock()
                .map_err(|_| anyhow!("local embeddings runtime lock poisoned"))?;
            if guard.is_none() {
                *guard = Some(initialize_local_runtime(&cache_dir, model)?);
            }
            let runtime = guard
                .as_mut()
                .ok_or_else(|| anyhow!("local embeddings runtime was not initialized"))?;
            warmup_local_runtime(runtime)?;
            Ok(())
        })
        .await
        .map_err(|error| anyhow!("local embeddings worker failed: {}", error))?;

        match outcome {
            Ok(()) => {
                Self::set_local_status(&status, LocalEmbeddingStatus::Ready);
                Ok(())
            }
            Err(error) => {
                Self::set_local_status(
                    &status,
                    LocalEmbeddingStatus::Failed {
                        error: error.to_string(),
                        retry_after: Instant::now() + LOCAL_EMBEDDING_RETRY_INTERVAL,
                    },
                );
                Err(error)
            }
        }
    }

    fn local_retry_delay(status: &Arc<Mutex<LocalEmbeddingStatus>>) -> Result<Option<Duration>> {
        let status = status
            .lock()
            .map_err(|_| anyhow!("local embeddings status lock poisoned"))?
            .clone();
        let LocalEmbeddingStatus::Failed { retry_after, .. } = status else {
            return Ok(None);
        };
        Ok(Some(retry_after.saturating_duration_since(Instant::now())))
    }

    pub async fn prepare(&self) -> Result<()> {
        if self.prepared.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _guard = self.prepare_lock.lock().await;
        if self.prepared.load(Ordering::Relaxed) {
            return Ok(());
        }

        match &self.provider {
            EmbeddingProvider::LocalHf {
                cache_dir,
                runtime,
                status,
            } => {
                if let Some(delay) = Self::local_retry_delay(status)? {
                    if delay > Duration::ZERO {
                        return Err(anyhow!(
                            "local embeddings are unavailable; next download retry in about {} minute(s)",
                            delay.as_secs().div_ceil(60)
                        ));
                    }
                    Self::set_local_status(status, LocalEmbeddingStatus::Idle);
                }
                self.prepare_local_model(
                    cache_dir.clone(),
                    Arc::clone(runtime),
                    Arc::clone(status),
                )
                .await?;
            }
            EmbeddingProvider::OpenAICompatible { .. } | EmbeddingProvider::Ollama { .. } => {}
        }

        self.prepared.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub async fn health_check(&self) -> Result<String> {
        match &self.provider {
            EmbeddingProvider::LocalHf { status, .. } => {
                let status = status
                    .lock()
                    .map_err(|_| anyhow!("local embeddings status lock poisoned"))?
                    .clone();
                match status {
                    LocalEmbeddingStatus::Idle => Ok(format!(
                        "Local embeddings configured ({}) and will initialize in the background after startup",
                        self.model
                    )),
                    LocalEmbeddingStatus::Preparing => Ok(format!(
                        "Local embeddings downloading or initializing ({})",
                        self.model
                    )),
                    LocalEmbeddingStatus::Ready => {
                        Ok(format!("Local embeddings ready ({})", self.model))
                    }
                    LocalEmbeddingStatus::Failed { error, retry_after } => Err(anyhow!(
                        "Local embeddings unavailable: {}; next download retry in about {} minute(s)",
                        error,
                        retry_after
                            .saturating_duration_since(Instant::now())
                            .as_secs()
                            .div_ceil(60)
                    )),
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

        match &self.provider {
            EmbeddingProvider::LocalHf { runtime, .. } => {
                self.embed_local_hf(Arc::clone(runtime), texts).await
            }
            EmbeddingProvider::OpenAICompatible { api_key, base_url } => {
                self.embed_openai(api_key, base_url.as_deref(), texts).await
            }
            EmbeddingProvider::Ollama { base_url } => self.embed_ollama(base_url, texts).await,
        }
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

    async fn embed_local_hf(
        &self,
        runtime: Arc<Mutex<Option<LocalEmbeddingRuntime>>>,
        texts: &[String],
    ) -> Result<Vec<PgVector>> {
        let owned_texts = texts.to_vec();
        tokio::task::spawn_blocking(move || -> Result<Vec<PgVector>> {
            let mut guard = runtime
                .lock()
                .map_err(|_| anyhow!("local embeddings runtime lock poisoned"))?;
            let runtime = guard
                .as_mut()
                .ok_or_else(|| anyhow!("local embeddings runtime is not initialized"))?;
            embed_with_local_runtime(runtime, owned_texts)?
                .into_iter()
                .map(normalize_embedding)
                .collect()
        })
        .await
        .map_err(|error| anyhow!("local embeddings worker failed: {}", error))?
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
    fn local_embedding_aliases_resolve() {
        assert_eq!(
            resolve_local_embedding_model("sentence-transformers/all-MiniLM-L6-v2").unwrap(),
            LocalEmbeddingModel::AllMiniLML6V2
        );
        assert_eq!(
            resolve_local_embedding_model("BAAI/bge-small-en-v1.5").unwrap(),
            LocalEmbeddingModel::BGESmallENV15
        );
        assert_eq!(
            resolve_local_embedding_model("bge-small-en-v1.5").unwrap(),
            LocalEmbeddingModel::BGESmallENV15
        );
        assert_eq!(
            resolve_local_embedding_model("AllMiniLML6V2").unwrap(),
            LocalEmbeddingModel::AllMiniLML6V2
        );
    }

    #[test]
    fn normalize_embedding_rejects_empty_vectors() {
        assert!(normalize_embedding(Vec::new()).is_err());
    }
}
