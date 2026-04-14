use crate::workspace::protocol::{BlobResponse, InternalServiceHealth, WorkspaceStatusResponse};
use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, put},
    Json, Router,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Component, Path as FsPath, PathBuf};

#[derive(Debug, Clone)]
pub struct WorkspaceServiceConfig {
    pub bind_addr: String,
    pub root_dir: PathBuf,
    pub token: Option<String>,
}

impl WorkspaceServiceConfig {
    pub fn from_env_paths(config_dir: PathBuf, data_dir: PathBuf) -> Result<Self> {
        let token = crate::clients::load_or_create_internal_service_token(
            &config_dir,
            crate::clients::InternalServiceKind::Workspace,
        )?;
        Ok(Self {
            bind_addr: std::env::var("AGENTARK_WORKSPACE_BIND")
                .unwrap_or_else(|_| "127.0.0.1:8992".to_string()),
            root_dir: std::env::var("AGENTARK_WORKSPACE_ROOT")
                .ok()
                .map(PathBuf::from)
                .unwrap_or(data_dir),
            token: Some(token),
        })
    }
}

fn validate_internal_service_token(
    token: Option<&str>,
    env_name: &str,
    service_name: &str,
) -> Result<()> {
    let Some(value) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        anyhow::bail!(
            "{} requires {} to be set to a non-empty shared secret",
            service_name,
            env_name
        );
    };
    if value.eq_ignore_ascii_case("change-me") {
        anyhow::bail!(
            "{} requires {} to be changed from the insecure default placeholder",
            service_name,
            env_name
        );
    }
    Ok(())
}

#[derive(Clone)]
struct WorkspaceState {
    config: WorkspaceServiceConfig,
}

pub async fn run_service(config: WorkspaceServiceConfig) -> Result<()> {
    validate_internal_service_token(
        config.token.as_deref(),
        "AGENTARK_WORKSPACE_TOKEN",
        "Workspace service",
    )?;
    tokio::fs::create_dir_all(&config.root_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create workspace root {}",
                config.root_dir.display()
            )
        })?;
    let bind_addr = config.bind_addr.clone();
    let state = WorkspaceState { config };
    let app = Router::new()
        .route("/health", get(health))
        .route("/internal/v1/status", get(status))
        .route(
            "/internal/v1/blobs/{*path}",
            put(put_blob).get(get_blob).delete(delete_blob),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind workspace service at {}", bind_addr))?;
    tracing::info!("Workspace service listening on {}", bind_addr);
    axum::serve(listener, app)
        .await
        .context("Workspace service stopped unexpectedly")
}

async fn health(State(state): State<WorkspaceState>) -> impl IntoResponse {
    Json(InternalServiceHealth {
        service: "workspace".to_string(),
        mode: "workspace".to_string(),
        ok: true,
        details: BTreeMap::from([(
            "root_dir".to_string(),
            state.config.root_dir.display().to_string(),
        )]),
    })
}

async fn status(
    State(state): State<WorkspaceState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return status.into_response();
    }
    Json(WorkspaceStatusResponse {
        service: "workspace".to_string(),
        mode: "workspace".to_string(),
        root_dir: state.config.root_dir.clone(),
        token_configured: state.config.token.is_some(),
    })
    .into_response()
}

fn authorize_internal(
    headers: &axum::http::HeaderMap,
    token: Option<&str>,
) -> Result<(), StatusCode> {
    let Some(expected) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    if provided.is_some_and(|value| {
        crate::security::constant_time_eq(value.as_bytes(), expected.as_bytes())
    }) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn put_blob(
    State(state): State<WorkspaceState>,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return status.into_response();
    }
    match write_blob(&state.config.root_dir, &path, &body).await {
        Ok(_) => Json(BlobResponse {
            path,
            bytes: body.len(),
        })
        .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn get_blob(
    State(state): State<WorkspaceState>,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return status.into_response();
    }
    match read_blob(&state.config.root_dir, &path).await {
        Ok(bytes) => {
            let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            response
        }
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn delete_blob(
    State(state): State<WorkspaceState>,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return status.into_response();
    }
    match delete_blob_from_root(&state.config.root_dir, &path).await {
        Ok(_) => Json(json!({ "status": "ok", "path": path })).into_response(),
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

fn sanitize_relative_path(path: &str) -> Result<PathBuf> {
    let candidate = FsPath::new(path.trim_start_matches('/'));
    let mut sanitized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            _ => anyhow::bail!("invalid workspace path '{}'", path),
        }
    }
    if sanitized.as_os_str().is_empty() {
        anyhow::bail!("workspace path cannot be empty");
    }
    Ok(sanitized)
}

async fn write_blob(root_dir: &FsPath, path: &str, bytes: &Bytes) -> Result<()> {
    let rel = sanitize_relative_path(path)?;
    let full_path = root_dir.join(rel);
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(full_path, bytes).await?;
    Ok(())
}

async fn read_blob(root_dir: &FsPath, path: &str) -> Result<Vec<u8>> {
    let rel = sanitize_relative_path(path)?;
    let full_path = root_dir.join(rel);
    Ok(tokio::fs::read(full_path).await?)
}

async fn delete_blob_from_root(root_dir: &FsPath, path: &str) -> Result<()> {
    let rel = sanitize_relative_path(path)?;
    let full_path = root_dir.join(rel);
    tokio::fs::remove_file(full_path).await?;
    Ok(())
}
