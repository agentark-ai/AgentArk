use super::*;

#[derive(Debug, Deserialize, Default)]
pub(super) struct CreateVoiceSessionRequest {
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    transport: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct VoiceTurnRequest {
    transcript: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct VoiceStreamQuery {
    #[serde(default)]
    stream_token: Option<String>,
}

async fn fetch_voice_bridge_status(bridge_url: &str) -> Result<serde_json::Value, String> {
    let url = format!("{}/status", bridge_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1800))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("voice bridge returned {}", response.status()));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())
}

fn setup_error(code: &str, message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "message": message.into(),
    })
}

async fn voice_status_payload() -> serde_json::Value {
    let config = crate::core::voice_runtime_config_from_current_env();
    let disabled_reason = config
        .disabled_reason
        .clone()
        .unwrap_or_else(|| "voice_bridge_unavailable".to_string());
    let setup_message = if config.enabled {
        "Configured voice bridge is not reachable. Voice is a future opt-in capability and is not part of the default install."
    } else {
        "Voice is not enabled in this build. Two-way local voice is planned as a future opt-in capability."
    };
    let mut payload = serde_json::json!({
        "status": if config.enabled { "unavailable" } else { "disabled" },
        "voice_available": false,
        "bridge_url": config.bridge_url,
        "disabled_reason": disabled_reason.clone(),
        "session": null,
        "transport": ["browser", "browser_websocket"],
        "engine": "pipecat",
        "setup_errors": [setup_error(
            &disabled_reason,
            setup_message,
        )],
    });
    let Some(bridge_url) = config.bridge_url.as_deref() else {
        return payload;
    };
    match fetch_voice_bridge_status(bridge_url).await {
        Ok(bridge) => {
            let ready = bridge
                .as_object()
                .and_then(|object| object.get("status"))
                .and_then(serde_json::Value::as_str)
                == Some("ready");
            if let serde_json::Value::Object(map) = &mut payload {
                map.insert(
                    "status".to_string(),
                    bridge
                        .as_object()
                        .and_then(|object| object.get("status"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!("setup_needed")),
                );
                map.insert("voice_available".to_string(), serde_json::json!(ready));
                for key in [
                    "transport",
                    "engine",
                    "stream_path",
                    "stt",
                    "tts",
                    "setup_errors",
                ] {
                    if let Some(value) = bridge.as_object().and_then(|object| object.get(key)) {
                        map.insert(key.to_string(), value.clone());
                    }
                }
                if ready {
                    map.insert("disabled_reason".to_string(), serde_json::Value::Null);
                } else {
                    map.insert(
                        "disabled_reason".to_string(),
                        bridge
                            .as_object()
                            .and_then(|object| object.get("disabled_reason"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!("voice_assets_missing")),
                    );
                }
            }
        }
        Err(error) => {
            if let serde_json::Value::Object(map) = &mut payload {
                map.insert(
                    "setup_errors".to_string(),
                    serde_json::json!([setup_error(
                        "voice_bridge_unavailable",
                        format!(
                            "Configured voice bridge is not reachable: {error}. Voice is a future opt-in capability and is not part of the default install."
                        ),
                    )]),
                );
            }
        }
    }
    payload
}

pub(super) async fn get_voice_status(State(state): State<AppState>) -> Response {
    let mut payload = voice_status_payload().await;
    if let serde_json::Value::Object(map) = &mut payload {
        let session = state.voice_sessions.read().await.latest_active();
        map.insert("session".to_string(), serde_json::json!(session));
    }
    Json(payload).into_response()
}

pub(super) async fn create_voice_session(
    State(state): State<AppState>,
    Json(request): Json<CreateVoiceSessionRequest>,
) -> Response {
    let readiness = voice_status_payload().await;
    let voice_available = readiness
        .as_object()
        .and_then(|object| object.get("voice_available"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !voice_available {
        return (StatusCode::CONFLICT, Json(readiness)).into_response();
    }
    let transport = request
        .transport
        .as_deref()
        .unwrap_or(crate::core::voice::VOICE_TRANSPORT_BROWSER)
        .trim();
    if transport != crate::core::voice::VOICE_TRANSPORT_BROWSER
        && transport != crate::core::voice::VOICE_TRANSPORT_BROWSER_WEBSOCKET
    {
        return error_response(StatusCode::BAD_REQUEST, "Unsupported voice transport");
    }
    let conversation_id = request.conversation_id;
    let (session, stream_token) = {
        let mut sessions = state.voice_sessions.write().await;
        let session = if transport == crate::core::voice::VOICE_TRANSPORT_BROWSER_WEBSOCKET {
            conversation_id
                .as_deref()
                .and_then(|id| sessions.active_stream_for_conversation(id))
                .unwrap_or_else(|| sessions.start_browser_stream_session(conversation_id))
        } else {
            conversation_id
                .as_deref()
                .and_then(|id| sessions.active_for_conversation(id))
                .unwrap_or_else(|| sessions.start_browser_session(conversation_id))
        };
        let stream_token = sessions.stream_token_for_session(&session.id);
        (session, stream_token)
    };
    Json(serde_json::json!({
        "status": "ok",
        "session": session,
        "stream_token": stream_token,
    }))
    .into_response()
}

pub(super) async fn stream_voice_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<VoiceStreamQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let config = crate::core::voice_runtime_config_from_current_env();
    let Some(bridge_url) = config.bridge_url.as_deref() else {
        return error_response(StatusCode::CONFLICT, "Voice bridge URL is not configured");
    };
    if state.voice_sessions.read().await.get(&id).is_none() {
        return error_response(StatusCode::NOT_FOUND, "Voice session not found");
    }
    let stream_token = query.stream_token.as_deref().unwrap_or_default();
    if !state
        .voice_sessions
        .read()
        .await
        .stream_token_matches(&id, stream_token)
    {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid voice stream token");
    }
    let stream_url = match crate::core::voice::voice_bridge_stream_url(bridge_url, &id) {
        Ok(url) => url,
        Err(error) => return error_response(StatusCode::BAD_GATEWAY, error),
    };
    ws.on_upgrade(move |socket| handle_voice_stream_socket(state, id, stream_url, socket))
}

pub(super) async fn stop_voice_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let stopped = state.voice_sessions.write().await.stop(&id);
    match stopped {
        Some(session) => {
            Json(serde_json::json!({ "status": "ok", "session": session })).into_response()
        }
        None => error_response(StatusCode::NOT_FOUND, "Voice session not found"),
    }
}

pub(super) async fn submit_voice_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<VoiceTurnRequest>,
) -> Response {
    let transcript = request.transcript.trim();
    if transcript.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "transcript is required");
    }
    let session = {
        let mut sessions = state.voice_sessions.write().await;
        match sessions.set_phase(&id, crate::core::VoiceSessionPhase::Thinking) {
            Some(session) => session,
            None => return error_response(StatusCode::NOT_FOUND, "Voice session not found"),
        }
    };
    let agent = Agent::snapshot(&state.agent).await;
    let processed = agent
        .process_message_with_meta(
            transcript,
            "voice",
            session.conversation_id.as_deref(),
            None,
        )
        .await;
    match processed {
        Ok(processed) => {
            let updated = {
                let mut sessions = state.voice_sessions.write().await;
                sessions.set_conversation_id(&id, processed.conversation_id.clone());
                sessions.set_phase(&id, crate::core::VoiceSessionPhase::Speaking)
            };
            Json(serde_json::json!({
                "status": "ok",
                "session": updated,
                "conversation_id": processed.conversation_id,
                "assistant_text": processed.response,
                "trace_id": processed.trace_id,
            }))
            .into_response()
        }
        Err(error) => {
            let updated = state
                .voice_sessions
                .write()
                .await
                .set_error(&id, error.to_string());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "session": updated,
                    "error": error.to_string(),
                })),
            )
                .into_response()
        }
    }
}

