use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use super::{AppState, ErrorResponse};

#[derive(Debug, Deserialize)]
pub(super) struct UpsertChannelAccountRequest {
    pub channel_id: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub peer_scope: Option<String>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpsertRouteRuleRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub match_kind: Option<String>,
    #[serde(default)]
    pub match_value: Option<String>,
    #[serde(default)]
    pub target_kind: Option<String>,
    #[serde(default)]
    pub target_value: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub conversation_scope: Option<String>,
    #[serde(default)]
    pub broadcast_group_id: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateBroadcastGroupRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
}

pub(super) async fn get_channels(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::load_gateway_channels(&agent.storage, &agent.config).await {
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

pub(super) async fn create_channel_account(
    State(state): State<AppState>,
    Json(request): Json<UpsertChannelAccountRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let account = crate::core::GatewayChannelAccountUpsert {
        id: None,
        channel_id: request.channel_id.trim().to_string(),
        label: Some(request.label.trim().to_string()),
        enabled: Some(request.enabled),
        status: Some(request.status.unwrap_or("missing_config".to_string())),
        peer_scope: request.peer_scope.filter(|value| !value.trim().is_empty()),
        default_agent_id: request
            .default_agent_id
            .filter(|value| !value.trim().is_empty()),
        last_seen_at: None,
        last_error: None,
        note: None,
        metadata: request.metadata,
    };
    match crate::core::upsert_gateway_channel_account(&agent.storage, account).await {
        Ok(account) => Json(serde_json::json!({
            "status": "ok",
            "account": account,
        }))
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

pub(super) async fn update_channel_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpsertChannelAccountRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let account = crate::core::GatewayChannelAccountUpsert {
        id: Some(id),
        channel_id: request.channel_id.trim().to_string(),
        label: Some(request.label.trim().to_string()),
        enabled: Some(request.enabled),
        status: Some(request.status.unwrap_or("missing_config".to_string())),
        peer_scope: request.peer_scope.filter(|value| !value.trim().is_empty()),
        default_agent_id: request
            .default_agent_id
            .filter(|value| !value.trim().is_empty()),
        last_seen_at: None,
        last_error: None,
        note: None,
        metadata: request.metadata,
    };
    match crate::core::upsert_gateway_channel_account(&agent.storage, account).await {
        Ok(account) => Json(serde_json::json!({
            "status": "ok",
            "account": account,
        }))
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

pub(super) async fn delete_channel_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::delete_gateway_channel_account(&agent.storage, &id).await {
        Ok(true) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Channel account not found".to_string(),
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

pub(super) async fn get_routing(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match crate::core::load_gateway_routing(&agent.storage).await {
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

pub(super) async fn create_route_rule(
    State(state): State<AppState>,
    Json(request): Json<UpsertRouteRuleRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let rule = materialize_rule(String::new(), request);
    match crate::core::upsert_gateway_route_rule(&agent.storage, rule).await {
        Ok(rule) => Json(serde_json::json!({
            "status": "ok",
            "rule": rule,
        }))
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

pub(super) async fn update_route_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpsertRouteRuleRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let rule = materialize_rule(id, request);
    match crate::core::upsert_gateway_route_rule(&agent.storage, rule).await {
        Ok(rule) => Json(serde_json::json!({
            "status": "ok",
            "rule": rule,
        }))
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

pub(super) async fn delete_route_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::delete_gateway_route_rule(&agent.storage, &id).await {
        Ok(true) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Route rule not found".to_string(),
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

pub(super) async fn create_broadcast_group(
    State(state): State<AppState>,
    Json(request): Json<CreateBroadcastGroupRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let group = crate::core::GatewayBroadcastGroupCreate {
        name: request.name.trim().to_string(),
        description: request.description.filter(|value| !value.trim().is_empty()),
        channels: request.channels,
        targets: request.targets,
    };
    match crate::core::create_gateway_broadcast_group(&agent.storage, group).await {
        Ok(group) => Json(serde_json::json!({
            "status": "ok",
            "group": group,
        }))
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

pub(super) async fn simulate_routing(
    State(state): State<AppState>,
    Json(request): Json<crate::core::GatewayRoutingSimulationRequest>,
) -> Response {
    let agent = state.agent.read().await;
    match crate::core::load_gateway_routing(&agent.storage).await {
        Ok(payload) => {
            let simulation = crate::core::simulate_gateway_routing(&payload.rules, &request);
            Json(serde_json::json!({
                "status": "ok",
                "simulation": simulation,
            }))
            .into_response()
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

fn materialize_rule(
    id: String,
    request: UpsertRouteRuleRequest,
) -> crate::core::GatewayRouteRuleUpsert {
    let match_kind = request.match_kind.unwrap_or("all".to_string());
    let match_value = request.match_value.unwrap_or_default();
    let target_kind = request.target_kind.unwrap_or("agent".to_string());
    let target_value = request.target_value.unwrap_or_default();
    let fallback_name = if match_value.trim().is_empty() {
        format!("{} -> {}", match_kind, target_kind)
    } else {
        format!("{}:{} -> {}", match_kind, match_value, target_value)
    };
    crate::core::GatewayRouteRuleUpsert {
        id: if id.trim().is_empty() { None } else { Some(id) },
        name: Some(
            request
                .name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(fallback_name),
        ),
        enabled: Some(request.enabled),
        priority: Some(request.priority),
        channel_id: request.channel_id.filter(|value| !value.trim().is_empty()),
        account_id: request.account_id.filter(|value| !value.trim().is_empty()),
        match_kind: Some(match_kind),
        match_value: Some(match_value),
        target_kind: Some(target_kind),
        target_value: Some(target_value),
        agent_id: request.agent_id.filter(|value| !value.trim().is_empty()),
        conversation_scope: request
            .conversation_scope
            .filter(|value| !value.trim().is_empty()),
        broadcast_group_id: request
            .broadcast_group_id
            .filter(|value| !value.trim().is_empty()),
        notes: request.notes.filter(|value| !value.trim().is_empty()),
    }
}

fn default_true() -> bool {
    true
}
