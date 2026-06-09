use super::*;

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn list_custom_apis(State(state): State<AppState>) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_apis::list_custom_apis(&storage, &config_dir, &data_dir).await {
        Ok(apis) => Json(serde_json::json!({
            "custom_apis": apis,
            "count": apis.len(),
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn preview_custom_api(
    State(state): State<AppState>,
    Json(request): Json<crate::custom_apis::CustomApiPreviewRequest>,
) -> Response {
    let docs_inference_model = {
        let agent = state.agent.read().await;
        agent.llm_for_role(&ModelRole::Fast).clone()
    };
    match crate::custom_apis::preview_custom_api_with_model(request, Some(&docs_inference_model))
        .await
    {
        Ok(preview) => Json(serde_json::json!({
            "status": "ok",
            "preview": preview,
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn create_custom_api(
    State(state): State<AppState>,
    Json(request): Json<crate::custom_apis::CustomApiUpsertRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let request_saves_secret = custom_api_request_saves_secret(&request);
    let mut prior_ready = false;
    let path_id = match crate::custom_apis::custom_api_candidate_id(
        request.id.as_deref(),
        request.name.as_str(),
    ) {
        Some(candidate_id) => {
            match crate::custom_apis::list_custom_apis(
                &agent.storage,
                &agent.config_dir,
                &agent.data_dir,
            )
            .await
            {
                Ok(existing) => {
                    let prior = existing.iter().find(|api| api.config.id == candidate_id);
                    prior_ready = prior.map(custom_api_view_is_ready).unwrap_or(false);
                    prior.map(|_| candidate_id)
                }
                Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
            }
        }
        None => None,
    };
    match crate::custom_apis::upsert_custom_api(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
        &agent.runtime,
        request,
        path_id.as_deref(),
    )
    .await
    {
        Ok(api) => {
            agent
                .refresh_action_catalog_index("custom_api_upsert")
                .await;
            agent
                .refresh_custom_api_capability_readiness_snapshot(
                    crate::core::agent::capability_readiness::CapabilityReadinessSource::RuntimeEvent,
                )
                .await;
            // Reconcile only on a credential-ready EDGE (became ready, or a
            // secret was saved in this request) — editing metadata on an
            // already-connected API must not re-probe and re-notify.
            if custom_api_view_is_ready(&api) && (request_saves_secret || !prior_ready) {
                agent.spawn_custom_api_auth_ready_reconcile(
                    vec![api.config.id.clone()],
                    "custom_api_auth_ready",
                );
            }
            Json(serde_json::json!({ "status": "ok", "custom_api": api })).into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

/// True when the save carries new credential material (the deterministic
/// credential-ready edge signal for settings upserts).
fn custom_api_request_saves_secret(request: &crate::custom_apis::CustomApiUpsertRequest) -> bool {
    request
        .secret
        .as_deref()
        .map(str::trim)
        .is_some_and(|secret| !secret.is_empty())
}

/// Ready = enabled and either credentialed or credential-free by design.
fn custom_api_view_is_ready(view: &crate::custom_apis::CustomApiView) -> bool {
    view.config.enabled
        && (view.secret_configured
            || matches!(
                view.config.auth_mode,
                crate::custom_apis::CustomApiAuthMode::None
            ))
}

pub(super) async fn update_custom_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<crate::custom_apis::CustomApiUpsertRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let request_saves_secret = custom_api_request_saves_secret(&request);
    let prior_ready = match crate::custom_apis::list_custom_apis(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
    )
    .await
    {
        Ok(existing) => existing
            .iter()
            .find(|api| api.config.id == id)
            .map(custom_api_view_is_ready)
            .unwrap_or(false),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    match crate::custom_apis::upsert_custom_api(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
        &agent.runtime,
        request,
        Some(id.as_str()),
    )
    .await
    {
        Ok(api) => {
            agent
                .refresh_action_catalog_index("custom_api_upsert")
                .await;
            agent
                .refresh_custom_api_capability_readiness_snapshot(
                    crate::core::agent::capability_readiness::CapabilityReadinessSource::RuntimeEvent,
                )
                .await;
            // Credential-ready EDGE only (see create_custom_api): metadata
            // edits on a connected API must not re-probe and re-notify.
            if custom_api_view_is_ready(&api) && (request_saves_secret || !prior_ready) {
                agent.spawn_custom_api_auth_ready_reconcile(
                    vec![api.config.id.clone()],
                    "custom_api_auth_ready",
                );
            }
            Json(serde_json::json!({ "status": "ok", "custom_api": api })).into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn delete_custom_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::custom_apis::delete_custom_api(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
        &agent.runtime,
        id.as_str(),
    )
    .await
    {
        Ok(()) => {
            agent
                .refresh_action_catalog_index("custom_api_delete")
                .await;
            agent
                .refresh_custom_api_capability_readiness_snapshot(
                    crate::core::agent::capability_readiness::CapabilityReadinessSource::RuntimeEvent,
                )
                .await;
            Json(serde_json::json!({ "status": "ok" })).into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn test_custom_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::custom_apis::test_custom_api(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
        &agent.runtime,
        id.as_str(),
    )
    .await
    {
        Ok(result) => {
            agent
                .refresh_custom_api_capability_readiness_snapshot(
                    crate::core::agent::capability_readiness::CapabilityReadinessSource::RuntimeEvent,
                )
                .await;
            Json(serde_json::json!({ "status": "ok", "result": result })).into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}
