use axum::{
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::BTreeMap;

use super::{AppState, ErrorResponse};

fn actor_label(
    maybe_caller: Option<&crate::actions::ActionCallerPrincipal>,
    fallback: &str,
) -> String {
    maybe_caller
        .map(|caller| format!("{}:{}", caller.auth_source, caller.user_id))
        .unwrap_or_else(|| fallback.to_string())
}

async fn plane_from_state(state: &AppState) -> crate::core::CompanionControlPlane {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    crate::core::CompanionControlPlane::new(storage)
}

fn json_error(status: axum::http::StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.into(),
        }),
    )
        .into_response()
}

#[derive(Debug, Clone)]
struct CompanionWsAuth {
    device_id: String,
    token: String,
}

fn companion_ws_secure(headers: &HeaderMap) -> bool {
    if crate::core::net::allow_insecure_local_transport() {
        return true;
    }
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or_default();
    if forwarded_proto.eq_ignore_ascii_case("https") || forwarded_proto.eq_ignore_ascii_case("wss")
    {
        return true;
    }
    headers
        .get("forwarded")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value.split(';').any(|part| {
                part.trim().eq_ignore_ascii_case("proto=https")
                    || part.trim().eq_ignore_ascii_case("proto=wss")
            })
        })
        .unwrap_or(false)
}

fn companion_ws_header_auth(headers: &HeaderMap) -> Option<CompanionWsAuth> {
    let device_id = headers
        .get("x-agentark-companion-device")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())?
        .trim();
    let (scheme, token) = auth.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(CompanionWsAuth {
        device_id,
        token: token.to_string(),
    })
}

pub(super) async fn get_presets() -> Response {
    Json(crate::core::companion_presets_response()).into_response()
}

pub(super) async fn get_protocol() -> Response {
    Json(crate::core::companion_protocol_document()).into_response()
}

pub(super) async fn list_devices(State(state): State<AppState>) -> Response {
    let plane = plane_from_state(&state).await;
    match (
        plane.list_devices().await,
        plane.overview().await,
        plane.list_pairing_sessions().await,
        plane.list_pending_approval_commands().await,
    ) {
        (Ok(devices), Ok(overview), Ok(pairing_sessions), Ok(pending_approvals)) => {
            Json(serde_json::json!({
                "status": "ok",
                "devices": devices,
                "overview": overview,
                "pairing_sessions": pairing_sessions,
                "pending_approvals": pending_approvals,
            }))
            .into_response()
        }
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => json_error(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            error.to_string(),
        ),
    }
}

