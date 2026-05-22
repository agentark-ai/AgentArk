use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{AppState, ErrorResponse};

pub(super) async fn get_overview(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::GatewayOpsControlPlane::overview(&agent).await {
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