fn axum_to_tungstenite_message(msg: AxumWsMessage) -> Option<TungsteniteMessage> {
    match msg {
        AxumWsMessage::Text(text) => Some(TungsteniteMessage::Text(text.to_string().into())),
        AxumWsMessage::Binary(data) => Some(TungsteniteMessage::Binary(data)),
        AxumWsMessage::Ping(data) => Some(TungsteniteMessage::Ping(data)),
        AxumWsMessage::Pong(data) => Some(TungsteniteMessage::Pong(data)),
        AxumWsMessage::Close(_) => Some(TungsteniteMessage::Close(None)),
    }
}

fn tungstenite_to_axum_message(msg: TungsteniteMessage) -> Option<AxumWsMessage> {
    match msg {
        TungsteniteMessage::Text(text) => Some(AxumWsMessage::Text(text.to_string().into())),
        TungsteniteMessage::Binary(data) => Some(AxumWsMessage::Binary(data)),
        TungsteniteMessage::Ping(data) => Some(AxumWsMessage::Ping(data)),
        TungsteniteMessage::Pong(data) => Some(AxumWsMessage::Pong(data)),
        TungsteniteMessage::Close(_) => Some(AxumWsMessage::Close(None)),
        TungsteniteMessage::Frame(_) => None,
    }
}

