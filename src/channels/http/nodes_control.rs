use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::{AppState, ErrorResponse};

#[derive(Debug, Deserialize)]
pub(super) struct UpsertNodeRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<crate::core::NodeCapability>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub transport: Option<crate::core::NodeTransportKind>,
    #[serde(default)]
    pub state: Option<crate::core::NodeState>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct NodeHeartbeatPayload {
    #[serde(default)]
    pub state: Option<crate::core::NodeState>,
    #[serde(default)]
    pub transport: Option<crate::core::NodeTransportKind>,
    #[serde(default)]
    pub capabilities: Vec<crate::core::NodeCapability>,
    #[serde(default)]
    pub metrics: BTreeMap<String, String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct NodeCommandPayload {
    pub command: String,
    #[serde(default = "default_true")]
    pub success: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub output_preview: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub context: BTreeMap<String, String>,
}

pub(super) async fn list_nodes(State(state): State<AppState>) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let plane = crate::core::NodeControlPlane::new(storage);
    match (plane.list().await, plane.status().await) {
        (Ok(nodes), Ok(status)) => Json(serde_json::json!({
            "status": "ok",
            "nodes": nodes,
            "summary": status.summary,
            "generated_at": status.generated_at,
        }))
        .into_response(),
        (Err(error), _) | (_, Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn create_node(
    State(state): State<AppState>,
    Json(request): Json<UpsertNodeRequest>,
) -> Response {
    upsert_node_impl(state, None, request).await
}

pub(super) async fn update_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpsertNodeRequest>,
) -> Response {
    upsert_node_impl(state, Some(id), request).await
}

pub(super) async fn revoke_node(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let plane = crate::core::NodeControlPlane::new(storage);
    match plane.revoke(&id).await {
        Ok(Some(node)) => Json(serde_json::json!({ "status": "ok", "node": node })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Node not found".to_string(),
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

pub(super) async fn heartbeat_node(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<NodeHeartbeatPayload>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let plane = crate::core::NodeControlPlane::new(storage);
    let payload = crate::core::NodeHeartbeatRequest {
        node_id: id,
        transport: request
            .transport
            .unwrap_or(crate::core::NodeTransportKind::Node),
        state: request.state.unwrap_or(crate::core::NodeState::Online),
        capabilities: request.capabilities,
        metrics: request.metrics,
        version: request.version.filter(|value| !value.trim().is_empty()),
        message: request.message.filter(|value| !value.trim().is_empty()),
    };
    match plane.heartbeat(payload).await {
        Ok(heartbeat) => {
            Json(serde_json::json!({ "status": "ok", "heartbeat": heartbeat })).into_response()
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

pub(super) async fn list_node_commands(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let plane = crate::core::NodeControlPlane::new(storage);
    match plane.list_commands(&id).await {
        Ok(commands) => {
            Json(serde_json::json!({ "status": "ok", "commands": commands })).into_response()
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

pub(super) async fn log_node_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<NodeCommandPayload>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let plane = crate::core::NodeControlPlane::new(storage);
    let payload = crate::core::NodeCommandLogRequest {
        node_id: id,
        command: request.command.trim().to_string(),
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
        success: request.success,
        exit_code: request.exit_code,
        output_preview: request
            .output_preview
            .filter(|value| !value.trim().is_empty()),
        actor: request.actor.filter(|value| !value.trim().is_empty()),
        context: request.context,
    };
    match plane.log_command(payload).await {
        Ok(command) => {
            Json(serde_json::json!({ "status": "ok", "command": command })).into_response()
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

async fn upsert_node_impl(
    state: AppState,
    path_id: Option<String>,
    request: UpsertNodeRequest,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let plane = crate::core::NodeControlPlane::new(storage);
    let id = path_id
        .or(request.id)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let base = request
                .name
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
                .split('-')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            if base.is_empty() {
                format!("node-{}", uuid::Uuid::new_v4())
            } else {
                format!("node-{}", base)
            }
        });
    let payload = crate::core::NodeUpsertRequest {
        id,
        display_name: request.name.trim().to_string(),
        transport: request
            .transport
            .unwrap_or(crate::core::NodeTransportKind::Node),
        state: request.state.unwrap_or(crate::core::NodeState::Paired),
        capabilities: request.capabilities,
        labels: request.labels,
        platform: request.platform.filter(|value| !value.trim().is_empty()),
        owner: request.owner.filter(|value| !value.trim().is_empty()),
        metadata: request.metadata,
    };
    match plane.upsert(payload).await {
        Ok(node) => Json(serde_json::json!({ "status": "ok", "node": node })).into_response(),
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
