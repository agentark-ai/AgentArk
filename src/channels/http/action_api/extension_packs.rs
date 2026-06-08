use super::*;

#[derive(Debug, Deserialize, Default)]
pub(super) struct ExtensionPackQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExtensionPackEnabledRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ExtensionPackDeleteQuery {
    #[serde(default)]
    pub remove_connections: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ExtensionPackConnectUrlQuery {
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ExtensionPackEventsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
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

async fn sync_extension_pack_runtime(
    state: &AppState,
    registry: &std::sync::Arc<tokio::sync::RwLock<crate::extension_packs::ExtensionPackRegistry>>,
) -> Option<String> {
    let (runtime, agent_for_catalog) = {
        let agent = state.agent.read().await;
        (agent.runtime.clone(), agent.clone())
    };
    let guard = registry.read().await;
    let warning = guard
        .sync_to_runtime(&runtime)
        .await
        .err()
        .map(|e| e.to_string());
    drop(guard);
    if warning.is_none() {
        agent_for_catalog
            .refresh_action_catalog_index("extension_pack_runtime_sync")
            .await;
    }
    warning
}

fn header_snapshot(headers: &HeaderMap, names: &[&str]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for name in names {
        if let Some(value) = headers.get(*name).and_then(|value| value.to_str().ok()) {
            map.insert(
                (*name).to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
    serde_json::Value::Object(map)
}

fn bridge_auth_matches(headers: &HeaderMap, expected: &str) -> bool {
    if expected.trim().is_empty() {
        return false;
    }
    auth::has_valid_bearer_api_key(headers, Some(expected))
        || headers
            .get("x-agentark-bridge-token")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.trim() == expected)
}

pub(super) async fn list_extension_packs(
    State(state): State<AppState>,
    Query(query): Query<ExtensionPackQuery>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let guard = registry.read().await;
    match guard
        .search_packs(query.query.as_deref(), query.kind.as_deref())
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn get_extension_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let guard = registry.read().await;
    match guard.get_pack(pack_id.as_str()).await {
        Ok(Some(pack)) => match guard.list_connections(pack_id.as_str()).await {
            Ok(connections) => Json(serde_json::json!({
                "pack": pack,
                "connections": connections,
            }))
            .into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        },
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Pack not found"),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn list_extension_pack_events(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Query(query): Query<ExtensionPackEventsQuery>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let guard = registry.read().await;
    match guard
        .list_events(pack_id.as_str(), query.limit.unwrap_or(25))
        .await
    {
        Ok(events) => Json(events).into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn install_extension_pack(
    State(state): State<AppState>,
    Json(request): Json<crate::extension_packs::ExtensionPackInstallRequest>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.install(request).await
    };
    match result {
        Ok(pack) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "pack": pack, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn upload_extension_pack(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let mut trust_unverified = false;
    let mut filename: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;
    loop {
        let next = match multipart.next_field().await {
            Ok(value) => value,
            Err(error) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Failed to read upload: {}", error),
                );
            }
        };
        let Some(field) = next else {
            break;
        };
        let name = field.name().unwrap_or_default().to_string();
        if name == "trust_unverified" {
            match field.text().await {
                Ok(value) => {
                    trust_unverified = !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "false" | "0" | "no"
                    );
                }
                Err(error) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        format!("Failed to read trust_unverified: {}", error),
                    );
                }
            }
            continue;
        }
        if name == "file" {
            filename = field.file_name().map(|value| value.to_string());
            match field.bytes().await {
                Ok(value) => bytes = Some(value.to_vec()),
                Err(error) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        format!("Failed to read uploaded file: {}", error),
                    );
                }
            }
        }
    }
    let Some(bytes) = bytes else {
        return error_response(StatusCode::BAD_REQUEST, "Missing file field");
    };
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard
            .install_uploaded_bundle(filename.as_deref(), &bytes, trust_unverified)
            .await
    };
    match result {
        Ok(pack) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "pack": pack, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn scaffold_extension_pack(
    State(state): State<AppState>,
    Json(request): Json<crate::extension_packs::ExtensionPackScaffoldRequest>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.scaffold(request).await
    };
    match result {
        Ok(pack) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "pack": pack, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn list_extension_pack_connections(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let guard = registry.read().await;
    match guard.list_connections(pack_id.as_str()).await {
        Ok(connections) => Json(serde_json::json!({
            "connections": connections,
            "count": connections.len(),
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn upsert_extension_pack_connection(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(request): Json<crate::extension_packs::ExtensionPackConnectionUpsertRequest>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.upsert_connection(pack_id.as_str(), request).await
    };
    match result {
        Ok(connection) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({
                "status": "ok",
                "connection": connection,
                "warning": warning,
            }))
            .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn get_extension_pack_connect_url(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Query(query): Query<ExtensionPackConnectUrlQuery>,
    headers: HeaderMap,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let requested_redirect_uri =
        match oauth_redirect_uri_for_request(&state, &headers, query.redirect_uri.as_deref()) {
            Ok(value) => value,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        };
    let supports_connect_url = {
        let guard = registry.read().await;
        guard.supports_connect_url(pack_id.as_str())
    };
    if !supports_connect_url {
        return error_response(
            StatusCode::BAD_REQUEST,
            "This pack does not expose a browser connect URL",
        );
    }
    let redirect_uri = if pack_id.eq_ignore_ascii_case("google_workspace") {
        requested_redirect_uri.clone()
    } else {
        let guard = registry.read().await;
        match guard.connect_redirect_uri(pack_id.as_str(), &requested_redirect_uri) {
            Ok(value) => value,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        }
    };

    let (state_token, code_challenge) = if pack_id.eq_ignore_ascii_case("google_workspace") {
        auth::issue_oauth_state_with_pkce(&state, pack_id.as_str(), Some(redirect_uri.clone()))
            .await
    } else {
        let profile_id = {
            let mut guard = registry.write().await;
            match guard.ensure_connect_auth_profile(pack_id.as_str()).await {
                Ok(Some(profile_id)) => profile_id,
                Ok(None) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "This pack does not expose a browser connect URL",
                    );
                }
                Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
            }
        };
        auth::issue_auth_profile_oauth_state_with_pkce(
            &state,
            &profile_id,
            Some(redirect_uri.clone()),
        )
        .await
    };

    let result = {
        let mut guard = registry.write().await;
        guard
            .build_connect_url(
                pack_id.as_str(),
                &redirect_uri,
                &state_token,
                &code_challenge,
            )
            .await
    };
    match result {
        Ok(url) => Json(serde_json::json!({
            "auth_url": url,
            "url": url,
            "redirect_uri": redirect_uri,
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn verify_extension_pack_webhook(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let resolved = {
        let guard = registry.read().await;
        match guard.resolve_webhook_binding(pack_id.as_str()).await {
            Ok(value) => value,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        }
    };
    let mut guard = registry.write().await;
    let event = match guard
        .record_event_received(
            pack_id.as_str(),
            resolved.feature.id.as_str(),
            resolved.connection_id.as_deref(),
            "webhook_get",
            "webhook_verification",
            None,
            serde_json::json!({
                "provider": pack_id,
                "mode": params.get("hub.mode").cloned(),
                "has_challenge": params.contains_key("hub.challenge"),
            }),
            serde_json::Value::Null,
            "received",
            None,
            None,
        )
        .await
    {
        Ok(event) => event,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    drop(guard);

    if !resolved
        .manifest
        .id
        .eq_ignore_ascii_case("whatsapp_channel")
    {
        let mut guard = registry.write().await;
        let _ = guard
            .finish_event(
                event.id.as_str(),
                "error",
                Some("This pack does not support GET webhook verification."),
                None,
            )
            .await;
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "This pack does not support GET webhook verification",
        );
    }
    let Some(secret) = resolved.secret.as_ref() else {
        let mut guard = registry.write().await;
        let _ = guard
            .finish_event(
                event.id.as_str(),
                "error",
                Some("Pack connection is missing secret configuration."),
                None,
            )
            .await;
        return error_response(
            StatusCode::FORBIDDEN,
            "Pack connection is missing secret configuration",
        );
    };
    let config =
        match crate::extension_packs::whatsapp_config_from_secret(&serde_json::Value::Null, secret)
        {
            Ok(value) => value,
            Err(error) => {
                let mut guard = registry.write().await;
                let _ = guard
                    .finish_event(event.id.as_str(), "error", Some(&error.to_string()), None)
                    .await;
                return error_response(StatusCode::BAD_REQUEST, error);
            }
        };
    match crate::channels::whatsapp::verify_webhook(&params, &config.verify_token).await {
        Ok(challenge) => {
            let mut guard = registry.write().await;
            let _ = guard
                .finish_event(
                    event.id.as_str(),
                    "processed",
                    Some("verification_ok"),
                    Some(&challenge),
                )
                .await;
            challenge.into_response()
        }
        Err(error) => {
            let mut guard = registry.write().await;
            let _ = guard
                .finish_event(event.id.as_str(), "error", Some(&error.to_string()), None)
                .await;
            error_response(StatusCode::FORBIDDEN, error)
        }
    }
}

pub(super) async fn handle_extension_pack_webhook(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    request: Request<axum::body::Body>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let resolved = {
        let guard = registry.read().await;
        match guard.resolve_webhook_binding(pack_id.as_str()).await {
            Ok(value) => value,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        }
    };
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let body_json = serde_json::from_slice::<serde_json::Value>(&body_bytes).ok();

    match resolved.manifest.id.as_str() {
        "slack_channel" => {
            let Some(secret) = resolved.secret.as_ref() else {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "Pack connection is missing secret configuration",
                );
            };
            let config = match crate::extension_packs::slack_config_from_secret(
                &serde_json::Value::Null,
                secret,
            ) {
                Ok(value) => value,
                Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
            };
            let timestamp = parts
                .headers
                .get("x-slack-request-timestamp")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string());
            let signature = parts
                .headers
                .get("x-slack-signature")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string());
            let payload = body_json.clone().unwrap_or_else(|| {
                serde_json::Value::String(String::from_utf8_lossy(&body_bytes).to_string())
            });
            let event_type = body_json
                .as_ref()
                .and_then(|value| value.get("type"))
                .and_then(|value| value.as_str())
                .unwrap_or("slack_event");
            let provider_event_id = body_json
                .as_ref()
                .and_then(|value| value.get("event_id"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            let metadata = header_snapshot(
                &parts.headers,
                &["content-type", "user-agent", "x-slack-request-timestamp"],
            );
            let event = {
                let mut guard = registry.write().await;
                match guard
                    .record_event_received(
                        pack_id.as_str(),
                        resolved.feature.id.as_str(),
                        resolved.connection_id.as_deref(),
                        "webhook_post",
                        event_type,
                        provider_event_id.as_deref(),
                        metadata,
                        payload,
                        "received",
                        None,
                        None,
                    )
                    .await
                {
                    Ok(event) => event,
                    Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
                }
            };
            if let Err(error) = crate::channels::slack::verify_webhook_request_for_config(
                &config,
                &body_bytes,
                timestamp.as_deref(),
                signature.as_deref(),
            ) {
                let mut guard = registry.write().await;
                let _ = guard
                    .finish_event(event.id.as_str(), "error", Some(&error.to_string()), None)
                    .await;
                return error_response(StatusCode::BAD_REQUEST, error);
            }
            let is_url_verification = body_json
                .as_ref()
                .and_then(|value| value.get("type"))
                .and_then(|value| value.as_str())
                .is_some_and(|value| value == "url_verification");
            if is_url_verification {
                return match crate::channels::slack::handle_webhook_with_config(
                    state.agent.clone(),
                    Some(&config),
                    &body_bytes,
                    timestamp.as_deref(),
                    signature.as_deref(),
                )
                .await
                {
                    Ok(response) => {
                        let mut guard = registry.write().await;
                        let _ = guard
                            .finish_event(
                                event.id.as_str(),
                                "processed",
                                Some("url_verification"),
                                Some(&response),
                            )
                            .await;
                        (StatusCode::OK, response).into_response()
                    }
                    Err(error) => {
                        let mut guard = registry.write().await;
                        let _ = guard
                            .finish_event(
                                event.id.as_str(),
                                "error",
                                Some(&error.to_string()),
                                None,
                            )
                            .await;
                        error_response(StatusCode::BAD_REQUEST, error)
                    }
                };
            }
            let registry_clone = registry.clone();
            let agent = state.agent.clone();
            let event_id = event.id.clone();
            crate::spawn_logged!("src/channels/http/extension_packs.rs:566", async move {
                let outcome = crate::channels::slack::handle_webhook_with_config(
                    agent,
                    Some(&config),
                    &body_bytes,
                    timestamp.as_deref(),
                    signature.as_deref(),
                )
                .await;
                let (status, detail, preview) = match outcome {
                    Ok(response) => ("processed", Some("ok".to_string()), Some(response)),
                    Err(error) => ("error", Some(error.to_string()), None),
                };
                let mut guard = registry_clone.write().await;
                let _ = guard
                    .finish_event(
                        event_id.as_str(),
                        status,
                        detail.as_deref(),
                        preview.as_deref(),
                    )
                    .await;
            });
            (StatusCode::OK, "ok").into_response()
        }
        "teams_channel" => {
            let Some(secret) = resolved.secret.as_ref() else {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "Pack connection is missing secret configuration",
                );
            };
            let config = match crate::extension_packs::teams_config_from_secret(
                &serde_json::Value::Null,
                secret,
            ) {
                Ok(value) => value,
                Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
            };
            let activity = match serde_json::from_slice::<crate::channels::teams::TeamsActivity>(
                &body_bytes,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        format!("Invalid Teams activity payload: {}", error),
                    );
                }
            };
            let metadata = header_snapshot(&parts.headers, &["content-type", "user-agent"]);
            let event = {
                let mut guard = registry.write().await;
                match guard
                    .record_event_received(
                        pack_id.as_str(),
                        resolved.feature.id.as_str(),
                        resolved.connection_id.as_deref(),
                        "webhook_post",
                        activity.activity_type.as_str(),
                        activity.id.as_deref(),
                        metadata,
                        body_json.clone().unwrap_or(serde_json::Value::Null),
                        "received",
                        None,
                        None,
                    )
                    .await
                {
                    Ok(event) => event,
                    Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
                }
            };
            let authorization = parts
                .headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string());
            let verified = match crate::channels::teams::verify_inbound_activity_request(
                &config,
                authorization.as_deref(),
                &activity,
            )
            .await
            {
                Ok(value) => value,
                Err(error) => {
                    let mut guard = registry.write().await;
                    let _ = guard
                        .finish_event(event.id.as_str(), "error", Some(&error.to_string()), None)
                        .await;
                    return error_response(StatusCode::FORBIDDEN, error);
                }
            };
            let registry_clone = registry.clone();
            let agent = state.agent.clone();
            let event_id = event.id.clone();
            crate::spawn_logged!("src/channels/http/extension_packs.rs:663", async move {
                let outcome =
                    crate::channels::teams::handle_activity(&agent, &config, activity, verified)
                        .await;
                let (status, detail, preview) = match outcome {
                    Ok(summary) => (
                        if summary.processed {
                            "processed"
                        } else {
                            "ignored"
                        },
                        Some(if summary.processed { "ok" } else { "ignored" }.to_string()),
                        summary.response_preview,
                    ),
                    Err(error) => ("error", Some(error.to_string()), None),
                };
                let mut guard = registry_clone.write().await;
                let _ = guard
                    .finish_event(
                        event_id.as_str(),
                        status,
                        detail.as_deref(),
                        preview.as_deref(),
                    )
                    .await;
            });
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "accepted" })),
            )
                .into_response()
        }
        "whatsapp_channel" => {
            let Some(secret) = resolved.secret.as_ref() else {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "Pack connection is missing secret configuration",
                );
            };
            let config = match crate::extension_packs::whatsapp_config_from_secret(
                &serde_json::Value::Null,
                secret,
            ) {
                Ok(value) => value,
                Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
            };
            let payload = match body_json {
                Some(value) => value,
                None => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "Failed to parse WhatsApp webhook payload",
                    );
                }
            };
            let is_baileys =
                payload.get("_source").and_then(|value| value.as_str()) == Some("baileys");
            let provider_event_id = payload
                .get("entry")
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("changes"))
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("messages"))
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            let event_type = payload
                .get("entry")
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("changes"))
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("messages"))
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("type"))
                .and_then(|value| value.as_str())
                .unwrap_or(if is_baileys {
                    "baileys_event"
                } else {
                    "whatsapp_event"
                });
            let metadata = header_snapshot(
                &parts.headers,
                &["content-type", "user-agent", "x-hub-signature-256"],
            );
            let event = {
                let mut guard = registry.write().await;
                match guard
                    .record_event_received(
                        pack_id.as_str(),
                        resolved.feature.id.as_str(),
                        resolved.connection_id.as_deref(),
                        "webhook_post",
                        event_type,
                        provider_event_id.as_deref(),
                        metadata,
                        payload.clone(),
                        "received",
                        None,
                        None,
                    )
                    .await
                {
                    Ok(event) => event,
                    Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
                }
            };
            if is_baileys {
                if config.mode != crate::channels::whatsapp::WhatsAppMode::Baileys {
                    let message =
                        "WhatsApp bridge payload rejected because Cloud API mode is configured";
                    let mut guard = registry.write().await;
                    let _ = guard
                        .finish_event(event.id.as_str(), "error", Some(message), None)
                        .await;
                    return error_response(StatusCode::FORBIDDEN, message);
                }
                let expected = if !config.bridge_token.trim().is_empty() {
                    config.bridge_token.trim().to_string()
                } else {
                    state.api_key.read().await.clone().unwrap_or_default()
                };
                if expected.trim().is_empty() || !bridge_auth_matches(&parts.headers, &expected) {
                    let message = "WhatsApp bridge authorization failed";
                    let mut guard = registry.write().await;
                    let _ = guard
                        .finish_event(event.id.as_str(), "error", Some(message), None)
                        .await;
                    return error_response(StatusCode::UNAUTHORIZED, message);
                }
            } else if let Err(error) = crate::channels::whatsapp::verify_cloud_api_request_signature(
                &config,
                &body_bytes,
                parts
                    .headers
                    .get("x-hub-signature-256")
                    .and_then(|value| value.to_str().ok()),
            ) {
                let mut guard = registry.write().await;
                let _ = guard
                    .finish_event(event.id.as_str(), "error", Some(&error.to_string()), None)
                    .await;
                return error_response(StatusCode::BAD_REQUEST, error);
            }
            let registry_clone = registry.clone();
            let agent = state.agent.clone();
            let event_id = event.id.clone();
            crate::spawn_logged!("src/channels/http/extension_packs.rs:812", async move {
                let outcome =
                    crate::channels::whatsapp::handle_webhook_with_config(agent, &config, &payload)
                        .await;
                let (status, detail, preview) = match outcome {
                    Ok(response) => ("processed", Some("ok".to_string()), Some(response)),
                    Err(error) => ("error", Some(error.to_string()), None),
                };
                let mut guard = registry_clone.write().await;
                let _ = guard
                    .finish_event(
                        event_id.as_str(),
                        status,
                        detail.as_deref(),
                        preview.as_deref(),
                    )
                    .await;
            });
            StatusCode::OK.into_response()
        }
        _ => error_response(
            StatusCode::BAD_REQUEST,
            "This pack does not expose a generic webhook runtime",
        ),
    }
}

