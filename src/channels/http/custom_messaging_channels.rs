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

fn caller_actor_label(
    maybe_caller: Option<&Extension<crate::actions::ActionCallerPrincipal>>,
) -> String {
    maybe_caller
        .map(|Extension(caller)| {
            format!(
                "user_id={},role={},source={}",
                caller.user_id, caller.role, caller.auth_source
            )
        })
        .unwrap_or_else(|| "local_ui".to_string())
}

fn audit_custom_channel_event(
    state: &AppState,
    event_type: &str,
    severity: &str,
    actor: &str,
    channel_id: &str,
    message: &str,
) {
    spawn_security_log(
        state.agent.clone(),
        event_type,
        severity,
        format!("{} actor={} channel_id={}", message, actor, channel_id),
        Some(format!("actor={};channel_id={}", actor, channel_id)),
    );
}

fn audit_custom_channel_capability_review(
    state: &AppState,
    channel: &crate::custom_messaging_channels::CustomMessagingChannelView,
) {
    let mut capabilities = vec![
        "calls-network".to_string(),
        "sends-message".to_string(),
        "sends-external".to_string(),
    ];
    if channel.requires_auth {
        capabilities.push("requests-secrets".to_string());
        capabilities.push("uses-auth-profile".to_string());
    }
    let report = crate::security::capabilities::evaluate_declared_capabilities(
        "custom_channel",
        &channel.id,
        &capabilities,
    );
    let severity = if report.blocked || report.risk_score_10 >= 8.0 {
        "high"
    } else if report.risk_score_10 >= 5.0 || !report.warnings.is_empty() {
        "medium"
    } else {
        "low"
    };
    let rules = report
        .matched_rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    spawn_security_log(
        state.agent.clone(),
        "capability_review",
        severity,
        format!(
            "Custom messaging channel capability review: channel_id={}, risk_score={}, capabilities=[{}], rules=[{}]",
            channel.id,
            report.risk_score_10,
            capabilities.join(", "),
            rules
        ),
        Some(format!(
            "source_kind=custom_channel;channel_id={}",
            channel.id
        )),
    );
}

pub(super) async fn list_custom_messaging_channels(State(state): State<AppState>) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_messaging_channels::list_custom_messaging_channels(
        &storage,
        &config_dir,
        &data_dir,
    )
    .await
    {
        Ok(channels) => Json(serde_json::json!({
            "custom_messaging_channels": channels,
            "count": channels.len(),
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn create_custom_messaging_channel(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Json(request): Json<crate::custom_messaging_channels::CustomMessagingChannelUpsertRequest>,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_messaging_channels::upsert_custom_messaging_channel(
        &storage,
        &config_dir,
        &data_dir,
        request,
        None,
    )
    .await
    {
        Ok(channel) => {
            let actor = caller_actor_label(maybe_caller.as_ref());
            audit_custom_channel_event(
                &state,
                "custom_messaging_channel_create",
                "medium",
                &actor,
                &channel.id,
                "Custom messaging channel created.",
            );
            audit_custom_channel_capability_review(&state, &channel);
            Json(serde_json::json!({
                "status": "ok",
                "custom_messaging_channel": channel,
            }))
            .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn update_custom_messaging_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Json(request): Json<crate::custom_messaging_channels::CustomMessagingChannelUpsertRequest>,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_messaging_channels::upsert_custom_messaging_channel(
        &storage,
        &config_dir,
        &data_dir,
        request,
        Some(id.as_str()),
    )
    .await
    {
        Ok(channel) => {
            let actor = caller_actor_label(maybe_caller.as_ref());
            audit_custom_channel_event(
                &state,
                "custom_messaging_channel_update",
                "medium",
                &actor,
                &channel.id,
                "Custom messaging channel updated.",
            );
            audit_custom_channel_capability_review(&state, &channel);
            Json(serde_json::json!({
                "status": "ok",
                "custom_messaging_channel": channel,
            }))
            .into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn delete_custom_messaging_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_messaging_channels::delete_custom_messaging_channel(
        &storage,
        &config_dir,
        &data_dir,
        id.as_str(),
    )
    .await
    {
        Ok(()) => {
            let actor = caller_actor_label(maybe_caller.as_ref());
            audit_custom_channel_event(
                &state,
                "custom_messaging_channel_delete",
                "medium",
                &actor,
                &id,
                "Custom messaging channel deleted.",
            );
            Json(serde_json::json!({ "status": "ok" })).into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn store_custom_messaging_channel_credentials(
    State(state): State<AppState>,
    Path(id): Path<String>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Json(request): Json<crate::custom_messaging_channels::CustomMessagingChannelCredentialsRequest>,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_messaging_channels::store_custom_messaging_channel_credentials(
        &storage,
        &config_dir,
        &data_dir,
        id.as_str(),
        &request.values,
    )
    .await
    {
        Ok(channel) => {
            let actor = caller_actor_label(maybe_caller.as_ref());
            audit_custom_channel_event(
                &state,
                "custom_messaging_channel_credentials_update",
                "medium",
                &actor,
                &channel.id,
                "Custom messaging channel credentials updated.",
            );
            Json(serde_json::json!({
                "status": "ok",
                "custom_messaging_channel": channel,
            }))
            .into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn test_custom_messaging_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match crate::custom_messaging_channels::test_custom_messaging_channel(
        &storage,
        &config_dir,
        &data_dir,
        id.as_str(),
    )
    .await
    {
        Ok(result) => {
            let actor = caller_actor_label(maybe_caller.as_ref());
            audit_custom_channel_event(
                &state,
                "custom_messaging_channel_test",
                if result.ok { "low" } else { "medium" },
                &actor,
                &id,
                if result.ok {
                    "Custom messaging channel test succeeded."
                } else {
                    "Custom messaging channel test failed."
                },
            );
            Json(serde_json::json!({ "status": "ok", "result": result })).into_response()
        }
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}
