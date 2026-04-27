use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};

const DEFAULT_BIND: &str = "0.0.0.0:8993";
const DEFAULT_CACHE_DIR: &str = "/app/prebuilt-embeddings-cache";
const DEFAULT_MODEL: &str = "BAAI/bge-small-en-v1.5";
const MAX_TEXTS_PER_REQUEST: usize = 64;

#[derive(Clone)]
struct AppState {
    cache_dir: PathBuf,
    default_model: String,
    runtime: Arc<Mutex<RuntimeState>>,
}

#[derive(Default)]
struct RuntimeState {
    model_name: Option<String>,
    model: Option<TextEmbedding>,
    last_error: Option<String>,
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
}

#[derive(Serialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    ready: bool,
    model: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn json_error(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(ErrorResponse { error: error.into() })).into_response()
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
        std::fs::create_dir_all(cache_dir)
            .with_context(|| format!("failed to create embeddings cache dir {:?}", cache_dir))?;
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

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match state.runtime.try_lock() {
        Ok(runtime) => Json(HealthResponse {
            ok: true,
            ready: runtime.model.is_some(),
            model: runtime
                .model_name
                .clone()
                .or_else(|| Some(state.default_model.clone())),
            error: runtime.last_error.clone(),
        })
        .into_response(),
        Err(std::sync::TryLockError::WouldBlock) => Json(HealthResponse {
            ok: true,
            ready: false,
            model: Some(state.default_model.clone()),
            error: Some("embedding runtime is busy".to_string()),
        })
        .into_response(),
        Err(std::sync::TryLockError::Poisoned(_)) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "local embeddings runtime lock poisoned",
        ),
    }
}

async fn embed(State(state): State<AppState>, Json(request): Json<EmbedRequest>) -> Response {
    if request.texts.len() > MAX_TEXTS_PER_REQUEST {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!(
                "too many texts in embedding request: max {}, got {}",
                MAX_TEXTS_PER_REQUEST,
                request.texts.len()
            ),
        );
    }

    if request.texts.is_empty() {
        return Json(EmbedResponse {
            embeddings: Vec::new(),
        })
        .into_response();
    }

    let model_name = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&state.default_model)
        .to_string();
    let cache_dir = state.cache_dir.clone();
    let runtime = Arc::clone(&state.runtime);
    let texts = request.texts;

    let outcome = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>> {
        let mut guard = runtime
            .lock()
            .map_err(|_| anyhow!("local embeddings runtime lock poisoned"))?;
        match ensure_runtime(&mut guard, &cache_dir, &model_name).and_then(|model| {
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
        Ok(Ok(embeddings)) => Json(EmbedResponse { embeddings }).into_response(),
        Ok(Err(error)) => {
            json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("{:#}", error))
        }
        Err(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("local embeddings worker task failed: {}", error),
        ),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyper=warn,reqwest=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init();

    let bind = configured_bind();
    let state = AppState {
        cache_dir: configured_cache_dir(),
        default_model: configured_model(),
        runtime: Arc::new(Mutex::new(RuntimeState::default())),
    };

    tracing::info!(
        "Starting AgentArk embeddings sidecar on {} with model {}",
        bind,
        state.default_model
    );

    let app = Router::new()
        .route("/health", get(health))
        .route("/embed", post(embed))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("failed to bind embeddings sidecar on {}", bind))?;
    axum::serve(listener, app).await?;
    Ok(())
}