pub(super) async fn test_extension_pack_connection(
    State(state): State<AppState>,
    Path((pack_id, connection_id)): Path<(String, String)>,
) -> Response {
    let (registry, mcp, plugins) = {
        let agent = state.agent.read().await;
        (
            agent.extension_packs.clone(),
            agent.mcp.clone(),
            agent.plugins.clone(),
        )
    };
    let mut guard = registry.write().await;
    match guard
        .test_connection(
            pack_id.as_str(),
            connection_id.as_str(),
            Some(mcp),
            Some(plugins),
        )
        .await
    {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "result": result })).into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn set_extension_pack_enabled(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(request): Json<ExtensionPackEnabledRequest>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard
            .set_pack_enabled(pack_id.as_str(), request.enabled)
            .await
    };
    match result {
        Ok(pack) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "pack": pack, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn delete_extension_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Query(query): Query<ExtensionPackDeleteQuery>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard
            .delete_pack(pack_id.as_str(), query.remove_connections.unwrap_or(true))
            .await
    };
    match result {
        Ok(()) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "warning": warning })).into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn install_extension_pack_runtime(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.install_runtime(pack_id.as_str()).await
    };
    match result {
        Ok(runtime) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "result": runtime, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn verify_extension_pack_runtime(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.verify_runtime(pack_id.as_str()).await
    };
    match result {
        Ok(runtime) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "result": runtime, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn update_extension_pack_runtime(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.update_runtime(pack_id.as_str()).await
    };
    match result {
        Ok(runtime) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "result": runtime, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn uninstall_extension_pack_runtime(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Response {
    let registry = {
        let agent = state.agent.read().await;
        agent.extension_packs.clone()
    };
    let result = {
        let mut guard = registry.write().await;
        guard.uninstall_runtime(pack_id.as_str()).await
    };
    match result {
        Ok(runtime) => {
            let warning = sync_extension_pack_runtime(&state, &registry).await;
            Json(serde_json::json!({ "status": "ok", "result": runtime, "warning": warning }))
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn invoke_extension_pack_feature(
    State(state): State<AppState>,
    Json(request): Json<crate::extension_packs::ExtensionPackInvokeRequest>,
) -> Response {
    let (registry, mcp, plugins) = {
        let agent = state.agent.read().await;
        (
            agent.extension_packs.clone(),
            agent.mcp.clone(),
            agent.plugins.clone(),
        )
    };
    let mut guard = registry.write().await;
    match guard
        .invoke_feature(request, Some(mcp), Some(plugins))
        .await
    {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "result": result })).into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}
