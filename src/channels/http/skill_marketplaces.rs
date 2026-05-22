use super::*;
use crate::core::skill_marketplaces::{
    SkillMarketplaceUpsertRequest, load_marketplaces, refresh_marketplace, remove_marketplace,
    upsert_marketplace,
};

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn list_skill_marketplaces(State(state): State<AppState>) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    match load_marketplaces(&storage).await {
        Ok(marketplaces) => {
            let installers_count: usize = marketplaces
                .iter()
                .map(|marketplace| marketplace.installers.len())
                .sum();
            Json(serde_json::json!({
                "marketplaces": marketplaces,
                "count": marketplaces.len(),
                "installers_count": installers_count,
            }))
            .into_response()
        }
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn create_skill_marketplace(
    State(state): State<AppState>,
    Json(request): Json<SkillMarketplaceUpsertRequest>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    match upsert_marketplace(&storage, None, request).await {
        Ok((status, marketplace)) => Json(serde_json::json!({
            "status": status,
            "marketplace": marketplace,
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn update_skill_marketplace(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SkillMarketplaceUpsertRequest>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    match upsert_marketplace(&storage, Some(id.as_str()), request).await {
        Ok((status, marketplace)) => Json(serde_json::json!({
            "status": status,
            "marketplace": marketplace,
        }))
        .into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn delete_skill_marketplace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    match remove_marketplace(&storage, id.as_str()).await {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn refresh_skill_marketplace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    match refresh_marketplace(&storage, id.as_str()).await {
        Ok(marketplace) => Json(serde_json::json!({
            "status": if marketplace.last_error.is_some() { "warning" } else { "ok" },
            "marketplace": marketplace,
        }))
        .into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}
