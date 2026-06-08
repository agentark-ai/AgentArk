use super::*;

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({ "status": "error", "error": error.to_string() })),
    )
        .into_response()
}

fn integration_sync_error_status(error: &anyhow::Error) -> StatusCode {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("already in progress") {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct IntegrationSyncFeedQuery {
    pub integration_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IntegrationSyncRunsQuery {
    pub integration_id: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub(super) async fn list_integration_sync_statuses(State(state): State<AppState>) -> Response {
    let shared_agent = state.agent.clone();
    let ctx = {
        let agent = shared_agent.read().await;
        crate::core::connectivity::integration_sync::context_from_agent(
            &agent,
            Some(shared_agent.clone()),
        )
    };
    let statuses = crate::core::connectivity::integration_sync::list_statuses(&ctx).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "statuses": statuses })),
    )
        .into_response()
}

pub(super) async fn list_integration_sync_feed(
    State(state): State<AppState>,
    Query(query): Query<IntegrationSyncFeedQuery>,
) -> Response {
    let shared_agent = state.agent.clone();
    let ctx = {
        let agent = shared_agent.read().await;
        crate::core::connectivity::integration_sync::context_from_agent(
            &agent,
            Some(shared_agent.clone()),
        )
    };
    let items = crate::core::connectivity::integration_sync::list_feed_items(
        &ctx,
        query.integration_id.as_deref(),
        query.limit.unwrap_or(20),
    )
    .await;
    (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response()
}

pub(super) async fn list_integration_sync_runs(
    State(state): State<AppState>,
    Query(query): Query<IntegrationSyncRunsQuery>,
) -> Response {
    let shared_agent = state.agent.clone();
    let ctx = {
        let agent = shared_agent.read().await;
        crate::core::connectivity::integration_sync::context_from_agent(
            &agent,
            Some(shared_agent.clone()),
        )
    };
    let page = crate::core::connectivity::integration_sync::list_runs(
        &ctx,
        query.integration_id.as_deref(),
        query.limit.unwrap_or(20),
        query.offset.unwrap_or(0),
    )
    .await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "items": page.items,
            "total": page.total,
            "stats": page.stats
        })),
    )
        .into_response()
}

pub(super) async fn update_integration_sync_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<crate::core::connectivity::integration_sync::IntegrationSyncUpdateRequest>,
) -> Response {
    let shared_agent = state.agent.clone();
    let ctx = {
        let agent = shared_agent.read().await;
        crate::core::connectivity::integration_sync::context_from_agent(
            &agent,
            Some(shared_agent.clone()),
        )
    };
    match crate::core::connectivity::integration_sync::update_config(&ctx, &id, request).await {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "sync": status })),
        )
            .into_response(),
        Err(error) => error_response(integration_sync_error_status(&error), error),
    }
}

pub(super) async fn run_integration_sync_now(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let shared_agent = state.agent.clone();
    let ctx = {
        let agent = shared_agent.read().await;
        crate::core::connectivity::integration_sync::context_from_agent(
            &agent,
            Some(shared_agent.clone()),
        )
    };
    match crate::core::connectivity::integration_sync::sync_now(&ctx, &id).await {
        Ok(status) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "sync": status })),
        )
            .into_response(),
        Err(error) => error_response(integration_sync_error_status(&error), error),
    }
}
