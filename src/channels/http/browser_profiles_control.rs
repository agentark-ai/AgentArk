use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::{AppState, ErrorResponse};

#[derive(Debug, Deserialize)]
pub(super) struct UpsertBrowserProfileRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub browser: Option<String>,
    #[serde(default = "default_true")]
    pub managed: bool,
    #[serde(default)]
    pub target_kind: Option<crate::core::BrowserProfileTargetKind>,
    #[serde(default)]
    pub target_endpoint: Option<String>,
    #[serde(default)]
    pub target_profile_path: Option<String>,
    #[serde(default)]
    pub target_workspace: Option<String>,
    #[serde(default)]
    pub login_state: Option<crate::core::BrowserLoginState>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct LockBrowserProfileRequest {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RecordBrowserSessionRequest {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

pub(super) async fn list_profiles(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::BrowserProfileControlPlane::list(&agent.storage).await {
        Ok(payload) => Json(payload).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn create_profile(
    State(state): State<AppState>,
    Json(request): Json<UpsertBrowserProfileRequest>,
) -> Response {
    upsert_profile_impl(state, None, request).await
}

pub(super) async fn update_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpsertBrowserProfileRequest>,
) -> Response {
    upsert_profile_impl(state, Some(id), request).await
}

pub(super) async fn delete_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::BrowserProfileControlPlane::delete(&agent.storage, &id).await {
        Ok(true) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Browser profile not found".to_string(),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn lock_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<LockBrowserProfileRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let payload = crate::core::BrowserProfileLockRequest {
        owner: request
            .owner
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "browser-panel".to_string()),
        reason: request.reason.filter(|value| !value.trim().is_empty()),
        expires_at: request.expires_at.filter(|value| !value.trim().is_empty()),
    };
    match crate::core::BrowserProfileControlPlane::lock(&agent.storage, &id, payload).await {
        Ok(profile) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn unlock_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<LockBrowserProfileRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let owner = request
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match crate::core::BrowserProfileControlPlane::unlock(&agent.storage, &id, owner).await {
        Ok(profile) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn record_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<RecordBrowserSessionRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let entry = crate::core::BrowserProfileSessionRecord {
        profile_id: id,
        session_id: request.session_id.filter(|value| !value.trim().is_empty()),
        started_at: request
            .started_at
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        ended_at: request.ended_at.filter(|value| !value.trim().is_empty()),
        duration_secs: request.duration_secs,
        title: request.title.filter(|value| !value.trim().is_empty()),
        url: request.url.filter(|value| !value.trim().is_empty()),
        outcome: request
            .outcome
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "completed".to_string()),
        channel: request.channel.filter(|value| !value.trim().is_empty()),
        note: request.note.filter(|value| !value.trim().is_empty()),
    };
    match crate::core::BrowserProfileControlPlane::record_session(&agent.storage, entry).await {
        Ok(profile) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn upsert_profile_impl(
    state: AppState,
    path_id: Option<String>,
    request: UpsertBrowserProfileRequest,
) -> Response {
    let agent = state.agent.read().await;
    let target_kind = request.target_kind.unwrap_or(if request.managed {
        crate::core::BrowserProfileTargetKind::Sandbox
    } else {
        crate::core::BrowserProfileTargetKind::Host
    });
    let browser = request
        .browser
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "chrome".to_string());
    let metadata = request.metadata.unwrap_or_else(|| {
        serde_json::json!({
            "browser": browser,
            "managed": request.managed,
        })
    });
    let payload = crate::core::BrowserProfileUpsert {
        id: path_id
            .or(request.id)
            .filter(|value| !value.trim().is_empty()),
        name: Some(request.name.trim().to_string()),
        description: request.description.filter(|value| !value.trim().is_empty()),
        target_kind: Some(target_kind),
        target_endpoint: request
            .target_endpoint
            .filter(|value| !value.trim().is_empty()),
        target_profile_path: request
            .target_profile_path
            .filter(|value| !value.trim().is_empty()),
        target_workspace: request
            .target_workspace
            .filter(|value| !value.trim().is_empty()),
        login_state: request.login_state,
        login_checked_at: Some(chrono::Utc::now().to_rfc3339()),
        login_note: None,
        recent_sessions: None,
        tags: Vec::new(),
        enabled: Some(true),
        last_used_at: None,
        last_error: None,
        metadata: Some(metadata),
    };
    match crate::core::BrowserProfileControlPlane::upsert(&agent.storage, payload).await {
        Ok(profile) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn default_true() -> bool {
    true
}
