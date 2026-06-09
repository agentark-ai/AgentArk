#[cfg(target_os = "windows")]
fn main() {
    eprintln!("agentark_embed_server is built for the Docker/Linux embeddings sidecar.");
}

#[cfg(not(target_os = "windows"))]
mod non_windows {
    use anyhow::{anyhow, Context, Result};
    use axum::{
        extract::{rejection::JsonRejection, State},
        http::{Request, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use serde::{Deserialize, Serialize};
    use std::{
        path::{Path, PathBuf},
        str::FromStr,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Mutex,
        },
        time::Instant,
    };
    use tower_http::trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer};
    use tracing::Level;

    const DEFAULT_BIND: &str = "0.0.0.0:8993";
    const DEFAULT_CACHE_DIR: &str = "/app/prebuilt-embeddings-cache";
    const DEFAULT_MODEL: &str = "BAAI/bge-small-en-v1.5";
    const MAX_TEXTS_PER_REQUEST: usize = 64;

    #[derive(Clone)]
    struct AppState {
        cache_dir: PathBuf,
        default_model: String,
        standard_runtime: Arc<Mutex<RuntimeState>>,
        hot_runtime: Arc<Mutex<RuntimeState>>,
        request_counter: Arc<AtomicU64>,
    }

    #[derive(Default)]
    struct RuntimeState {
        model_name: Option<String>,
        model: Option<TextEmbedding>,
        last_error: Option<String>,
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum EmbedPriority {
        #[default]
        Standard,
        HotPath,
    }

    impl EmbedPriority {
        fn lane_name(self) -> &'static str {
            match self {
                Self::Standard => "standard",
                Self::HotPath => "hot_path",
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum LocalEmbeddingModel {
        AllMiniLML6V2,
        AllMiniLML12V2,
        BGESmallENV15,
        NomicEmbedTextV15,
        Custom(String),
    }

    #[derive(Deserialize)]
    struct EmbedRequest {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        texts: Vec<String>,
        #[serde(default)]
        priority: EmbedPriority,
    }

    #[derive(Serialize)]
    struct EmbedResponse {
        embeddings: Vec<Vec<f32>>,
    }

    #[derive(Serialize)]
    struct HealthResponse {
        ok: bool,
        ready: bool,
        standard_ready: bool,
        hot_ready: bool,
        busy: bool,
        model: Option<String>,
        error: Option<String>,
    }

    #[derive(Serialize)]
    struct ErrorResponse {
        error: String,
    }

    #[derive(Clone, Copy)]
    struct EmbedRequestLogMetrics {
        text_count: usize,
        empty_text_count: usize,
        total_text_chars: usize,
        max_text_chars: usize,
        total_text_bytes: usize,
    }

    fn embed_request_log_metrics(texts: &[String]) -> EmbedRequestLogMetrics {
        let mut empty_text_count = 0usize;
        let mut total_text_chars = 0usize;
        let mut max_text_chars = 0usize;
        let mut total_text_bytes = 0usize;

        for text in texts {
            if text.is_empty() {
                empty_text_count += 1;
            }
            let char_count = text.chars().count();
            total_text_chars = total_text_chars.saturating_add(char_count);
            max_text_chars = max_text_chars.max(char_count);
            total_text_bytes = total_text_bytes.saturating_add(text.len());
        }

        EmbedRequestLogMetrics {
            text_count: texts.len(),
            empty_text_count,
            total_text_chars,
            max_text_chars,
            total_text_bytes,
        }
    }

    fn json_error(status: StatusCode, error: impl Into<String>) -> Response {
        (
            status,
            Json(ErrorResponse {
                error: error.into(),
            }),
        )
            .into_response()
    }

    fn configured_bind() -> String {
        std::env::var("AGENTARK_EMBEDDINGS_BIND")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BIND.to_string())
    }

    fn configured_cache_dir() -> PathBuf {
        std::env::var_os("AGENTARK_LOCAL_EMBEDDINGS_CACHE_DIR")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CACHE_DIR))
    }