pub(super) async fn create_pairing_session(
    State(state): State<AppState>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
    Json(input): Json<crate::core::CompanionPairingSessionCreate>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane.create_pairing_session(input, &actor).await {
        Ok(session) => Json(serde_json::json!({
            "status": "ok",
            "session": session,
            "pairing_payload": {
                "protocol_version": "agentark-companion-v1",
                "websocket_path": "/companion/ws",
                "session_id": session.id,
                "code": session.code,
                "expires_at": session.expires_at,
            }
        }))
        .into_response(),
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn approve_pairing_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane.approve_pairing_session(&id, &actor).await {
        Ok(session) => {
            Json(serde_json::json!({ "status": "ok", "session": session })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

async fn current_device_scopes(
    plane: &crate::core::CompanionControlPlane,
    device_id: &str,
) -> Result<Vec<String>, String> {
    let device = plane
        .get_device(device_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "device not found".to_string())?;
    Ok(device.token_capabilities)
}

pub(super) async fn create_command(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(input): Json<crate::core::CompanionCommandCreate>,
) -> Response {
    let plane = plane_from_state(&state).await;
    let caller_scopes = match current_device_scopes(&plane, &device_id).await {
        Ok(scopes) => scopes,
        Err(error) => return json_error(axum::http::StatusCode::NOT_FOUND, error),
    };
    match plane
        .create_command(&device_id, input, &caller_scopes)
        .await
    {
        Ok(command) => {
            Json(serde_json::json!({ "status": "ok", "command": command })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn list_commands(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Response {
    let plane = plane_from_state(&state).await;
    match plane.list_commands(&device_id).await {
        Ok(commands) => {
            Json(serde_json::json!({ "status": "ok", "commands": commands })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CommandApprovalPayload {
    #[serde(default = "default_true")]
    approved: bool,
    #[serde(default)]
    reason: Option<String>,
}

pub(super) async fn approve_command(
    State(state): State<AppState>,
    Path(command_id): Path<String>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
    Json(input): Json<CommandApprovalPayload>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane
        .approve_command(&command_id, &actor, input.approved, input.reason)
        .await
    {
        Ok(command) => {
            Json(serde_json::json!({ "status": "ok", "command": command })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn revoke_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane.revoke_device(&device_id, &actor).await {
        Ok(device) => Json(serde_json::json!({ "status": "ok", "device": device })).into_response(),
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn rotate_token(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(input): Json<crate::core::CompanionTokenRotationRequest>,
) -> Response {
    let plane = plane_from_state(&state).await;
    let caller_scopes = match current_device_scopes(&plane, &device_id).await {
        Ok(scopes) => scopes,
        Err(error) => return json_error(axum::http::StatusCode::NOT_FOUND, error),
    };
    match plane
        .rotate_token(&device_id, input.requested_scopes, &caller_scopes)
        .await
    {
        Ok(result) => {
            Json(serde_json::json!({ "status": "ok", "rotation": result })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) async fn get_audit(
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Response {
    let plane = plane_from_state(&state).await;
    match plane.list_audit_events(query.limit.unwrap_or(100)).await {
        Ok(events) => Json(serde_json::json!({ "status": "ok", "events": events })).into_response(),
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn companion_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !companion_ws_secure(&headers) {
        return json_error(
            StatusCode::UPGRADE_REQUIRED,
            "Companion WebSocket requires TLS in production.",
        );
    }
    let initial_auth = companion_ws_header_auth(&headers);
    ws.on_upgrade(move |socket| handle_companion_socket(state, socket, initial_auth))
}

#[derive(Debug, Deserialize)]
struct CompanionWsEnvelope {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    device_public_key: Option<String>,
    #[serde(default)]
    attestation: Option<crate::core::companion::CompanionAttestationClaim>,
    #[serde(default)]
    state: Option<crate::core::CompanionDeviceState>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    command_id: Option<String>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    result_preview: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

async fn send_ws_json(
    sender: &mut futures::stream::SplitSink<WebSocket, AxumWsMessage>,
    payload: serde_json::Value,
) -> bool {
    let Ok(text) = serde_json::to_string(&payload) else {
        return false;
    };
    sender.send(AxumWsMessage::Text(text.into())).await.is_ok()
}

async fn send_next_command(
    plane: &crate::core::CompanionControlPlane,
    sender: &mut futures::stream::SplitSink<WebSocket, AxumWsMessage>,
    device_id: &str,
) -> bool {
    match plane.dispatch_next_command(device_id).await {
        Ok(Some(command)) => {
            send_ws_json(
                sender,
                serde_json::json!({
                    "type": "command_dispatch",
                    "command": command,
                }),
            )
            .await
        }
        Ok(None) => true,
        Err(error) => {
            send_ws_json(
                sender,
                serde_json::json!({
                    "type": "error",
                    "error": error.to_string(),
                }),
            )
            .await
        }
    }
}

fn companion_ws_device_id_allowed(message_device_id: Option<&str>, authed_device_id: &str) -> bool {
    match message_device_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(message_device_id) => message_device_id == authed_device_id,
        None => true,
    }
}

async fn handle_companion_socket(
    state: AppState,
    socket: WebSocket,
    initial_auth: Option<CompanionWsAuth>,
) {
    let plane = plane_from_state(&state).await;
    let (mut sender, mut receiver) = socket.split();
    let mut authed_device_id: Option<String> = None;
    let _ = send_ws_json(
        &mut sender,
        serde_json::json!({
            "type": "hello",
            "protocol_version": "agentark-companion-v1",
            "pairing_required": true,
            "auth_transport": "authorization_header",
        }),
    )
    .await;
    if let Some(auth) = initial_auth {
        match plane
            .verify_device_token(&auth.device_id, &auth.token)
            .await
        {
            Ok(device) => {
                authed_device_id = Some(device.id.clone());
                let _ = plane
                    .pulse_device(
                        &device.id,
                        Some(crate::core::CompanionDeviceState::Online),
                        Vec::new(),
                        BTreeMap::new(),
                    )
                    .await;
                let _ = send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "auth_ok",
                        "device": device,
                    }),
                )
                .await;
                let _ = send_next_command(&plane, &mut sender, &auth.device_id).await;
            }
            Err(error) => {
                let _ = send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "auth_error",
                        "error": error.to_string(),
                    }),
                )
                .await;
            }
        }
    }

    while let Some(next) = receiver.next().await {
        let Ok(message) = next else {
            break;
        };
        let AxumWsMessage::Text(text) = message else {
            continue;
        };
        let parsed = match serde_json::from_str::<CompanionWsEnvelope>(&text) {
            Ok(parsed) => parsed,
            Err(error) => {
                if !send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "error",
                        "error": format!("invalid companion message: {error}"),
                    }),
                )
                .await
                {
                    break;
                }
                continue;
            }
        };

        match parsed.message_type.as_str() {
            "pairing_claim" => {
                let claim = crate::core::CompanionPairingClaim {
                    session_id: parsed.session_id.unwrap_or_default(),
                    code: parsed.code.unwrap_or_default(),
                    device_public_key: parsed.device_public_key,
                    attestation: parsed.attestation,
                    metadata: parsed.metadata,
                };
                match plane.claim_pairing_session(claim).await {
                    Ok(result) => {
                        if let Some(device) = result.device.as_ref() {
                            authed_device_id = Some(device.id.clone());
                        }
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "pairing_claim_result",
                                "result": result,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        if let Some(device_id) = authed_device_id.clone() {
                            if !send_next_command(&plane, &mut sender, &device_id).await {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            "auth" => {
                if parsed.token.is_some() {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_error",
                            "error": "send companion tokens in the WebSocket Authorization header, not in JSON messages",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                let Some(device_id) = authed_device_id.clone() else {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_error",
                            "error": "missing WebSocket Authorization header",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                };
                if !companion_ws_device_id_allowed(parsed.device_id.as_deref(), &device_id) {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_error",
                            "error": "message device_id does not match the authenticated device",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                if let Some(device) = plane.get_device(&device_id).await.ok().flatten() {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_ok",
                            "device": device,
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    if !send_next_command(&plane, &mut sender, &device_id).await {
                        break;
                    }
                }
            }
            "pulse" | "capability_report" => {
                let Some(device_id) = authed_device_id.clone() else {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "authenticate before pulse or capability_report",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                };
                if !companion_ws_device_id_allowed(parsed.device_id.as_deref(), &device_id) {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "message device_id does not match the authenticated device",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                match plane
                    .pulse_device(
                        &device_id,
                        parsed.state,
                        parsed.capabilities,
                        parsed.metadata,
                    )
                    .await
                {
                    Ok(device) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "pulse_ok",
                                "device": device,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        if !send_next_command(&plane, &mut sender, &device_id).await {
                            break;
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            "command_result" => {
                let Some(device_id) = authed_device_id.clone() else {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "authenticate before command_result",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                };
                if !companion_ws_device_id_allowed(parsed.device_id.as_deref(), &device_id) {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "message device_id does not match the authenticated device",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                match plane
                    .complete_command(
                        &device_id,
                        parsed.command_id.as_deref().unwrap_or_default(),
                        parsed.success.unwrap_or(false),
                        parsed.result_preview,
                        parsed.error,
                    )
                    .await
                {
                    Ok(command) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "command_result_ok",
                                "command": command,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            _ => {
                if !send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "error",
                        "error": "unsupported companion protocol message type",
                    }),
                )
                .await
                {
                    break;
                }
            }
        }
    }
}

fn default_true() -> bool {
    true
}