fn bridge_event_type(value: &serde_json::Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(serde_json::Value::as_str)
}

async fn handle_voice_stream_socket(
    state: AppState,
    session_id: String,
    stream_url: url::Url,
    client_socket: WebSocket,
) {
    let mut upstream_request = match stream_url.as_str().into_client_request() {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!("Failed to build voice bridge WS request: {}", error);
            return;
        }
    };
    upstream_request.headers_mut().insert(
        "x-agentark-voice-session",
        HeaderValue::from_str(&session_id).unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    let (bridge_socket, _) = match tokio_tungstenite::connect_async(upstream_request).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!("Failed to connect to voice bridge WS: {}", error);
            return;
        }
    };

    let (mut client_sender, mut client_receiver) = client_socket.split();
    let (mut bridge_sender, mut bridge_receiver) = bridge_socket.split();
    let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<TungsteniteMessage>(64);

    let bridge_writer = async move {
        while let Some(message) = bridge_rx.recv().await {
            if bridge_sender.send(message).await.is_err() {
                break;
            }
        }
        let _ = bridge_sender.close().await;
    };

    let client_to_bridge_tx = bridge_tx.clone();
    let client_to_bridge = async move {
        while let Some(result) = client_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(bridge_message) = axum_to_tungstenite_message(message) else {
                        continue;
                    };
                    if client_to_bridge_tx.send(bridge_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Voice client WS receive error: {}", error);
                    break;
                }
            }
        }
    };

    let bridge_to_client_state = state.clone();
    let bridge_to_client_session_id = session_id.clone();
    let bridge_to_client = async move {
        while let Some(result) = bridge_receiver.next().await {
            let message = match result {
                Ok(message) => message,
                Err(error) => {
                    tracing::debug!("Voice bridge WS receive error: {}", error);
                    break;
                }
            };
            if let TungsteniteMessage::Text(text) = &message {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
                    if bridge_event_type(&value) == Some("session.listening")
                        || bridge_event_type(&value) == Some("transcript.final")
                    {
                        bridge_to_client_state
                            .voice_sessions
                            .write()
                            .await
                            .set_phase(
                                &bridge_to_client_session_id,
                                crate::core::VoiceSessionPhase::Listening,
                            );
                    }
                }
            }
            let Some(client_message) = tungstenite_to_axum_message(message) else {
                continue;
            };
            if client_sender.send(client_message).await.is_err() {
                break;
            }
        }
        let _ = client_sender.send(AxumWsMessage::Close(None)).await;
    };

    tokio::select! {
        _ = bridge_writer => {}
        _ = client_to_bridge => {}
        _ = bridge_to_client => {}
    }
}