    fn configured_model() -> String {
        std::env::var("AGENTARK_LOCAL_EMBEDDINGS_MODEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string())
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
            "baai/bge-small-en-v1.5" | "bge-small-en-v1.5" => {
                Some(LocalEmbeddingModel::BGESmallENV15)
            }
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

        EmbeddingModel::from_str(value.trim())
            .map(|_| LocalEmbeddingModel::Custom(value.trim().to_string()))
            .map_err(|_| {
                anyhow!(
                    "Unsupported local embeddings model '{}'. Try {} or BAAI/bge-small-en-v1.5.",
                    value.trim(),
                    DEFAULT_MODEL
                )
            })
    }

    fn to_fastembed_model(model: LocalEmbeddingModel) -> EmbeddingModel {
        match model {
            LocalEmbeddingModel::AllMiniLML6V2 => EmbeddingModel::AllMiniLML6V2,
            LocalEmbeddingModel::AllMiniLML12V2 => EmbeddingModel::AllMiniLML12V2,
            LocalEmbeddingModel::BGESmallENV15 => EmbeddingModel::BGESmallENV15,
            LocalEmbeddingModel::NomicEmbedTextV15 => EmbeddingModel::NomicEmbedTextV15,
            LocalEmbeddingModel::Custom(name) => {
                EmbeddingModel::from_str(&name).expect("validated custom local embedding model")
            }
        }
    }

    fn ensure_runtime<'a>(
        state: &'a mut RuntimeState,
        cache_dir: &Path,
        model_name: &str,
    ) -> Result<&'a mut TextEmbedding> {
        if state.model_name.as_deref() != Some(model_name) {
            state.model = None;
            state.model_name = None;
            std::fs::create_dir_all(cache_dir).with_context(|| {
                format!("failed to create embeddings cache dir {:?}", cache_dir)
            })?;
            let model = resolve_local_embedding_model(model_name)?;
            let options = InitOptions::new(to_fastembed_model(model))
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(true);
            state.model = Some(TextEmbedding::try_new(options)?);
            state.model_name = Some(model_name.to_string());
        }

