use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use super::{AppState, ErrorResponse};

#[derive(Debug, Deserialize, Default)]
pub(super) struct DisableProfileRequest {
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct DisableProviderRequest {
    #[serde(default)]
    pub disabled: bool,
}

pub(super) async fn list_failover(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::list(&agent.storage).await {
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

pub(super) async fn upsert_profile(
    State(state): State<AppState>,
    Json(request): Json<crate::core::AuthProfileUpsert>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::upsert_auth_profile(&agent.storage, request).await
    {
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

pub(super) async fn set_default_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::set_default_auth_profile(&agent.storage, &id)
        .await
    {
        Ok(Some(profile)) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Auth profile not found".to_string(),
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

pub(super) async fn disable_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<DisableProfileRequest>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::disable_auth_profile(
        &agent.storage,
        &id,
        request.disabled,
    )
    .await
    {
        Ok(Some(profile)) => {
            Json(serde_json::json!({ "status": "ok", "profile": profile })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Auth profile not found".to_string(),
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

pub(super) async fn clear_profile_cooldown(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::list(&agent.storage).await {
        Ok(payload) => {
            let provider_id = payload
                .auth_profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| profile.provider_id.clone());
            let Some(provider_id) = provider_id else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Auth profile not found".to_string(),
                    }),
                )
                    .into_response();
            };
            match crate::core::ModelFailoverControlPlane::clear_cooldowns(
                &agent.storage,
                Some(provider_id.as_str()),
            )
            .await
            {
                Ok(result) => {
                    Json(serde_json::json!({ "status": "ok", "result": result })).into_response()
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
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn rotate_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::rotate_auth_profile(&agent.storage, &id).await {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "result": result })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn upsert_provider(
    State(state): State<AppState>,
    Json(request): Json<crate::core::ProviderHealthUpsert>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::upsert_provider_health(&agent.storage, request)
        .await
    {
        Ok(provider) => {
            Json(serde_json::json!({ "status": "ok", "provider": provider })).into_response()
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

pub(super) async fn disable_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<DisableProviderRequest>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::disable_provider(
        &agent.storage,
        &id,
        request.disabled,
    )
    .await
    {
        Ok(Some(provider)) => {
            Json(serde_json::json!({ "status": "ok", "provider": provider })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Provider health record not found".to_string(),
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

pub(super) async fn clear_provider_cooldown(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::clear_cooldowns(&agent.storage, Some(&id)).await {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "result": result })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn upsert_chain(
    State(state): State<AppState>,
    Json(request): Json<crate::core::FallbackChainUpsert>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::upsert_fallback_chain(&agent.storage, request)
        .await
    {
        Ok(chain) => Json(serde_json::json!({ "status": "ok", "chain": chain })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn select_candidate(
    State(state): State<AppState>,
    Json(request): Json<crate::core::ModelFailoverSelectionRequest>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::ModelFailoverControlPlane::select_candidate(&agent.storage, request).await {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "result": result })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}
