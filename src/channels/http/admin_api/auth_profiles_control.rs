use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::collections::BTreeMap;

use super::{auth, AppState, ErrorResponse};

#[derive(Debug, Deserialize, Default)]
pub(super) struct StartOAuthProfileRequest {
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RevokeAuthProfileRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct CaptureSessionMaterialRequest {
    #[serde(default)]
    pub cookies: Vec<crate::core::connectivity::auth_profiles::AuthCookieRecord>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub browser_profile_id: Option<String>,
    #[serde(default)]
    pub login_url: Option<String>,
}

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn list_profiles(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::list(&agent.storage)
        .await
    {
        Ok(payload) => Json(payload).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn get_profile(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::get(
        &agent.storage,
        &id,
    )
    .await
    {
        Ok(Some(profile)) => Json(serde_json::json!({ "profile": profile })).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Auth profile not found"),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn create_profile(
    State(state): State<AppState>,
    Json(request): Json<crate::core::connectivity::auth_profiles::AuthProfileUpsert>,
) -> Response {
    upsert_profile_impl(state, None, request).await
}

pub(super) async fn update_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<crate::core::connectivity::auth_profiles::AuthProfileUpsert>,
) -> Response {
    upsert_profile_impl(state, Some(id), request).await
}

pub(super) async fn delete_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::delete(
        &agent.storage,
        &id,
    )
    .await
    {
        Ok(true) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Auth profile not found"),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn revoke_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<RevokeAuthProfileRequest>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::revoke(
        &agent.storage,
        &id,
        request.reason,
    )
    .await
    {
        // No reconcile hook here: revoke() sets revoked_at and clears the
        // session material, and profile_http_status reports ready=false
        // whenever revoked_at is set — a revoked profile can never be ready,
        // so a ready-gated reconcile on this path is unreachable by
        // construction.
        Ok(profile) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn start_oauth_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<StartOAuthProfileRequest>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let redirect_uri = match super::oauth_redirect_uri_for_request(
        &state,
        &headers,
        request.redirect_uri.as_deref(),
    ) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };
    let (state_token, code_challenge) =
        auth::issue_auth_profile_oauth_state_with_pkce(&state, &id, Some(redirect_uri.clone()))
            .await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::oauth_authorization_url(
        &storage,
        &id,
        &state_token,
        Some(&code_challenge),
        Some(redirect_uri.as_str()),
    )
    .await
    {
        Ok(auth_url) => Json(serde_json::json!({
            "status": "ok",
            "auth_url": auth_url,
            "state": state_token,
        }))
        .into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn capture_session_material(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CaptureSessionMaterialRequest>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::capture_session_material(
        &agent.storage,
        &id,
        request.cookies,
        request.headers,
        request.origin,
        request.browser_profile_id,
        request.login_url,
    )
    .await
    {
        Ok(profile) => {
            // Session capture is a credential-ready edge: it clears
            // revoked_at/last_error and populates session material, so a
            // cookie/browser profile can become ready HERE without ever
            // passing through upsert. Mirror the upsert hook so dependent
            // custom APIs get probed and surfaced without an agent run.
            if profile.ready {
                agent.spawn_custom_api_auth_profile_ready_reconcile(
                    profile.id.clone(),
                    "auth_profile_ready",
                );
            }
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

async fn upsert_profile_impl(
    state: AppState,
    path_id: Option<String>,
    mut request: crate::core::connectivity::auth_profiles::AuthProfileUpsert,
) -> Response {
    request.id = path_id.or(request.id);
    let agent = state.agent.read().await;
    match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::upsert(
        &agent.storage,
        request,
    )
    .await
    {
        Ok(profile) => {
            if profile.ready {
                agent.spawn_custom_api_auth_profile_ready_reconcile(
                    profile.id.clone(),
                    "auth_profile_ready",
                );
            }
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}