        state
            .model
            .as_mut()
            .ok_or_else(|| anyhow!("local embeddings runtime was not initialized"))
    }

    fn runtime_health_snapshot(
        runtime: &Arc<Mutex<RuntimeState>>,
    ) -> (bool, bool, Option<String>, Option<String>) {
        match runtime.try_lock() {
            Ok(runtime) => (
                false,
                runtime.model.is_some(),
                runtime.model_name.clone(),
                runtime.last_error.clone(),
            ),
            Err(std::sync::TryLockError::WouldBlock) => (
                true,
                false,
                None,
                Some("embedding runtime is busy".to_string()),
            ),
            Err(std::sync::TryLockError::Poisoned(_)) => (
                false,
                false,
                None,
                Some("local embeddings runtime lock poisoned".to_string()),
            ),
        }
    }

    async fn health(State(state): State<AppState>) -> impl IntoResponse {
        let (standard_busy, standard_ready, standard_model, standard_error) =
            runtime_health_snapshot(&state.standard_runtime);
        let (hot_busy, hot_ready, hot_model, hot_error) =
            runtime_health_snapshot(&state.hot_runtime);
        let error = standard_error.or(hot_error);
        if error
            .as_deref()
            .is_some_and(|value| value.contains("lock poisoned"))
        {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, error.unwrap());
        }
        Json(HealthResponse {
            ok: true,
            ready: standard_ready || hot_ready,
            standard_ready,
            hot_ready,
            busy: standard_busy || hot_busy,
            model: standard_model
                .or(hot_model)
                .or_else(|| Some(state.default_model.clone())),
            error,
        })
        .into_response()
    }

    async fn embed(
        State(state): State<AppState>,
        payload: Result<Json<EmbedRequest>, JsonRejection>,
    ) -> Response {
        let request_id = state.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let started = Instant::now();

        let Json(request) = match payload {
            Ok(payload) => payload,
            Err(rejection) => {
                let status = rejection.status();
                tracing::warn!(
                    request_id,
                    status = status.as_u16(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "embedding request rejected before JSON body was accepted"
                );
                return rejection.into_response();
            }
        };

        let metrics = embed_request_log_metrics(&request.texts);
        let model_name = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&state.default_model)
            .to_string();

        tracing::info!(
            request_id,
            model = %model_name,
            lane = request.priority.lane_name(),
            text_count = metrics.text_count,
            empty_text_count = metrics.empty_text_count,
            total_text_chars = metrics.total_text_chars,
            max_text_chars = metrics.max_text_chars,
            total_text_bytes = metrics.total_text_bytes,
            "embedding request received"
        );

        if metrics.text_count > MAX_TEXTS_PER_REQUEST {
            tracing::warn!(
                request_id,
                model = %model_name,
                lane = request.priority.lane_name(),
                text_count = metrics.text_count,
                max_texts = MAX_TEXTS_PER_REQUEST,
                status = StatusCode::BAD_REQUEST.as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "embedding request rejected: too many texts"
            );
            return json_error(
                StatusCode::BAD_REQUEST,
                format!(
                    "too many texts in embedding request: max {}, got {}",
                    MAX_TEXTS_PER_REQUEST, metrics.text_count
                ),
            );
        }

        if metrics.text_count == 0 {
            tracing::info!(
                request_id,
                model = %model_name,
                lane = request.priority.lane_name(),
                status = StatusCode::OK.as_u16(),
                embeddings_count = 0usize,
                embedding_dimensions = 0usize,
                elapsed_ms = started.elapsed().as_millis(),
                "embedding request completed"
            );
            return Json(EmbedResponse {
                embeddings: Vec::new(),
            })
            .into_response();
        }

        let cache_dir = state.cache_dir.clone();
        let lane = request.priority;
        let runtime = match lane {
            EmbedPriority::Standard => Arc::clone(&state.standard_runtime),
            EmbedPriority::HotPath => Arc::clone(&state.hot_runtime),
        };
        let worker_model_name = model_name.clone();
        let texts = request.texts;

        let outcome = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>> {
            let mut guard = runtime
                .lock()
                .map_err(|_| anyhow!("local embeddings runtime lock poisoned"))?;
            match ensure_runtime(&mut guard, &cache_dir, &worker_model_name).and_then(|model| {
                model
                    .embed(texts, None)
                    .map_err(|error| anyhow!("local embeddings runtime failed: {}", error))
            }) {
                Ok(embeddings) => {
                    guard.last_error = None;
                    Ok(embeddings)
                }
                Err(error) => {
                    guard.last_error = Some(error.to_string());
                    Err(error)
                }
            }
        })
        .await;

        match outcome {
            Ok(Ok(embeddings)) => {
                let embedding_dimensions = embeddings.first().map(Vec::len).unwrap_or(0);
                tracing::info!(
                    request_id,
                    model = %model_name,
                    lane = lane.lane_name(),
                    status = StatusCode::OK.as_u16(),
                    embeddings_count = embeddings.len(),
                    embedding_dimensions,
                    elapsed_ms = started.elapsed().as_millis(),
                    "embedding request completed"
                );
                Json(EmbedResponse { embeddings }).into_response()
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    request_id,
                    model = %model_name,
                    lane = lane.lane_name(),
                    status = StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    elapsed_ms = started.elapsed().as_millis(),
                    error = %error,
                    "embedding request failed"
                );
                json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("{:#}", error))
            }
            Err(error) => {
                tracing::warn!(
                    request_id,
                    model = %model_name,
                    lane = lane.lane_name(),
                    status = StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    elapsed_ms = started.elapsed().as_millis(),
                    error = %error,
                    "embedding request worker task failed"
                );
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("local embeddings worker task failed: {}", error),
                )
            }
        }
    }

    fn spawn_hot_lane_warmup(state: AppState) {
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let model_name = state.default_model.clone();
            let result = state
                .hot_runtime
                .lock()
                .map_err(|_| anyhow!("local embeddings hot runtime lock poisoned"))
                .and_then(|mut guard| {
                    ensure_runtime(&mut guard, &state.cache_dir, &model_name).and_then(|model| {
                        model
                            .embed(vec!["agentark hot path embedding warmup"], None)
                            .map(|_| ())
                            .map_err(|error| {
                                anyhow!("local embeddings hot warmup failed: {}", error)
                            })
                    })
                });
            match result {
                Ok(()) => tracing::info!(
                    elapsed_ms = started.elapsed().as_millis(),
                    model = %model_name,
                    "embedding hot lane warmup completed"
                ),
                Err(error) => tracing::warn!(
                    elapsed_ms = started.elapsed().as_millis(),
                    model = %model_name,
                    error = %error,
                    "embedding hot lane warmup failed"
                ),
            }
        });
    }

    #[tokio::main]
    pub(crate) async fn run() -> Result<()> {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyper=warn,reqwest=warn"));
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

        let bind = configured_bind();
        let state = AppState {
            cache_dir: configured_cache_dir(),
            default_model: configured_model(),
            standard_runtime: Arc::new(Mutex::new(RuntimeState::default())),
            hot_runtime: Arc::new(Mutex::new(RuntimeState::default())),
            request_counter: Arc::new(AtomicU64::new(0)),
        };

        tracing::info!(
            "Starting AgentArk embeddings sidecar on {} with model {}",
            bind,
            state.default_model
        );
        spawn_hot_lane_warmup(state.clone());

        let app = Router::new()
            .route("/health", get(health))
            .route("/embed", post(embed))
            .with_state(state)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(|request: &Request<_>| {
                        tracing::info_span!(
                            "embeddings_http_request",
                            method = %request.method(),
                            path = %request.uri().path(),
                        )
                    })
                    .on_request(DefaultOnRequest::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO))
                    .on_failure(DefaultOnFailure::new().level(Level::WARN)),
            );

        let listener = tokio::net::TcpListener::bind(&bind)
            .await
            .with_context(|| format!("failed to bind embeddings sidecar on {}", bind))?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn main() -> anyhow::Result<()> {
    non_windows::run()
}
