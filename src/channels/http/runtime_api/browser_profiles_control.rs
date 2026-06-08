use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
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
    let exists = match crate::core::BrowserProfileControlPlane::list(&agent.storage).await {
        Ok(payload) => payload.profiles.iter().any(|profile| profile.id == id),
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    if !exists {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Browser profile not found".to_string(),
            }),
        )
            .into_response();
    }
    let closed_sessions = match agent.browser_sessions.stop_profile_sessions(&id).await {
        Ok(views) => views.len(),
        Err(error) => {
            tracing::warn!(
                "Failed to close browser sessions before deleting profile '{}': {}",
                id,
                error
            );
            0
        }
    };
    let storage_deleted = match agent.browser_sessions.delete_profile_storage(&id).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(
                "Failed to delete browser storage for profile '{}': {}",
                id,
                error
            );
            false
        }
    };
    match crate::core::BrowserProfileControlPlane::delete(&agent.storage, &id).await {
        Ok(true) => Json(serde_json::json!({
            "status": "ok",
            "closed_sessions": closed_sessions,
            "storage_deleted": storage_deleted,
        }))
        .into_response(),
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

pub(super) async fn launch_profile_browser(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let profile = match crate::core::BrowserProfileControlPlane::list(&agent.storage).await {
        Ok(payload) => payload
            .profiles
            .into_iter()
            .find(|profile| profile.id == id),
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    let Some(profile) = profile else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Browser profile not found".to_string(),
            }),
        )
            .into_response();
    };
    if !profile.enabled {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Browser profile is disabled".to_string(),
            }),
        )
            .into_response();
    }

    match agent
        .browser_sessions
        .start_profile_login_session(
            crate::core::connectivity::browser_session::BrowserSessionProfileContext::from_browser_profile(
                &profile,
            ),
        )
        .await
    {
        Ok(view) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "session": view,
            })),
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

pub(super) async fn close_profile_browser(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let exists = match crate::core::BrowserProfileControlPlane::list(&agent.storage).await {
        Ok(payload) => payload.profiles.iter().any(|profile| profile.id == id),
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    if !exists {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Browser profile not found".to_string(),
            }),
        )
            .into_response();
    }
    let stopped = match agent.browser_sessions.stop_profile_sessions(&id).await {
        Ok(views) => views,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    let ended_at = chrono::Utc::now().to_rfc3339();
    for view in &stopped {
        let _ = crate::core::BrowserProfileControlPlane::record_session(
            &agent.storage,
            crate::core::BrowserProfileSessionRecord {
                profile_id: id.clone(),
                session_id: Some(view.id.clone()),
                started_at: view.created_at.clone(),
                ended_at: Some(ended_at.clone()),
                duration_secs: None,
                title: view.page_title.clone(),
                url: view.page_url.clone(),
                outcome: "saved".to_string(),
                channel: Some("web".to_string()),
                note: Some("Manual login profile browser closed and saved.".to_string()),
            },
        )
        .await;
    }
    if !stopped.is_empty() {
        let (login_state, login_note) = profile_close_login_outcome();
        let _ = crate::core::BrowserProfileControlPlane::upsert(
            &agent.storage,
            crate::core::BrowserProfileUpsert {
                id: Some(id.clone()),
                login_state: Some(login_state),
                login_checked_at: Some(ended_at),
                login_note: Some(login_note),
                ..Default::default()
            },
        )
        .await;
    }
    Json(serde_json::json!({
        "status": "ok",
        "closed_sessions": stopped.len(),
        "sessions": stopped,
    }))
    .into_response()
}

fn profile_close_login_outcome() -> (crate::core::BrowserLoginState, String) {
    (
        crate::core::BrowserLoginState::Unknown,
        "Browser state was saved. Login success was not verified by AgentArk.".to_string(),
    )
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
    let managed = matches!(target_kind, crate::core::BrowserProfileTargetKind::Sandbox);
    let metadata = request.metadata.unwrap_or_else(|| {
        serde_json::json!({
            "browser": browser,
            "managed": managed,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_close_saves_state_without_claiming_verified_login() {
        let (state, note) = profile_close_login_outcome();

        assert_eq!(state, crate::core::BrowserLoginState::Unknown);
        assert!(note.contains("saved"));
        assert!(note.contains("not verified"));
    }
}
