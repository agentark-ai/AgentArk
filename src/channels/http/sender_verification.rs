use super::*;
use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustPolicy};

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct UpdateSenderVerificationSettingsRequest {
    #[serde(default)]
    pub google_chat_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub google_chat_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub signal_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub signal_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub imessage_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub imessage_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub line_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub line_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub slack_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub slack_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub teams_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub teams_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub whatsapp_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub whatsapp_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub wechat_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub wechat_allowed_senders: Option<Vec<String>>,
    #[serde(default)]
    pub qq_policy: Option<SenderTrustPolicy>,
    #[serde(default)]
    pub qq_allowed_senders: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ApproveSenderRequest {
    pub channel: SenderChannel,
    pub sender_id: String,
    #[serde(default)]
    pub sender_label: Option<String>,
    #[serde(default)]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub scope_label: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub approved_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RevokeSenderRequest {
    pub channel: SenderChannel,
    pub sender_id: String,
    #[serde(default)]
    pub scope_id: Option<String>,
}

fn normalize_sender_list(channel: SenderChannel, values: Option<Vec<String>>) -> Vec<String> {
    let mut normalized = values
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| match channel {
            SenderChannel::Whatsapp => value
                .chars()
                .filter(|ch| ch.is_ascii_digit())
                .collect::<String>(),
            SenderChannel::Slack
            | SenderChannel::Teams
            | SenderChannel::GoogleChat
            | SenderChannel::Signal
            | SenderChannel::IMessage
            | SenderChannel::Line
            | SenderChannel::WeChat
            | SenderChannel::Qq => value.to_ascii_lowercase(),
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn sender_identity_from_request(request: &ApproveSenderRequest) -> SenderIdentity {
    SenderIdentity {
        channel: request.channel,
        sender_id: request.sender_id.trim().to_string(),
        sender_label: request
            .sender_label
            .clone()
            .filter(|value| !value.trim().is_empty()),
        scope_id: request
            .scope_id
            .clone()
            .filter(|value| !value.trim().is_empty()),
        scope_label: request
            .scope_label
            .clone()
            .filter(|value| !value.trim().is_empty()),
        conversation_id: request
            .conversation_id
            .clone()
            .filter(|value| !value.trim().is_empty()),
        message_preview: None,
    }
}

async fn sync_whatsapp_legacy_approval(
    storage: &crate::storage::Storage,
    sender_id: &str,
    approved: bool,
) -> Result<()> {
    let digits = sender_id
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return Ok(());
    }
    let approved_key = format!("whatsapp:approved:{}", digits);
    if approved {
        storage.set(&approved_key, b"1").await?;
        let _ = storage
            .delete(&format!("whatsapp:pairing:{}", digits))
            .await;
    } else {
        let _ = storage.delete(&approved_key).await;
    }
    Ok(())
}

async fn overview_payload(agent: &Agent) -> Result<serde_json::Value> {
    let snapshot = sender_verification::load_snapshot(&agent.storage).await?;
    let google_chat = serde_json::json!({
        "configured": agent.config.google_chat.is_some(),
        "policy": snapshot.settings.google_chat.policy,
        "allowed_senders": snapshot.settings.google_chat.allowed_senders,
    });
    let signal = serde_json::json!({
        "configured": agent.config.signal.is_some(),
        "policy": snapshot.settings.signal.policy,
        "allowed_senders": snapshot.settings.signal.allowed_senders,
    });
    let imessage = serde_json::json!({
        "configured": agent.config.imessage.is_some(),
        "policy": snapshot.settings.imessage.policy,
        "allowed_senders": snapshot.settings.imessage.allowed_senders,
    });
    let line = serde_json::json!({
        "configured": agent.config.line.is_some(),
        "policy": snapshot.settings.line.policy,
        "allowed_senders": snapshot.settings.line.allowed_senders,
    });
    let slack = serde_json::json!({
        "configured": agent.config.slack.is_some(),
        "policy": snapshot.settings.slack.policy,
        "allowed_senders": snapshot.settings.slack.allowed_senders,
    });
    let teams = serde_json::json!({
        "configured": agent.config.teams.is_some(),
        "policy": snapshot.settings.teams.policy,
        "allowed_senders": snapshot.settings.teams.allowed_senders,
    });
    let wechat = serde_json::json!({
        "configured": agent.config.wechat.is_some(),
        "policy": snapshot.settings.wechat.policy,
        "allowed_senders": snapshot.settings.wechat.allowed_senders,
    });
    let qq = serde_json::json!({
        "configured": agent.config.qq.is_some(),
        "policy": snapshot.settings.qq.policy,
        "allowed_senders": snapshot.settings.qq.allowed_senders,
    });
    let whatsapp_config = agent.config.whatsapp.clone();
    let whatsapp = serde_json::json!({
        "configured": whatsapp_config.is_some(),
        "policy": whatsapp_config
            .as_ref()
            .map(|config| {
                if config.dm_policy.eq_ignore_ascii_case("pairing") {
                    SenderTrustPolicy::Pairing
                } else {
                    SenderTrustPolicy::Open
                }
            })
            .unwrap_or(SenderTrustPolicy::Pairing),
        "allowed_senders": whatsapp_config
            .as_ref()
            .map(|config| config.allowed_numbers.clone())
            .unwrap_or_default(),
    });

    Ok(serde_json::json!({
        "settings": {
            "google_chat": google_chat,
            "signal": signal,
            "imessage": imessage,
            "line": line,
            "slack": slack,
            "teams": teams,
            "whatsapp": whatsapp,
            "wechat": wechat,
            "qq": qq,
        },
        "pending": snapshot.pending,
        "approved": snapshot.approved,
        "channels": ["google_chat", "signal", "imessage", "line", "slack", "teams", "whatsapp", "wechat", "qq"],
    }))
}

pub(super) async fn get_sender_verification(State(state): State<AppState>) -> Response {
    let payload = {
        let agent = state.agent.read().await;
        overview_payload(&agent).await
    };
    match payload {
        Ok(body) => Json(body).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn update_sender_verification_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateSenderVerificationSettingsRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let storage = agent.storage.clone();
    let mut settings = match sender_verification::load_settings(&storage).await {
        Ok(settings) => settings,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };

    if let Some(policy) = request.google_chat_policy {
        settings.google_chat.policy = policy;
    }
    if let Some(values) = request.google_chat_allowed_senders {
        settings.google_chat.allowed_senders =
            normalize_sender_list(SenderChannel::GoogleChat, Some(values));
    }
    if let Some(policy) = request.signal_policy {
        settings.signal.policy = policy;
    }
    if let Some(values) = request.signal_allowed_senders {
        settings.signal.allowed_senders =
            normalize_sender_list(SenderChannel::Signal, Some(values));
    }
    if let Some(policy) = request.imessage_policy {
        settings.imessage.policy = policy;
    }
    if let Some(values) = request.imessage_allowed_senders {
        settings.imessage.allowed_senders =
            normalize_sender_list(SenderChannel::IMessage, Some(values));
    }
    if let Some(policy) = request.line_policy {
        settings.line.policy = policy;
    }
    if let Some(values) = request.line_allowed_senders {
        settings.line.allowed_senders = normalize_sender_list(SenderChannel::Line, Some(values));
    }
    if let Some(policy) = request.slack_policy {
        settings.slack.policy = policy;
    }
    if let Some(values) = request.slack_allowed_senders {
        settings.slack.allowed_senders = normalize_sender_list(SenderChannel::Slack, Some(values));
    }
    if let Some(policy) = request.teams_policy {
        settings.teams.policy = policy;
    }
    if let Some(values) = request.teams_allowed_senders {
        settings.teams.allowed_senders = normalize_sender_list(SenderChannel::Teams, Some(values));
    }
    if let Some(policy) = request.wechat_policy {
        settings.wechat.policy = policy;
    }
    if let Some(values) = request.wechat_allowed_senders {
        settings.wechat.allowed_senders =
            normalize_sender_list(SenderChannel::WeChat, Some(values));
    }
    if let Some(policy) = request.qq_policy {
        settings.qq.policy = policy;
    }
    if let Some(values) = request.qq_allowed_senders {
        settings.qq.allowed_senders = normalize_sender_list(SenderChannel::Qq, Some(values));
    }

    if request.whatsapp_policy.is_some() || request.whatsapp_allowed_senders.is_some() {
        let Some(current) = agent.config.whatsapp.as_mut() else {
            return error_response(
                StatusCode::BAD_REQUEST,
                "WhatsApp must be configured before sender verification can be changed here.",
            );
        };
        if let Some(policy) = request.whatsapp_policy {
            current.dm_policy = match policy {
                SenderTrustPolicy::Open => "open".to_string(),
                SenderTrustPolicy::Pairing => "pairing".to_string(),
            };
        }
        if let Some(values) = request.whatsapp_allowed_senders {
            current.allowed_numbers = normalize_sender_list(SenderChannel::Whatsapp, Some(values));
        }
        if let Err(error) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    }

    if let Err(error) = sender_verification::save_settings(&storage, &settings).await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
    }

    match overview_payload(&agent).await {
        Ok(body) => Json(body).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn approve_sender(
    State(state): State<AppState>,
    Json(request): Json<ApproveSenderRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let storage = agent.storage.clone();
    let identity = sender_identity_from_request(&request);
    let approved = match sender_verification::approve_sender(
        &storage,
        &identity,
        request.approved_by.as_deref().or(Some("settings_ui")),
    )
    .await
    {
        Ok(approved) => approved,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    if request.channel == SenderChannel::Whatsapp {
        if let Err(error) = sync_whatsapp_legacy_approval(&storage, &request.sender_id, true).await
        {
            tracing::warn!("Failed to sync legacy WhatsApp approval: {}", error);
        }
    }

    let body = format!(
        "Approved {} sender {}{}.",
        approved.channel.as_str(),
        approved
            .sender_label
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(approved.sender_id.as_str()),
        approved
            .scope_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|scope| format!(" in {}", scope))
            .unwrap_or_default()
    );
    agent
        .emit_notification_forced("Sender Approved", &body, "info", "sender_verification")
        .await;

    let payload = match overview_payload(&agent).await {
        Ok(body) => body,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    Json(serde_json::json!({
        "status": "ok",
        "approved": approved,
        "overview": payload,
    }))
    .into_response()
}

pub(super) async fn revoke_sender(
    State(state): State<AppState>,
    Json(request): Json<RevokeSenderRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let storage = agent.storage.clone();
    let deleted = match sender_verification::revoke_sender(
        &storage,
        request.channel,
        request.sender_id.as_str(),
        request.scope_id.as_deref(),
    )
    .await
    {
        Ok(found) => found,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    if request.channel == SenderChannel::Whatsapp {
        if let Err(error) = sync_whatsapp_legacy_approval(&storage, &request.sender_id, false).await
        {
            tracing::warn!("Failed to sync legacy WhatsApp revocation: {}", error);
        }
    }

    if deleted {
        let body = format!(
            "Revoked {} sender {}.",
            request.channel.as_str(),
            request.sender_id.trim()
        );
        agent
            .emit_notification_forced("Sender Revoked", &body, "warning", "sender_verification")
            .await;
    }

    let payload = match overview_payload(&agent).await {
        Ok(body) => body,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    Json(serde_json::json!({
        "status": "ok",
        "deleted": deleted,
        "overview": payload,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Agent;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use axum::routing::{get, post};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    async fn build_test_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let shared = Arc::new(RwLock::new(
            Agent::init(
                config_dir.path(),
                data_dir.path(),
                crate::storage::DatabaseConfig::for_tests()
                    .expect("test database config should initialize"),
                None,
            )
            .await
            .unwrap(),
        ));
        let (trace_history, last_trace, tasks, user_profile, security_events, app_registry) = {
            let guard = shared.read().await;
            (
                guard.trace_history.clone(),
                guard.last_trace.clone(),
                guard.tasks.clone(),
                guard.user_profile.clone(),
                guard.security_events.clone(),
                guard.app_registry.clone(),
            )
        };
        (
            AppState {
                agent: shared,
                trace_history,
                last_trace,
                tasks,
                chat_task_cancellations: Arc::new(RwLock::new(HashMap::new())),
                chat_conversation_cancellations: Arc::new(RwLock::new(HashMap::new())),
                user_profile,
                tiered_rate_limiter: TieredRateLimiter::new(),
                api_key: Arc::new(RwLock::new(None)),
                api_key_expires_at: Arc::new(RwLock::new(None)),
                allow_insecure_no_auth: true,
                ui_sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
                local_ui_bootstrap_enabled: true,
                local_ui_bootstrap_tokens: Arc::new(RwLock::new(HashMap::new())),
                cookie_secure_default: false,
                oauth_states: Arc::new(RwLock::new(HashMap::new())),
                remote_login_attempts: Arc::new(RwLock::new(HashMap::new())),
                tunnel: Arc::new(RwLock::new(tunnel::TunnelState::new())),
                whatsapp_bridge: Arc::new(RwLock::new(WhatsAppBridgeState::new())),
                security_events,
                app_registry,
                executor_client: None,
                workspace_client: None,
                application_registry: applications::ApplicationLauncherRegistry::default(),
                deployment_mode: DeploymentMode::TrustedLocal,
                server_role: HttpServerRole::ControlPlane,
                public_app_bind_addr: None,
                public_app_base_url: None,
            },
            config_dir,
            data_dir,
        )
    }

    async fn json_response(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn sender_verification_routes_round_trip() {
        let (state, _config_dir, _data_dir) = build_test_state().await;
        let app = Router::new()
            .route("/sender-verification", get(get_sender_verification))
            .route(
                "/sender-verification/settings",
                post(update_sender_verification_settings),
            )
            .route("/sender-verification/approve", post(approve_sender))
            .route("/sender-verification/revoke", post(revoke_sender))
            .with_state(state);

        let request = Request::builder()
            .uri("/sender-verification/settings")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "slack_policy": "pairing",
                    "slack_allowed_senders": ["UADMIN"]
                })
                .to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_response(response).await;
        assert_eq!(payload["settings"]["slack"]["policy"], "pairing");

        let approve_request = Request::builder()
            .uri("/sender-verification/approve")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "channel": "slack",
                    "sender_id": "U123",
                    "scope_id": "T999",
                    "sender_label": "Alice"
                })
                .to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(approve_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_response(response).await;
        assert_eq!(payload["approved"]["sender_id"], "U123");

        let get_request = Request::builder()
            .uri("/sender-verification")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(get_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_response(response).await;
        assert_eq!(payload["approved"][0]["sender_id"], "U123");

        let revoke_request = Request::builder()
            .uri("/sender-verification/revoke")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "channel": "slack",
                    "sender_id": "U123",
                    "scope_id": "T999"
                })
                .to_string(),
            ))
            .unwrap();
        let response = app.oneshot(revoke_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_response(response).await;
        assert_eq!(payload["deleted"], true);
    }
}
