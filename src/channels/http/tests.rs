use super::*;
use crate::core::autonomy::{RecommendedAction, RiskEnvelope};
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, Request};
use axum::routing::{get, post};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tempfile::TempDir;
use tower::ServiceExt;

async fn build_test_state() -> (AppState, TempDir, TempDir) {
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
    let state = AppState {
        agent: shared,
        trace_history,
        last_trace,
        tasks,
        chat_task_cancellations: Arc::new(RwLock::new(HashMap::new())),
        action_test_cancellations: Arc::new(RwLock::new(HashMap::new())),
        chat_conversation_cancellations: Arc::new(RwLock::new(HashMap::new())),
        user_profile,
        tiered_rate_limiter: TieredRateLimiter::new(),
        api_key: Arc::new(RwLock::new(None)),
        api_key_expires_at: Arc::new(RwLock::new(None)),
        allow_insecure_no_auth: true,
        ui_sessions: Arc::new(RwLock::new(HashMap::new())),
        local_ui_bootstrap_enabled: true,
        local_ui_bootstrap_tokens: Arc::new(RwLock::new(HashMap::new())),
        cookie_secure_default: false,
        oauth_states: Arc::new(RwLock::new(HashMap::new())),
        remote_login_attempts: Arc::new(RwLock::new(HashMap::new())),
        tunnel: Arc::new(RwLock::new(tunnel::TunnelState::new())),
        whatsapp_bridge: Arc::new(RwLock::new(WhatsAppBridgeState::new())),
        security_events,
        app_registry,
        app_publish_locks: Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new())),
        executor_client: None,
        workspace_client: None,
        application_registry: applications::ApplicationLauncherRegistry::default(),
        deployment_mode: DeploymentMode::TrustedLocal,
        server_role: HttpServerRole::ControlPlane,
        public_app_bind_addr: None,
        public_app_base_url: None,
        release_update_cache: Arc::new(RwLock::new(ReleaseUpdateCache::default())),
    };
    (state, config_dir, data_dir)
}

fn loopback_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 45678)
}

async fn response_json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn clarification_choices_from_operational_payload_requires_true_clarification_flag() {
    let payload = serde_json::json!({
        "should_clarify": false,
        "choices": [{
            "label": "Build and deploy",
            "submit_text": "Build and deploy it as an isolated AgentArk app."
        }]
    })
    .to_string();

    let choices = clarification_choices_from_operational_payload(Some(&payload));

    assert!(choices.is_empty());
}

#[test]
fn clarification_choices_from_operational_payload_keeps_real_clarifications() {
    let payload = serde_json::json!({
        "should_clarify": true,
        "clarification_question": "Workspace or app?",
        "choices": [{
            "label": "Workspace only",
            "submit_text": "Only build the files in the workspace."
        }]
    })
    .to_string();

    let choices = clarification_choices_from_operational_payload(Some(&payload));

    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].label, "Workspace only");
    assert_eq!(
        choices[0].submit_text,
        "Only build the files in the workspace."
    );
}

async fn add_test_task(state: &AppState, task: crate::core::Task) {
    let agent = state.agent.read().await;
    agent
        .add_task(task)
        .await
        .expect("test task should be added");
}

async fn create_test_conversation_with_user_message(
    state: &AppState,
    conversation_id: &str,
    message_text: &str,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let agent = state.agent.read().await;
    agent
        .storage
        .create_conversation(&crate::storage::entities::conversation::Model {
            id: conversation_id.to_string(),
            title: "Resume test conversation".to_string(),
            channel: "web".to_string(),
            project_id: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            message_count: 1,
            archived: false,
            starred: false,
        })
        .await
        .expect("conversation should be created");
    agent
        .encrypted_storage
        .insert_message_encrypted(&crate::storage::entities::message::Model {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.to_string(),
            role: "user".to_string(),
            content: message_text.to_string(),
            timestamp: now,
            model_used: None,
            trace_id: None,
        })
        .await
        .expect("user message should be inserted");
}

fn test_conversation_model() -> crate::storage::entities::conversation::Model {
    let now = chrono::Utc::now().to_rfc3339();
    crate::storage::entities::conversation::Model {
        id: "conv-proactive".to_string(),
        title: "Proactive automation".to_string(),
        channel: "web".to_string(),
        project_id: None,
        created_at: now.clone(),
        updated_at: now,
        message_count: 1,
        archived: false,
        starred: false,
    }
}

fn test_user_message(content: &str) -> crate::storage::entities::message::Model {
    crate::storage::entities::message::Model {
        id: "msg-proactive".to_string(),
        conversation_id: "conv-proactive".to_string(),
        role: "user".to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        model_used: None,
        trace_id: None,
    }
}

#[test]
fn build_chat_suggestion_from_inference_creates_watcher() {
    let conversation = test_conversation_model();
    let message = test_user_message(
        "Watch my inbox for urgent client messages and alert me if I do not reply.",
    );
    let payload = serde_json::json!({
        "should_suggest": true,
        "kind": "watcher",
        "title": "Watch urgent client inbox follow-ups",
        "detail": "Create a watcher that checks for urgent client messages and brings drafts back for approval.",
        "rationale": "The user is describing background monitoring with a review step.",
        "confidence": 0.82,
        "goal_title": "Set up client inbox watcher",
        "goal_detail": "Monitor urgent client mail and keep outbound replies approval-gated."
    });

    let suggestion =
        build_chat_automation_suggestion_from_inference(&conversation, &message, &payload)
            .expect("inbox monitoring should become a watcher suggestion");

    assert_eq!(suggestion.kind, "watcher");
    assert_eq!(suggestion.title, "Watch urgent client inbox follow-ups");
    assert_eq!(suggestion.conversation_id, conversation.id);
    assert_eq!(suggestion.source_message_id, message.id);
}

#[test]
fn build_chat_suggestion_from_inference_creates_workflow() {
    let conversation = test_conversation_model();
    let message = test_user_message(
        "Every weekday, draft replies from my inbox and wait for my approval before sending.",
    );
    let payload = serde_json::json!({
        "should_suggest": true,
        "kind": "workflow",
        "title": "Prepare weekday inbox reply drafts",
        "detail": "Create a recurring workflow that prepares reply drafts and waits for approval before sending.",
        "rationale": "The user is describing repeated proactive preparation with an explicit approval boundary.",
        "confidence": 0.88
    });

    let suggestion =
        build_chat_automation_suggestion_from_inference(&conversation, &message, &payload)
            .expect("recurring draft-and-approve request should become a workflow suggestion");

    assert_eq!(suggestion.kind, "workflow");
    assert!(suggestion.rationale.contains("approval boundary"));
}

#[test]
fn build_chat_suggestion_from_inference_skips_negative_decision() {
    let conversation = test_conversation_model();
    let message = test_user_message("Can you explain the architecture in the README?");
    let payload = serde_json::json!({
        "should_suggest": false,
        "kind": null,
        "title": "",
        "detail": "",
        "rationale": "The user asked for a one-off explanation.",
        "confidence": 0.94
    });

    assert!(
        build_chat_automation_suggestion_from_inference(&conversation, &message, &payload)
            .is_none()
    );
}

fn test_recommended_action(kind: &str, title: &str) -> RecommendedAction {
    RecommendedAction {
        id: "action-1".to_string(),
        title: title.to_string(),
        description: String::new(),
        action_kind: kind.to_string(),
        payload: serde_json::json!({}),
        trust: RiskEnvelope::default(),
        readiness: None,
    }
}

#[test]
fn summarize_daily_brief_result_includes_delivery_and_preview() {
    let action = test_recommended_action("daily_brief_now", "Generate Daily Brief");
    let summary = summarize_autonomy_action_result(
        &action,
        &serde_json::json!({
            "status": "executed",
            "kind": "daily_brief_now",
            "brief": "Morning command brief for Tue, Apr 14 06:19 PM UTC\n- Priority: queue is quiet right now.",
            "delivery": {
                "in_app": { "channel": "web", "success": true, "error": serde_json::Value::Null },
                "push_attempts": [
                    { "channel": "telegram", "success": true, "error": serde_json::Value::Null }
                ]
            }
        }),
    );

    assert!(summary.contains("Push delivered via Telegram."));
    assert!(summary.contains("Preview: Morning command brief for Tue, Apr 14 06:19 PM UTC"));
}

#[test]
fn summarize_daily_brief_result_reports_in_app_only_when_push_fails() {
    let action = test_recommended_action("daily_brief_now", "Generate Daily Brief");
    let summary = summarize_autonomy_action_result(
        &action,
        &serde_json::json!({
            "status": "executed",
            "kind": "daily_brief_now",
            "brief": "Morning command brief",
            "delivery": {
                "in_app": { "channel": "web", "success": true, "error": serde_json::Value::Null },
                "push_attempts": [
                    {
                        "channel": "push",
                        "success": false,
                        "error": "No connected notification integrations available"
                    }
                ]
            }
        }),
    );

    assert!(summary.contains("Saved in-app only."));
    assert!(
        summary.contains("Push delivery failed: No connected notification integrations available.")
    );
}

#[tokio::test]
async fn security_headers_include_hsts_for_internet_facing() {
    let (mut state, _config_dir, _data_dir) = build_test_state().await;
    state.deployment_mode = DeploymentMode::InternetFacing;
    let router = Router::new()
        .route("/headers", get(|| async { "ok" }))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers_middleware,
        ));

    let response = router
        .oneshot(
            Request::builder()
                .uri("/headers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.headers().get(&HEADER_X_FRAME_OPTIONS),
        Some(&HeaderValue::from_static("DENY"))
    );
    assert_eq!(
        response.headers().get(&HEADER_X_CONTENT_TYPE_OPTIONS),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert_eq!(
        response.headers().get(&HEADER_CONTENT_SECURITY_POLICY),
        Some(&HeaderValue::from_static(
            "frame-ancestors 'none'; base-uri 'self'; object-src 'none'"
        ))
    );
    assert_eq!(
        response.headers().get(&HEADER_STRICT_TRANSPORT_SECURITY),
        Some(&HeaderValue::from_static(
            "max-age=31536000; includeSubDomains"
        ))
    );
}

#[test]
fn static_apps_do_not_proxy_to_executor() {
    assert!(!should_proxy_app_request_to_executor(true, true));
    assert!(!should_proxy_app_request_to_executor(true, false));
    assert!(should_proxy_app_request_to_executor(false, true));
    assert!(!should_proxy_app_request_to_executor(false, false));
}

#[test]
fn direct_control_plane_exposure_warning_ignores_trusted_local_docker_bind() {
    assert!(!should_warn_for_direct_control_plane_exposure(
        DeploymentMode::TrustedLocal,
        "0.0.0.0:8990"
    ));
}

#[test]
fn direct_control_plane_exposure_warning_requires_internet_facing_non_loopback_bind() {
    assert!(should_warn_for_direct_control_plane_exposure(
        DeploymentMode::InternetFacing,
        "0.0.0.0:8990"
    ));
    assert!(should_warn_for_direct_control_plane_exposure(
        DeploymentMode::InternetFacing,
        "192.168.1.20:8990"
    ));
    assert!(!should_warn_for_direct_control_plane_exposure(
        DeploymentMode::InternetFacing,
        "127.0.0.1:8990"
    ));
}

#[test]
fn wildcard_bind_display_addr_maps_to_localhost() {
    assert_eq!(
        display_addr_for_bind_addr("0.0.0.0:8990").as_deref(),
        Some("localhost:8990")
    );
    assert_eq!(
        display_addr_for_bind_addr("[::]:8990").as_deref(),
        Some("localhost:8990")
    );
}

#[test]
fn display_url_for_bind_addr_preserves_scheme_and_normalizes_wildcard_bind() {
    assert_eq!(
        display_url_for_bind_addr("0.0.0.0:8990", "http").as_deref(),
        Some("http://localhost:8990")
    );
    assert_eq!(
        display_url_for_bind_addr("192.168.1.20:8990", "https").as_deref(),
        Some("https://192.168.1.20:8990")
    );
}

fn test_access_planner_action(
    name: &str,
    description: &str,
    capabilities: &[&str],
    risk_level: crate::actions::ActionRiskLevel,
    permission_ids: &[&str],
) -> crate::actions::ActionDef {
    crate::actions::ActionDef {
        name: name.to_string(),
        description: description.to_string(),
        capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
        authorization: crate::actions::ActionAuthorization {
            risk_level,
            access: crate::actions::ActionAccessMetadata {
                permission_ids: permission_ids
                    .iter()
                    .map(|value| value.to_string())
                    .collect(),
                ..crate::actions::ActionAccessMetadata::default()
            },
            ..crate::actions::ActionAuthorization::default()
        },
        ..crate::actions::ActionDef::default()
    }
}

#[test]
fn access_planner_does_not_infer_capability_acquire_from_generic_custom_agent_text() {
    let spec_summary = "name: Code Reviewer\nrole: coder\ncapabilities: debugging, code review, refactoring\nsystem_prompt: You are a custom coder agent. Review pull requests and explain changes.";
    let actions = vec![
        test_access_planner_action(
            "capability_acquire",
            "Scaffold a reusable integration/action when the needed capability does not already exist. Generates a reviewable custom SKILL.md backed by connector_request and/or browser_auto.",
            &["integration_builder"],
            crate::actions::ActionRiskLevel::High,
            &["capability_acquire"],
        ),
        test_access_planner_action(
            "code_execute",
            "Run code and tests in a project workspace.",
            &["code_execute"],
            crate::actions::ActionRiskLevel::High,
            &["code_execute"],
        ),
    ];

    let planned = fallback_access_plan_actions(spec_summary, &actions);
    assert!(
        !planned
            .iter()
            .any(|action| action.name == "capability_acquire"),
        "unexpected capability_acquire plan: {:?}",
        planned
    );
}

#[tokio::test]
async fn auth_middleware_does_not_bypass_when_legacy_flag_is_set() {
    let (mut state, _config_dir, _data_dir) = build_test_state().await;
    state.allow_insecure_no_auth = true;

    let router = Router::new()
        .route("/protected", get(|| async { "ok" }))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));

    let mut request = Request::builder()
        .uri("/protected")
        .header(header::HOST, "example.com")
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 10], 4242))));

    let response = router.oneshot(request).await.unwrap();
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn chat_rejects_oversized_messages() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/chat", post(chat))
        .with_state(state.clone());
    let body = serde_json::json!({
        "message": "x".repeat(MAX_CHAT_MESSAGE_BYTES + 1),
        "channel": "http",
        "conversation_id": serde_json::Value::Null,
        "project_id": serde_json::Value::Null,
        "deep_research": false,
        "execution_mode": serde_json::Value::Null,
        "attachments_present": false
    });
    let mut request = Request::builder()
        .method("POST")
        .uri("/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8990))));

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let payload = response_json(response).await;
    assert!(payload
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("100000"));
}

#[tokio::test]
async fn chat_fast_path_stores_secret_without_live_server() {
    let (state, config_dir, data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/chat", post(chat))
        .with_state(state.clone());
    let body = serde_json::json!({
        "message": "/setsecret TEST_FAST_PATH_SECRET=abc123",
        "channel": "web",
        "conversation_id": serde_json::Value::Null,
        "project_id": serde_json::Value::Null,
        "deep_research": false,
        "execution_mode": serde_json::Value::Null,
        "attachments_present": false
    });
    let mut request = Request::builder()
        .method("POST")
        .uri("/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8990))));

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert!(payload["response"]
        .as_str()
        .unwrap_or_default()
        .contains("Saved secret 'TEST_FAST_PATH_SECRET'"));

    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        config_dir.path(),
        Some(data_dir.path()),
    )
    .expect("test manager should initialize");
    assert_eq!(
        manager
            .get_custom_secret("TEST_FAST_PATH_SECRET")
            .expect("secret should be readable"),
        Some("abc123".to_string())
    );
}

#[tokio::test]
async fn chat_fast_path_controls_notifications_without_live_server() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/chat", post(chat))
        .with_state(state.clone());
    let body = serde_json::json!({
        "message": "/notifications pause",
        "channel": "web",
        "conversation_id": serde_json::Value::Null,
        "project_id": serde_json::Value::Null,
        "deep_research": false,
        "execution_mode": serde_json::Value::Null,
        "attachments_present": false
    });
    let mut request = Request::builder()
        .method("POST")
        .uri("/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8990))));

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert!(payload["response"]
        .as_str()
        .unwrap_or_default()
        .contains("Push notifications paused until"));
    assert!(payload["conversation_id"]
        .as_str()
        .is_some_and(|value| !value.trim().is_empty()));

    let agent = state.agent.read().await;
    assert!(agent.push_notifications_muted_until_ts().await.is_some());
}

#[tokio::test]
async fn chat_stream_fast_path_stores_secret_without_live_server() {
    let (state, config_dir, data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/chat/stream", post(chat_stream))
        .with_state(state);
    let body = serde_json::json!({
        "message": "/setsecret TEST_STREAM_SECRET=stream-abc",
        "channel": "web",
        "conversation_id": serde_json::Value::Null,
        "project_id": serde_json::Value::Null,
        "deep_research": false,
        "execution_mode": serde_json::Value::Null,
        "attachments_present": false
    });
    let mut request = Request::builder()
        .method("POST")
        .uri("/chat/stream")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8990))));

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("event: content"));
    assert!(text.contains("TEST_STREAM_SECRET"));
    assert!(text.contains("\"conversation_id\":\""));

    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        config_dir.path(),
        Some(data_dir.path()),
    )
    .expect("test manager should initialize");
    assert_eq!(
        manager
            .get_custom_secret("TEST_STREAM_SECRET")
            .expect("stream secret should be readable"),
        Some("stream-abc".to_string())
    );
}

#[tokio::test]
async fn notifications_endpoints_work_hermetically() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let agent = state.agent.read().await;
        agent
            .emit_notification("Hermetic test one", "body", "info", "test")
            .await;
        agent
            .emit_notification("Hermetic test two", "body", "warning", "test")
            .await;
    }

    let router = Router::new()
        .route("/notifications/count", get(notification_count_endpoint))
        .route("/notifications/read-all", post(mark_all_read_endpoint))
        .with_state(state);

    let count_request = Request::builder()
        .method("GET")
        .uri("/notifications/count?unread=true")
        .body(Body::empty())
        .unwrap();
    let count_response = router.clone().oneshot(count_request).await.unwrap();
    assert_eq!(count_response.status(), StatusCode::OK);
    let count_payload = response_json(count_response).await;
    assert_eq!(count_payload["unread"].as_u64().unwrap_or_default(), 2);

    let mark_all_request = Request::builder()
        .method("POST")
        .uri("/notifications/read-all")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let mark_all_response = router.clone().oneshot(mark_all_request).await.unwrap();
    assert_eq!(mark_all_response.status(), StatusCode::OK);

    let unread_after_request = Request::builder()
        .method("GET")
        .uri("/notifications/count?unread=true")
        .body(Body::empty())
        .unwrap();
    let unread_after_response = router.oneshot(unread_after_request).await.unwrap();
    assert_eq!(unread_after_response.status(), StatusCode::OK);
    let unread_after_payload = response_json(unread_after_response).await;
    assert_eq!(
        unread_after_payload["unread"].as_u64().unwrap_or_default(),
        0
    );
}

#[tokio::test]
async fn swarm_endpoints_work_hermetically() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/swarm/status", get(swarm_status))
        .route("/swarm/config", get(swarm_get_config))
        .route("/swarm/config", post(swarm_update_config))
        .with_state(state);

    let status_request = Request::builder()
        .method("GET")
        .uri("/swarm/status")
        .body(Body::empty())
        .unwrap();
    let status_response = router.clone().oneshot(status_request).await.unwrap();
    assert_eq!(status_response.status(), StatusCode::OK);
    let status_payload = response_json(status_response).await;
    assert!(status_payload.get("enabled").is_some());
    assert!(status_payload.get("agents").is_some());

    let update_request = Request::builder()
        .method("POST")
        .uri("/swarm/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "max_specialists": 5,
                "default_timeout_secs": 240
            })
            .to_string(),
        ))
        .unwrap();
    let update_response = router.clone().oneshot(update_request).await.unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);

    let config_request = Request::builder()
        .method("GET")
        .uri("/swarm/config")
        .body(Body::empty())
        .unwrap();
    let config_response = router.oneshot(config_request).await.unwrap();
    assert_eq!(config_response.status(), StatusCode::OK);
    let config_payload = response_json(config_response).await;
    assert_eq!(
        config_payload["max_specialists"]
            .as_u64()
            .unwrap_or_default(),
        5
    );
    assert_eq!(
        config_payload["default_timeout_secs"]
            .as_u64()
            .unwrap_or_default(),
        240
    );
}

#[test]
fn arkpulse_managed_app_operation_remediation_maps_to_fix_plan() {
    let plan = arkpulse_fix_plan_from_remediation(
        &crate::sentinel::DoctorRemediationSpec::ManagedAppOperation {
            app_id: "demo_app".to_string(),
            operation: crate::sentinel::DoctorManagedAppOperation::GenerateCargoLockfile,
        },
        false,
    );

    match plan {
        Some(ArkPulseFixPlan::ManagedAppOperation { app_id, operation }) => {
            assert_eq!(app_id, "demo_app");
            assert_eq!(
                operation,
                crate::sentinel::DoctorManagedAppOperation::GenerateCargoLockfile
            );
        }
        other => panic!(
            "expected managed app operation, got {:?}",
            other.map(|_| ())
        ),
    }
}

#[test]
fn arkpulse_shell_command_text_is_not_an_auto_fix_plan() {
    let plan = arkpulse_fix_plan_from_remediation(
        &crate::sentinel::DoctorRemediationSpec::ShellCommand {
            command: r#"cd /app/data/apps/demo && cargo generate-lockfile"#.to_string(),
        },
        true,
    );
    assert!(plan.is_none());
}

#[test]
fn arkpulse_readonly_investigation_remediation_maps_to_fix_plan() {
    let plan = arkpulse_fix_plan_from_remediation(
        &crate::sentinel::DoctorRemediationSpec::ReadonlyInvestigation {
            topic: crate::sentinel::DoctorReadonlyInvestigationTopic::MemoryCaptureHealth,
        },
        false,
    );

    match plan {
        Some(ArkPulseFixPlan::ReadonlyInvestigation { topic }) => {
            assert_eq!(
                topic,
                crate::sentinel::DoctorReadonlyInvestigationTopic::MemoryCaptureHealth
            );
        }
        other => panic!(
            "expected readonly investigation plan, got {:?}",
            other.map(|_| ())
        ),
    }
}

#[test]
fn summarize_stream_tool_activity_content_hides_html_payloads() {
    let summary = summarize_stream_tool_activity_content(
        "<!DOCTYPE html><html><head><title>arXiv Research Monitor | RL & Time-Series</title></head><body><div>demo</div></body></html>",
    );

    assert_eq!(
        summary,
        "Read HTML document: arXiv Research Monitor | RL & Time-Series."
    );
    assert!(!summary.contains("<!DOCTYPE html>"));
}

#[test]
fn rewrite_external_proxy_urls_for_public_apps_rewrites_known_public_proxy_wrappers() {
    let input = r#"fetch("https://api.allorigins.win/raw?url=https://export.arxiv.org/api/query?search_query=all:transformer");"#;

    let rewritten = rewrite_external_proxy_urls_for_public_apps(input, "demo-app");

    assert!(rewritten.contains(
        "/apps/demo-app/__agentark/http/fetch?url=https://export.arxiv.org/api/query?search_query=all:transformer"
    ));
    assert!(!rewritten.contains("https://api.allorigins.win/raw?url="));
}

#[test]
fn rewrite_external_proxy_urls_for_public_apps_does_not_double_proxy_urls() {
    let input = r#"fetch("/apps/demo-app/__agentark/http/fetch?url=/apps/demo-app/__agentark/http/fetch?url=https://export.arxiv.org/api/query?search_query=all:transformer");"#;

    let rewritten = rewrite_external_proxy_urls_for_public_apps(input, "demo-app");

    assert_eq!(
        rewritten,
        r#"fetch("/apps/demo-app/__agentark/http/fetch?url=https://export.arxiv.org/api/query?search_query=all:transformer");"#
    );
    assert!(!rewritten.contains(
        "/apps/demo-app/__agentark/http/fetch?url=/apps/demo-app/__agentark/http/fetch?url="
    ));
}

#[test]
fn guess_app_content_type_uses_browser_mimes_for_app_assets() {
    assert_eq!(
        guess_app_content_type("style.css"),
        "text/css; charset=utf-8"
    );
    assert_eq!(
        guess_app_content_type("app.js"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        guess_app_content_type("module.mjs"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(guess_content_type("style.css"), "text/plain; charset=utf-8");
}

#[test]
fn build_arxiv_search_request_from_source_url_repairs_nested_proxy_and_field_aliases() {
    let source = "/apps/demo-app/__agentark/http/fetch?url=/public/proxy/raw?url=https://export.arxiv.org/api/query?search_query=(cat:cs.LG OR cat:cs.RO OR (cat:stat.ML AND (title:time series OR abs:time series OR forecasting OR abs:forecasting)))&start=0&max_results=50&sortBy=submittedDate&sortOrder=descending";

    let request = build_arxiv_search_request_from_source_url(source)
        .expect("nested arxiv proxy url should normalize");

    assert_eq!(request.start, 0);
    assert_eq!(request.max_results, 50);
    assert_eq!(request.sort_by, "submittedDate");
    assert_eq!(request.sort_order, "descending");
    assert!(request.search_query.contains("cat:cs.LG"));
    assert!(request.search_query.contains("cat:cs.RO"));
    assert!(request.search_query.contains("cat:stat.ML"));
    assert!(request.search_query.contains("ti:\"time series\""));
    assert!(request.search_query.contains("abs:\"time series\""));
    assert!(request.search_query.contains("ti:forecasting"));
    assert!(request.search_query.contains("abs:forecasting"));
}

#[test]
fn arxiv_public_proxy_canonicalizes_browser_generated_search_queries() {
    let source = "https://export.arxiv.org/api/query?search_query=all:%22machine%20learning%22%20OR%20all:%22reinforcement%20learning%22%20OR%20all:%22time%20series%22%20AND%20(abs:novel%20approach%20OR%20abs:propose)&start=0&max_results=45&sortBy=submittedDate&sortOrder=descending";
    let parsed = reqwest::Url::parse(source).expect("source arxiv url should parse");

    let canonical = canonicalize_public_arxiv_api_url(&parsed);
    let search_query =
        extract_query_value_from_url(&canonical, "search_query").expect("search query");

    assert!(search_query.contains("ti:\"machine learning\""));
    assert!(search_query.contains("abs:\"machine learning\""));
    assert!(search_query.contains("ti:\"reinforcement learning\""));
    assert!(search_query.contains("abs:\"time series\""));
    assert!(search_query.contains("ti:\"novel approach\""));
    assert!(search_query.contains("abs:propose"));
    assert_eq!(
        extract_query_value_from_url(&canonical, "max_results").as_deref(),
        Some("45")
    );
}

#[test]
fn inject_app_runtime_fetch_shims_adds_generic_public_fetch_proxy() {
    let html = "<!DOCTYPE html><html><head></head><body></body></html>";

    let rewritten = inject_app_runtime_fetch_shims(html, "demo-app");

    assert!(rewritten.contains("/apps/\" + encodeURIComponent(APP_ID) + \"/__agentark/http/fetch"));
    assert!(rewritten.contains("shouldProxyPublicRead"));
    assert!(rewritten.contains("x-agentark-app-proxy\", \"raw"));
    assert!(rewritten.contains("XMLHttpRequest"));
}

#[test]
fn inject_app_runtime_fetch_shims_keeps_xhr_helpers_in_scope() {
    let html = "<!DOCTYPE html><html><head></head><body></body></html>";

    let rewritten = inject_app_runtime_fetch_shims(html, "demo-app");

    let to_absolute = rewritten
        .find("const toAbsoluteUrl =")
        .expect("shim should define absolute-url helper");
    let should_proxy = rewritten
        .find("const shouldProxyPublicRead =")
        .expect("shim should define public-read helper");
    let native_fetch = rewritten
        .find("if (nativeFetch)")
        .expect("shim should wrap fetch when available");
    let xhr_open = rewritten
        .find("xhrProto.open = function")
        .expect("shim should wrap XMLHttpRequest.open");

    assert!(to_absolute < native_fetch);
    assert!(should_proxy < native_fetch);
    assert!(native_fetch < xhr_open);
}

#[test]
fn extract_public_proxy_target_hosts_from_text_finds_direct_and_wrapped_hosts() {
    let source = r#"
const direct = "https://news.ycombinator.com/rss";
const wrapped = "/public/proxy/raw?url=https://export.arxiv.org/api/query?search_query=all:rl";
const appWrapped = "/apps/demo-app/__agentark/http/fetch?url=https://api.github.com/repos/openai/openai-python";
"#;

    let hosts = extract_public_proxy_target_hosts_from_text(source);

    assert!(hosts.contains("news.ycombinator.com"));
    assert!(hosts.contains("export.arxiv.org"));
    assert!(hosts.contains("api.github.com"));
}

#[test]
fn normalize_stream_heartbeat_status_collapses_model_and_memory_messages() {
    assert_eq!(
        normalize_stream_heartbeat_status("Waiting for z-ai/glm-5 to respond (15s)..."),
        "Waiting on model response. No new output yet."
    );
    assert_eq!(
        normalize_stream_heartbeat_status(
            "Vector memory active | Scope: channel:web | Channel: web"
        ),
        "Memory/context setup in progress. No new output yet."
    );
    assert_eq!(
        normalize_stream_heartbeat_status("Context Packing | Loaded 3 messages"),
        "Preparing conversation context. No new output yet."
    );
    assert_eq!(
        normalize_stream_heartbeat_status(
            "Preparing research plan with google/gemma-4-31b-it (18s elapsed)..."
        ),
        "Preparing research plan. No new output yet."
    );
    assert_eq!(
        normalize_stream_heartbeat_status(
            "Memory available on demand | Scope: channel:web | Channel: web"
        ),
        "Still processing. No new output yet."
    );
}

#[test]
fn normalize_stream_event_for_sse_suppresses_duplicate_heartbeat_updates() {
    let (first_event, first_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::Thinking("Waiting for z-ai/glm-5 to respond (5s)...".to_string()),
        "",
    );
    let Some((event_name, payload)) = first_event else {
        panic!("expected first heartbeat event");
    };
    assert_eq!(event_name, "thinking");
    assert_eq!(
        payload.get("detail").and_then(|v| v.as_str()),
        Some("Waiting on model response. No new output yet.")
    );

    let (second_event, second_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::Thinking(
            "Model z-ai/glm-5 is generating (10s elapsed)...".to_string(),
        ),
        &first_state,
    );
    assert!(second_event.is_none());
    assert_eq!(second_state, first_state);
}

#[test]
fn normalize_stream_event_for_sse_summarizes_tool_results() {
    let (event, next_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::ToolResult {
            name: "file_read".to_string(),
            content: "<!DOCTYPE html><html><head><title>Demo</title></head><body></body></html>"
                .to_string(),
        },
        "Waiting on model response. No new output yet.",
    );
    assert!(next_state.is_empty());
    let Some((event_name, payload)) = event else {
        panic!("expected tool_result event");
    };
    assert_eq!(event_name, "tool_result");
    assert_eq!(
        payload.get("content").and_then(|v| v.as_str()),
        Some("Read HTML document: Demo.")
    );
}

#[test]
fn normalize_stream_event_for_sse_preserves_structured_tool_result_fields() {
    let (event, next_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::ToolResult {
            name: "ark_inspect".to_string(),
            content: serde_json::json!({
                "matched_app": {
                    "id": "demo",
                    "title": "Demo App",
                    "local_access_url": "http://localhost:8990/apps/demo/"
                },
                "message": "Matched deployed app."
            })
            .to_string(),
        },
        "",
    );
    assert!(next_state.is_empty());
    let Some((event_name, payload)) = event else {
        panic!("expected tool_result event");
    };
    assert_eq!(event_name, "tool_result");
    assert_eq!(
        payload
            .get("matched_app")
            .and_then(|v| v.get("title"))
            .and_then(|v| v.as_str()),
        Some("Demo App")
    );
    assert_eq!(
        payload.get("content").and_then(|v| v.as_str()),
        Some("Matched app and loaded metadata for Demo App.")
    );
}

#[test]
fn normalize_stream_event_for_sse_preserves_draft_file_progress_payload() {
    let (event, next_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::ToolProgress {
            name: "app_deploy".to_string(),
            content: "Drafting src/App.tsx".to_string(),
            payload: Some(serde_json::json!({
                "kind": "draft_file",
                "file": "src/App.tsx",
                "phase": "generating_files",
                "stream_key": "draft-file:app_deploy:src/App.tsx",
                "content_snapshot": "export default function App() {}",
                "line": 1,
                "total_lines": 1,
                "done": true
            })),
        },
        "",
    );

    assert!(next_state.is_empty());
    let Some((event_name, payload)) = event else {
        panic!("expected tool_progress event");
    };
    assert_eq!(event_name, "tool_progress");
    assert_eq!(
        payload.get("kind").and_then(|v| v.as_str()),
        Some("draft_file")
    );
    assert_eq!(
        payload.get("file").and_then(|v| v.as_str()),
        Some("src/App.tsx")
    );
    assert_eq!(
        payload.get("content_snapshot").and_then(|v| v.as_str()),
        Some("export default function App() {}")
    );
}

#[test]
fn normalize_stream_event_for_sse_preserves_phase_status_progress_payload() {
    let (event, next_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::ToolProgress {
            name: "app_deploy".to_string(),
            content: "Installing dependencies".to_string(),
            payload: Some(serde_json::json!({
                "kind": "phase_status",
                "phase": "installing",
                "label": "Installing",
                "detail": "Installing dependencies",
                "elapsed_secs": 18,
                "stream_key": "phase-status:app_deploy:installing"
            })),
        },
        "",
    );

    assert!(next_state.is_empty());
    let Some((event_name, payload)) = event else {
        panic!("expected tool_progress event");
    };
    assert_eq!(event_name, "tool_progress");
    assert_eq!(
        payload.get("kind").and_then(|v| v.as_str()),
        Some("phase_status")
    );
    assert_eq!(
        payload.get("phase").and_then(|v| v.as_str()),
        Some("installing")
    );
    assert_eq!(
        payload.get("elapsed_secs").and_then(|v| v.as_u64()),
        Some(18)
    );
}

#[test]
fn normalize_stream_event_for_sse_emits_lazy_chat_task_started_payload() {
    let (event, next_state) = normalize_stream_event_for_sse(
        crate::core::StreamEvent::ChatTaskStarted {
            task_id: "task-lazy-chat".to_string(),
            description: "App task: build a static HTML page".to_string(),
            work_type: "app".to_string(),
            conversation_id: Some("conv-123".to_string()),
        },
        "",
    );

    assert!(next_state.is_empty());
    let Some((event_name, payload)) = event else {
        panic!("expected task_started event");
    };
    assert_eq!(event_name, "task_started");
    assert_eq!(
        payload.get("task_id").and_then(|value| value.as_str()),
        Some("task-lazy-chat")
    );
    assert_eq!(
        payload.get("description").and_then(|value| value.as_str()),
        Some("App task: build a static HTML page")
    );
    assert_eq!(
        payload.get("work_type").and_then(|value| value.as_str()),
        Some("app")
    );
    assert_eq!(
        payload
            .get("conversation_id")
            .and_then(|value| value.as_str()),
        Some("conv-123")
    );
    assert!(payload.get("project_id").is_none());
}

#[test]
fn backfill_chat_task_origin_metadata_sets_missing_conversation_id() {
    let mut task = crate::core::Task::new(
        "Preview".to_string(),
        "chat_request".to_string(),
        serde_json::json!({
            "_origin": "chat",
            "channel": "web",
            "conversation_id": serde_json::Value::Null,
            "project_id": serde_json::Value::Null,
        }),
    );

    let encoded =
        backfill_chat_task_origin_metadata(&mut task, Some("fresh-conversation-id"), None)
            .expect("helper should update task arguments");

    let payload: serde_json::Value =
        serde_json::from_str(&encoded).expect("updated arguments should decode");
    assert_eq!(
        payload
            .get("conversation_id")
            .and_then(|value| value.as_str()),
        Some("fresh-conversation-id")
    );
    assert_eq!(
        task.arguments
            .get("conversation_id")
            .and_then(|value| value.as_str()),
        Some("fresh-conversation-id")
    );
}

#[tokio::test]
async fn resume_chat_stream_rejects_non_chat_and_paused_tasks() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route(
            "/tasks/{id}/resume-chat/stream",
            post(resume_chat_task_stream),
        )
        .with_state(state.clone());

    let mut non_chat_task = crate::core::Task::new(
        "Daily brief".to_string(),
        "daily_brief".to_string(),
        serde_json::json!({}),
    );
    non_chat_task.status = TaskStatus::Cancelled;
    let non_chat_id = non_chat_task.id;
    add_test_task(&state, non_chat_task).await;

    let non_chat_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{}/resume-chat/stream", non_chat_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_chat_response.status(), StatusCode::CONFLICT);
    let non_chat_body = response_json(non_chat_response).await;
    assert!(non_chat_body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("Only chat-request tasks"));

    let mut paused_chat_task = crate::core::Task::new(
        "Paused chat task".to_string(),
        "chat_request".to_string(),
        serde_json::json!({
            "_task_kind": "chat_request",
            "_origin": "chat",
            "message": "hello",
            "channel": "web",
            "conversation_id": "paused-conversation",
        }),
    );
    paused_chat_task.status = TaskStatus::Paused;
    let paused_chat_id = paused_chat_task.id;
    add_test_task(&state, paused_chat_task).await;

    let paused_response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{}/resume-chat/stream", paused_chat_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(paused_response.status(), StatusCode::CONFLICT);
    let paused_body = response_json(paused_response).await;
    assert!(paused_body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("cancelled or failed"));
}

#[tokio::test]
async fn resume_chat_stream_allows_paused_plan_confirmation_tasks() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route(
            "/tasks/{id}/resume-chat/stream",
            post(resume_chat_task_stream),
        )
        .with_state(state.clone());

    let conversation_id = uuid::Uuid::new_v4().to_string();
    create_test_conversation_with_user_message(&state, &conversation_id, "hello").await;

    let preview_plan = serde_json::json!({
        "summary": "Gather sources and verify claims",
        "steps": [
            {
                "id": 1,
                "title": "Gather source sets",
                "description": "Collect official docs and recent reporting.",
                "status": "pending",
                "action": serde_json::Value::Null,
                "arguments": serde_json::json!({}),
                "tool_hint": serde_json::Value::Null
            }
        ]
    });

    let mut paused_chat_task = crate::core::Task::new(
        "Paused deep research plan".to_string(),
        "chat_request".to_string(),
        serde_json::json!({
            "_task_kind": "chat_request",
            "_origin": "chat",
            "_work_type": "research",
            "_pause_kind": "plan_confirmation",
            "_plan_preview": {
                "original_plan": preview_plan.clone(),
                "current_plan": preview_plan.clone(),
                "source": "deep_research"
            },
            "message": "hello",
            "channel": "web",
            "conversation_id": conversation_id.clone(),
            "project_id": serde_json::Value::Null,
            "deep_research": true,
            "attachments_present": false,
        }),
    );
    paused_chat_task.status = TaskStatus::Paused;
    let paused_chat_id = paused_chat_task.id;
    add_test_task(&state, paused_chat_task).await;

    let override_body = serde_json::json!({
        "plan_override": {
            "summary": "Edited deep research plan",
            "steps": [
                {
                    "title": "Compare the top sources",
                    "description": "Verify the strongest claims before answering.",
                    "action": serde_json::Value::Null,
                    "arguments": serde_json::json!({}),
                    "tool_hint": serde_json::Value::Null
                }
            ]
        }
    });

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{}/resume-chat/stream", paused_chat_id))
                .header("content-type", "application/json")
                .body(Body::from(override_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("stream body should complete")
            .to_vec(),
    )
    .expect("sse body should be utf8");
    assert!(body.contains("event: task_started"));

    let tasks = state.tasks.read().await;
    let stored_task = tasks
        .all()
        .iter()
        .find(|candidate| candidate.id == paused_chat_id)
        .expect("resumed task should remain in queue");
    assert!(!matches!(stored_task.status, TaskStatus::Paused));
    assert!(stored_task.arguments.get("_pause_kind").is_none());
    assert_eq!(
        stored_task
            .arguments
            .get("_plan_preview")
            .and_then(|value| value.get("current_plan"))
            .and_then(|value| value.get("summary"))
            .and_then(|value| value.as_str()),
        Some("Edited deep research plan")
    );
}

#[tokio::test]
async fn generic_resume_and_retry_reject_chat_request_tasks() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/tasks/{id}/resume", post(resume_task))
        .route("/tasks/{id}/retry", post(retry_task))
        .with_state(state.clone());

    let mut paused_chat_task = crate::core::Task::new(
        "Paused chat task".to_string(),
        "chat_request".to_string(),
        serde_json::json!({
            "_task_kind": "chat_request",
            "_origin": "chat",
            "message": "hello",
            "channel": "web",
            "conversation_id": "resume-conflict-conversation",
        }),
    );
    paused_chat_task.status = TaskStatus::Paused;
    let paused_chat_id = paused_chat_task.id;
    add_test_task(&state, paused_chat_task).await;

    let resume_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{}/resume", paused_chat_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resume_response.status(), StatusCode::CONFLICT);
    let resume_body = response_json(resume_response).await;
    assert!(resume_body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("resume-chat/stream"));

    let mut cancelled_chat_task = crate::core::Task::new(
        "Cancelled chat task".to_string(),
        "chat_request".to_string(),
        serde_json::json!({
            "_task_kind": "chat_request",
            "_origin": "chat",
            "message": "hello",
            "channel": "web",
            "conversation_id": "retry-conflict-conversation",
        }),
    );
    cancelled_chat_task.status = TaskStatus::Cancelled;
    let cancelled_chat_id = cancelled_chat_task.id;
    add_test_task(&state, cancelled_chat_task).await;

    let retry_response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{}/retry", cancelled_chat_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry_response.status(), StatusCode::CONFLICT);
    let retry_body = response_json(retry_response).await;
    assert!(retry_body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("resume-chat/stream"));
}

#[tokio::test]
async fn resume_chat_stream_reuses_task_and_does_not_duplicate_user_message() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route(
            "/tasks/{id}/resume-chat/stream",
            post(resume_chat_task_stream),
        )
        .with_state(state.clone());

    let conversation_id = uuid::Uuid::new_v4().to_string();
    create_test_conversation_with_user_message(&state, &conversation_id, "hello").await;

    let mut task = crate::core::Task::new(
        "Chat task: hello".to_string(),
        "chat_request".to_string(),
        serde_json::json!({
            "_task_kind": "chat_request",
            "_origin": "chat",
            "_work_type": "task",
            "message": "hello",
            "channel": "web",
            "conversation_id": conversation_id.clone(),
            "project_id": serde_json::Value::Null,
            "deep_research": false,
            "attachments_present": false,
        }),
    );
    task.status = TaskStatus::Cancelled;
    let task_id = task.id;
    add_test_task(&state, task).await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/tasks/{}/resume-chat/stream", task_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("stream body should complete")
            .to_vec(),
    )
    .expect("sse body should be utf8");
    assert!(body.contains("event: task_started"));
    assert!(body.contains("event: content"));
    assert!(body.contains("Hello! What would you like help with today?"));

    let tasks = state.tasks.read().await;
    let stored_task = tasks
        .all()
        .iter()
        .find(|candidate| candidate.id == task_id)
        .expect("resumed task should remain in queue");
    assert!(matches!(stored_task.status, TaskStatus::Completed));
    drop(tasks);

    let agent = state.agent.read().await;
    let messages = agent
        .storage
        .get_messages(&conversation_id, 20, 0)
        .await
        .expect("messages should load");
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == "user")
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == "assistant")
            .count(),
        1
    );
}

#[tokio::test]
async fn settings_endpoint_rejects_discord_without_bot_token() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/settings", post(update_settings))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/settings")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "llm_provider": "ollama",
                "llm_model": "llama3.2",
                "discord_enabled": true,
                "discord_guild_id": "guild-1",
                "discord_webhook_url": "https://discord.com/api/webhooks/123/token"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("Discord bot token is required"));
}

#[tokio::test]
async fn settings_endpoint_rejects_discord_without_scope() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/settings", post(update_settings))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/settings")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "llm_provider": "ollama",
                "llm_model": "llama3.2",
                "discord_enabled": true,
                "discord_bot_token": "discord-bot-token"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("guild, channel, or thread scope"));
}

#[tokio::test]
async fn settings_endpoint_requires_google_chat_verify_token() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/settings", post(update_settings))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/settings")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "llm_provider": "ollama",
                "llm_model": "llama3.2",
                "google_chat_enabled": true,
                "google_chat_access_token": "chat-token",
                "google_chat_space": "spaces/AAA"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("verification token"));
}

#[tokio::test]
async fn settings_endpoint_rejects_incomplete_matrix_config() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/settings", post(update_settings))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/settings")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "llm_provider": "ollama",
                "llm_model": "llama3.2",
                "matrix_enabled": true,
                "matrix_access_token": "matrix-secret"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("Matrix homeserver URL and user ID are required"));
}

#[tokio::test]
async fn profile_onboarding_endpoint_persists_answers_and_marks_complete() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/profile", get(get_profile))
        .route("/profile/onboarding", post(update_profile_onboarding))
        .with_state(state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/profile/onboarding")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "preferred_name": "Ava",
                        "timezone": "America/New_York",
                        "tone": "concise",
                        "priority_focus": "Inbox triage and daily brief follow-up"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {}", body);
    assert_eq!(
        body.get("name").and_then(|value| value.as_str()),
        Some("Ava")
    );
    assert_eq!(
        body.get("priority_focus").and_then(|value| value.as_str()),
        Some("Inbox triage and daily brief follow-up")
    );
    assert_eq!(
        body.get("onboarding_complete")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        body.get("personalization_dismissed")
            .and_then(|value| value.as_bool()),
        Some(false)
    );

    let profile = state.user_profile.read().await.clone();
    assert_eq!(profile.timezone.as_deref(), Some("America/New_York"));
    assert_eq!(profile.tone.as_deref(), Some("concise"));
    assert!(profile.onboarding_complete);

    let agent = state.agent.read().await;
    let name_pref = agent
        .storage
        .get_user_preference("user_name", None)
        .await
        .expect("user_name lookup should succeed")
        .expect("user_name should be stored");
    assert_eq!(name_pref.value, "Ava");
    let focus_pref = agent
        .storage
        .get_user_preference("assistant_priority_focus", None)
        .await
        .expect("priority focus lookup should succeed")
        .expect("priority focus should be stored");
    assert_eq!(focus_pref.value, "Inbox triage and daily brief follow-up");
}

#[tokio::test]
async fn profile_onboarding_dismiss_endpoint_hides_prompt_until_settings_are_used() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/profile", get(get_profile))
        .route(
            "/profile/onboarding/dismiss",
            post(update_profile_onboarding_dismiss),
        )
        .with_state(state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/profile/onboarding/dismiss")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {}", body);
    assert_eq!(
        body.get("personalization_dismissed")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        body.get("onboarding_complete")
            .and_then(|value| value.as_bool()),
        Some(false)
    );

    let profile = state.user_profile.read().await.clone();
    assert!(profile.personalization_dismissed);
    assert!(!profile.onboarding_complete);
}

#[tokio::test]
async fn profile_onboarding_endpoint_rejects_invalid_timezone() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/profile/onboarding", post(update_profile_onboarding))
        .with_state(state);
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/profile/onboarding")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "preferred_name": "Ava",
                        "timezone": "Mars/Olympus_Mons",
                        "tone": "concise",
                        "priority_focus": "Inbox triage"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("Invalid timezone"));
}

#[tokio::test]
async fn gateway_channels_endpoint_returns_transport_inventory() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.slack = Some(crate::channels::slack::SlackChannelConfig {
            bot_token: "xoxb-test-token".to_string(),
            signing_secret: "topsecret".to_string(),
            ..Default::default()
        });
        guard.config.teams = Some(crate::channels::teams::TeamsTransportConfig {
            service_url: "https://smba.trafficmanager.net/teams".to_string(),
            access_token: "teams-token".to_string(),
            bot_app_id: Some("teams-app-id".to_string()),
            ..Default::default()
        });
    }
    let router = Router::new()
        .route("/gateway/channels", get(gateway_control::get_channels))
        .with_state(state);
    let request = Request::builder()
        .method("GET")
        .uri("/gateway/channels")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let channels = body
        .get("channels")
        .and_then(|value| value.as_array())
        .expect("channels array");
    assert!(channels
        .iter()
        .any(|channel| { channel.get("id").and_then(|value| value.as_str()) == Some("slack") }));
    assert!(channels
        .iter()
        .any(|channel| { channel.get("id").and_then(|value| value.as_str()) == Some("teams") }));
}

#[tokio::test]
async fn slack_webhook_endpoint_rejects_unsigned_url_verification_when_secret_is_configured() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.slack = Some(crate::channels::slack::SlackChannelConfig {
            bot_token: "xoxb-test-token".to_string(),
            signing_secret: "topsecret".to_string(),
            ..Default::default()
        });
    }
    let router = Router::new()
        .route("/webhook/slack", post(slack_webhook_handler))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/webhook/slack")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "type": "url_verification",
                "challenge": "abc"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.to_ascii_lowercase().contains("signature"));
}

#[tokio::test]
async fn slack_webhook_endpoint_rejects_unsigned_event_callback_when_secret_is_configured() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.slack = Some(crate::channels::slack::SlackChannelConfig {
            bot_token: "xoxb-test-token".to_string(),
            signing_secret: "topsecret".to_string(),
            ..Default::default()
        });
    }
    let router = Router::new()
        .route("/webhook/slack", post(slack_webhook_handler))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/webhook/slack")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "type": "event_callback",
                "event_id": "evt-1",
                "team_id": "T123",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "text": "hello",
                    "channel": "C123",
                    "ts": "1710000000.000100"
                }
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.to_ascii_lowercase().contains("signature"));
}

#[tokio::test]
async fn whatsapp_webhook_endpoint_rejects_unsigned_cloud_api_requests() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
            mode: crate::channels::whatsapp::WhatsAppMode::CloudApi,
            access_token: "wa-token".to_string(),
            app_secret: "topsecret".to_string(),
            phone_number_id: "phone-id".to_string(),
            verify_token: "verify".to_string(),
            bridge_runtime: Some(crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded),
            bridge_url: "http://127.0.0.1:8999".to_string(),
            bridge_token: String::new(),
            allowed_numbers: vec![],
            dm_policy: "pairing".to_string(),
        });
    }
    let router = Router::new()
        .route("/webhook/whatsapp", post(whatsapp_webhook_handler))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/webhook/whatsapp")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "entry": [{
                    "changes": [{
                        "value": {
                            "messages": [{
                                "from": "15551234567",
                                "id": "wamid.1",
                                "type": "text",
                                "text": { "body": "hello" }
                            }]
                        }
                    }]
                }]
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn whatsapp_webhook_endpoint_rejects_bridge_requests_without_bearer_auth() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    *state.api_key.write().await = Some("bridge-secret".to_string());
    {
        let mut guard = state.agent.write().await;
        guard.config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
            mode: crate::channels::whatsapp::WhatsAppMode::Baileys,
            access_token: String::new(),
            app_secret: String::new(),
            phone_number_id: String::new(),
            verify_token: "verify".to_string(),
            bridge_runtime: Some(crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded),
            bridge_url: "http://127.0.0.1:8999".to_string(),
            bridge_token: String::new(),
            allowed_numbers: vec![],
            dm_policy: "pairing".to_string(),
        });
    }
    let router = Router::new()
        .route("/webhook/whatsapp", post(whatsapp_webhook_handler))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/webhook/whatsapp")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "_source": "baileys",
                "entry": [{
                    "changes": [{
                        "value": {
                            "messages": [{
                                "from": "15551234567",
                                "id": "wamid.1",
                                "type": "text",
                                "text": { "body": "hello" }
                            }]
                        }
                    }]
                }]
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn whatsapp_bridge_status_returns_disabled_when_whatsapp_is_off() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/api/whatsapp-bridge/status", get(whatsapp_bridge_status))
        .with_state(state);

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/whatsapp-bridge/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("disabled")
    );
    assert_eq!(
        body.get("managed_by").and_then(|value| value.as_str()),
        Some("none")
    );
}

#[tokio::test]
async fn whatsapp_bridge_status_returns_disabled_for_cloud_api_mode() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
            mode: crate::channels::whatsapp::WhatsAppMode::CloudApi,
            access_token: "wa-token".to_string(),
            app_secret: "topsecret".to_string(),
            phone_number_id: "phone-id".to_string(),
            verify_token: "verify".to_string(),
            bridge_runtime: Some(crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded),
            bridge_url: crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string(),
            bridge_token: "bridge-secret".to_string(),
            allowed_numbers: vec![],
            dm_policy: "pairing".to_string(),
        });
    }

    let router = Router::new()
        .route("/api/whatsapp-bridge/status", get(whatsapp_bridge_status))
        .with_state(state);

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/whatsapp-bridge/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("disabled")
    );
    assert_eq!(
        body.get("managed_by").and_then(|value| value.as_str()),
        Some("none")
    );
}

#[tokio::test]
async fn whatsapp_bridge_status_reports_embedded_bridge_missing_from_image() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
            mode: crate::channels::whatsapp::WhatsAppMode::Baileys,
            access_token: String::new(),
            app_secret: String::new(),
            phone_number_id: String::new(),
            verify_token: "verify".to_string(),
            bridge_runtime: Some(crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded),
            bridge_url: crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string(),
            bridge_token: "bridge-secret".to_string(),
            allowed_numbers: vec![],
            dm_policy: "pairing".to_string(),
        });
    }

    let router = Router::new()
        .route("/api/whatsapp-bridge/status", get(whatsapp_bridge_status))
        .with_state(state);

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/whatsapp-bridge/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("unavailable")
    );
    assert_eq!(
        body.get("managed_by").and_then(|value| value.as_str()),
        Some("embedded")
    );
    assert_eq!(
        body.get("installed").and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn whatsapp_bridge_status_reports_external_warning_for_legacy_bridge() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.whatsapp = Some(crate::channels::whatsapp::WhatsAppChannelConfig {
            mode: crate::channels::whatsapp::WhatsAppMode::Baileys,
            access_token: String::new(),
            app_secret: String::new(),
            phone_number_id: String::new(),
            verify_token: "verify".to_string(),
            bridge_runtime: Some(crate::channels::whatsapp::WhatsAppBridgeRuntime::External),
            bridge_url: "http://127.0.0.1:65531".to_string(),
            bridge_token: String::new(),
            allowed_numbers: vec![],
            dm_policy: "pairing".to_string(),
        });
    }

    let router = Router::new()
        .route("/api/whatsapp-bridge/status", get(whatsapp_bridge_status))
        .with_state(state);

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/whatsapp-bridge/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("unavailable")
    );
    assert_eq!(
        body.get("managed_by").and_then(|value| value.as_str()),
        Some("external")
    );
    assert!(body
        .get("warning")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("legacy configuration"));
    assert!(body.get("installed").is_none());
}

#[tokio::test]
async fn update_settings_generates_whatsapp_bridge_token_for_embedded_baileys() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/settings", post(update_settings))
        .with_state(state.clone());

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "llm_provider": "ollama",
                        "llm_model": "llama3.2",
                        "llm_base_url": "http://127.0.0.1:11434",
                        "whatsapp_enabled": true,
                        "whatsapp_mode": "baileys",
                        "whatsapp_bridge_runtime": "embedded"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {}", body);
    let guard = state.agent.read().await;
    let config = guard
        .config
        .whatsapp
        .as_ref()
        .expect("whatsapp config should exist");
    assert_eq!(
        config.mode,
        crate::channels::whatsapp::WhatsAppMode::Baileys
    );
    assert_eq!(
        config.bridge_runtime(),
        crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded
    );
    assert_eq!(
        config.bridge_url,
        crate::channels::whatsapp::EMBEDDED_BRIDGE_URL
    );
    assert!(!config.bridge_token.trim().is_empty());
}

#[tokio::test]
async fn update_settings_rejects_new_external_whatsapp_bridge_without_token() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/settings", post(update_settings))
        .with_state(state);

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "llm_provider": "ollama",
                        "llm_model": "llama3.2",
                        "llm_base_url": "http://127.0.0.1:11434",
                        "whatsapp_enabled": true,
                        "whatsapp_mode": "baileys",
                        "whatsapp_bridge_runtime": "external",
                        "whatsapp_bridge_url": "http://127.0.0.1:65531"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unexpected response: {}",
        body
    );
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("external bridge token is required"));
}

#[tokio::test]
async fn teams_webhook_endpoint_rejects_requests_without_authorization() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    {
        let mut guard = state.agent.write().await;
        guard.config.teams = Some(crate::channels::teams::TeamsTransportConfig {
            service_url: "https://smba.trafficmanager.net/teams".to_string(),
            access_token: "teams-token".to_string(),
            bot_app_id: Some("teams-app-id".to_string()),
            ..Default::default()
        });
    }
    let router = Router::new()
        .route("/webhook/teams", post(teams_webhook_handler))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/webhook/teams")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "type": "message",
                "id": "activity-1",
                "serviceUrl": "https://smba.trafficmanager.net/teams",
                "conversation": { "id": "conv-1" },
                "text": "hello"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn arkpulse_fix_rejects_command_text_without_structured_remediation() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let router = Router::new()
        .route("/arkpulse/fix", post(run_arkpulse_fix))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/arkpulse/fix")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "issue_title": "Secret exposure",
                "target": "app:test",
                "fix_command": "cd C:/tmp/app && cargo generate-lockfile"
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .contains("no executable ArkPulse auto-fix"));
}

#[tokio::test]
async fn arkpulse_fix_skips_missing_app_restart_and_cleans_stale_state() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let app_id = "cad20c5e";
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    storage
        .insert_notification(&crate::storage::entities::notification::Model {
            id: "notif-stale-app".to_string(),
            title: format!("App {} unavailable", app_id),
            body: format!("App {} failed its last probe.", app_id),
            level: "warning".to_string(),
            source: "app_status".to_string(),
            read: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .await
        .expect("notification should be inserted");
    let pulse_event = crate::sentinel::PulseEvent {
        timestamp: chrono::Utc::now().to_rfc3339(),
        status: "error".to_string(),
        message: "App root probe failed".to_string(),
        summary: String::new(),
        flags: vec!["app".to_string()],
        overdue_tasks: 0,
        failed_tasks: 0,
        details: crate::sentinel::PulseDetails {
            deployed_apps: vec![crate::sentinel::AppPulseInfo {
                id: app_id.to_string(),
                title: "arXiv Live Feed".to_string(),
                is_static: true,
                process_alive: false,
                requests_since_last_check: 0,
                idle_hours: 0,
            }],
            doctor_findings: vec![crate::sentinel::DoctorFinding {
                severity: "high".to_string(),
                category: "app".to_string(),
                target: format!("http://127.0.0.1:8990/apps/{}/", app_id),
                title: "Restart missing app".to_string(),
                evidence: "App root probe failed".to_string(),
                root_cause: "App directory no longer exists".to_string(),
                fix_command: format!("POST /api/apps/{}/restart", app_id),
                remediation: Some(crate::sentinel::DoctorRemediationSpec::AppRestart {
                    app_id: app_id.to_string(),
                }),
                user_actionable: true,
            }],
            ..crate::sentinel::PulseDetails::default()
        },
    };
    storage
        .insert_arkpulse_event(&crate::storage::arkpulse_event::Model {
            id: "pulse-stale-app".to_string(),
            timestamp: pulse_event.timestamp.clone(),
            status: pulse_event.status.clone(),
            message: pulse_event.message.clone(),
            summary: pulse_event.summary.clone(),
            flags_json: serde_json::to_string(&pulse_event.flags).expect("flags json"),
            overdue_tasks: pulse_event.overdue_tasks as i32,
            failed_tasks: pulse_event.failed_tasks as i32,
            details_json: serde_json::to_string(&pulse_event.details).expect("details json"),
        })
        .await
        .expect("pulse event should be inserted");

    let router = Router::new()
        .route("/arkpulse/fix", post(run_arkpulse_fix))
        .with_state(state);
    let request = Request::builder()
        .method("POST")
        .uri("/arkpulse/fix")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "issue_title": "App root probe failed",
                "target": format!("http://127.0.0.1:8990/apps/{}/", app_id),
                "event_timestamp": pulse_event.timestamp,
                "finding_index": 0
            })
            .to_string(),
        ))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body.get("status").and_then(|value| value.as_str()),
        Some("ok")
    );
    assert_eq!(
        body.get("skipped").and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        body.get("reason").and_then(|value| value.as_str()),
        Some("missing_on_disk")
    );
    assert_eq!(
        body.get("deleted_notifications")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        body.get("deleted_pulse_events")
            .and_then(|value| value.as_u64()),
        Some(1)
    );

    assert_eq!(
        storage
            .list_notifications(10, 0, false)
            .await
            .expect("notifications should list")
            .len(),
        0
    );
    assert_eq!(
        storage
            .count_arkpulse_events()
            .await
            .expect("pulse rows should count"),
        0
    );
}

#[test]
fn describe_arkpulse_remediation_prefers_readonly_investigation_summary() {
    let summary = describe_arkpulse_remediation(
        Some(
            &crate::sentinel::DoctorRemediationSpec::ReadonlyInvestigation {
                topic: crate::sentinel::DoctorReadonlyInvestigationTopic::MemoryCaptureHealth,
            },
        ),
        "",
    );

    assert_eq!(summary, "Review failed memory captures and model health");
}

#[test]
fn parse_set_secret_command_rejects_normalized_paraphrases() {
    assert_eq!(
        parse_set_secret_command("please save my secret OPENAI_API_KEY=abc123"),
        None
    );
    assert_eq!(
        parse_set_secret_command("please save my secrret OPENAI_API_KEY=abc123"),
        None
    );
    assert_eq!(
        parse_set_secret_command("secret set OPENAI_API_KEY abc123"),
        None
    );
}

#[test]
fn parse_autonomy_quick_command_accepts_slash_commands() {
    assert!(parse_autonomy_quick_command("can you delegate fixing login").is_none());
    match parse_autonomy_quick_command("/delegate fixing login") {
        Some(AutonomyQuickCommand::Delegate { task, .. }) => {
            assert_eq!(task, "fixing login");
        }
        other => panic!("unexpected delegate parse result: {:?}", other),
    }
    match parse_autonomy_quick_command("/rollback evt-123 read") {
        Some(AutonomyQuickCommand::Rollback {
            event_id,
            operation,
        }) => {
            assert_eq!(event_id, "evt-123");
            assert_eq!(operation.as_deref(), Some("mark_read"));
        }
        other => panic!("unexpected rollback parse result: {:?}", other),
    }
    assert!(parse_autonomy_quick_command("could you please roll back evt-123 unread").is_none());
}

#[test]
fn parse_notification_control_command_accepts_slash_commands() {
    assert!(matches!(
        parse_notification_control_command("/notifications pause"),
        Some(NotificationControlCommand::Pause24h)
    ));
    assert!(parse_notification_control_command("turn notifications off").is_none());
    assert!(matches!(
        parse_notification_control_command("/notifications resume"),
        Some(NotificationControlCommand::Resume)
    ));
    assert!(matches!(
        parse_notification_control_command("/notifications status"),
        Some(NotificationControlCommand::Status)
    ));
}

fn test_experience_run(
    id: &str,
    prompt_version: &str,
    intent_key: &str,
    success_state: &str,
    correction_state: &str,
) -> crate::storage::entities::experience_run::Model {
    crate::storage::entities::experience_run::Model {
        id: id.to_string(),
        execution_run_id: None,
        trace_id: Some(format!("trace-{id}")),
        conversation_id: None,
        project_id: None,
        channel: "chat".to_string(),
        scope: "global".to_string(),
        intent_key: intent_key.to_string(),
        task_type: Some("task".to_string()),
        request_text: None,
        tool_sequence_digest: None,
        tool_sequence_json: serde_json::json!([]),
        strategy_version: None,
        policy_version: None,
        prompt_version: Some(prompt_version.to_string()),
        model_slot: None,
        success_state: success_state.to_string(),
        correction_state: correction_state.to_string(),
        outcome_summary: None,
        failure_reason: None,
        metadata: serde_json::json!({}),
        consolidated: false,
        accepted_at: None,
        corrected_at: None,
        heuristic_reflected: false,
        heuristic_reflection_status: None,
        heuristic_reflection_attempted_at: None,
        heuristic_reflection_completed_at: None,
        heuristic_lesson_id: None,
        heuristic_reflection_error: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn test_operational_log(
    id: &str,
    event_type: &str,
    prompt_version: &str,
    payload: serde_json::Value,
    success: bool,
    latency_ms: Option<i64>,
) -> crate::storage::entities::operational_log::Model {
    crate::storage::entities::operational_log::Model {
        id: id.to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        trace_id: Some(format!("trace-{id}")),
        conversation_id: None,
        channel: "chat".to_string(),
        event_type: event_type.to_string(),
        success,
        outcome: if success {
            "ok".to_string()
        } else {
            "failed".to_string()
        },
        tool_name: None,
        latency_ms,
        arguments: None,
        payload: Some(payload.to_string()),
        strategy_version: None,
        policy_version: None,
        prompt_version: Some(prompt_version.to_string()),
        model_slot: None,
    }
}

#[test]
fn aggregate_prompt_metrics_uses_experience_and_routing_signals() {
    let version = "system_prompt_v2+prompt-candidate-a";
    let experience_runs = vec![
        test_experience_run("run-1", version, "fix_bug", "accepted", "none"),
        test_experience_run("run-2", version, "fix_bug", "failed", "none"),
    ];
    let tool_logs = vec![
        test_operational_log(
            "tool-1",
            "tool_call",
            version,
            serde_json::json!({}),
            true,
            Some(100),
        ),
        test_operational_log(
            "tool-2",
            "tool_call",
            version,
            serde_json::json!({}),
            false,
            Some(200),
        ),
    ];
    let routing_logs = vec![
        test_operational_log(
            "route-1",
            "routing_decision",
            version,
            serde_json::json!({
                "needs_delegation": true,
                "should_clarify": false
            }),
            true,
            None,
        ),
        test_operational_log(
            "route-2",
            "routing_decision",
            version,
            serde_json::json!({
                "needs_delegation": false,
                "should_clarify": true
            }),
            true,
            None,
        ),
    ];
    let llm_logs = vec![test_operational_log(
        "llm-1",
        "llm_decision",
        version,
        serde_json::json!({ "tool_calls": 3 }),
        true,
        None,
    )];

    let metrics = aggregate_prompt_metrics(&experience_runs, &tool_logs, &routing_logs, &llm_logs);

    assert_eq!(metrics.len(), 1);
    let row = &metrics[0];
    assert_eq!(row.version, version);
    assert_eq!(row.samples, 2);
    assert_eq!(row.success_rate, 0.5);
    assert_eq!(row.error_rate, 0.5);
    assert_eq!(row.routing_decisions, 2);
    assert_eq!(row.delegation_rate, 0.5);
    assert_eq!(row.clarification_rate, 0.5);
    assert_eq!(row.avg_tool_calls_per_request, 3.0);
    assert_eq!(row.tool_success_rate, 0.5);
    assert_eq!(row.p95_latency_ms, Some(200));
}

#[test]
fn normalize_evolution_dev_limit_uses_bounded_sample_sizes() {
    assert_eq!(
        normalize_evolution_dev_limit(None),
        EVOLUTION_DEV_DEFAULT_LIMIT
    );
    assert_eq!(normalize_evolution_dev_limit(Some(1)), 24);
    assert_eq!(
        normalize_evolution_dev_limit(Some(999_999)),
        EVOLUTION_DEV_MAX_LIMIT
    );
}

#[test]
fn aggregate_prompt_metrics_excludes_provisional_runs_from_resolved_samples() {
    let version = "system_prompt_v2+prompt-candidate-b";
    let experience_runs = vec![
        test_experience_run("run-1", version, "fix_bug", "provisional", "none"),
        test_experience_run("run-2", version, "fix_bug", "accepted", "none"),
    ];
    let tool_logs: Vec<crate::storage::entities::operational_log::Model> = Vec::new();
    let routing_logs: Vec<crate::storage::entities::operational_log::Model> = Vec::new();
    let llm_logs: Vec<crate::storage::entities::operational_log::Model> = Vec::new();

    let metrics = aggregate_prompt_metrics(&experience_runs, &tool_logs, &routing_logs, &llm_logs);

    assert_eq!(metrics.len(), 1);
    let row = &metrics[0];
    assert_eq!(row.samples, 1);
    assert_eq!(row.success_rate, 1.0);
    assert_eq!(row.error_rate, 0.0);
}

#[test]
fn build_prompt_insights_surfaces_end_to_end_regressions() {
    let metrics = vec![
        PromptEvolutionMetric {
            version: "system_prompt_v2+baseline".to_string(),
            samples: 40,
            success_rate: 0.80,
            error_rate: 0.20,
            p95_latency_ms: Some(420),
            routing_decisions: 40,
            delegation_rate: 0.30,
            clarification_rate: 0.05,
            avg_tool_calls_per_request: 1.20,
            tool_success_rate: 0.92,
        },
        PromptEvolutionMetric {
            version: "system_prompt_v2+candidate".to_string(),
            samples: 40,
            success_rate: 0.62,
            error_rate: 0.38,
            p95_latency_ms: Some(610),
            routing_decisions: 40,
            delegation_rate: 0.45,
            clarification_rate: 0.12,
            avg_tool_calls_per_request: 1.10,
            tool_success_rate: 0.71,
        },
    ];
    let canary_state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
        enabled: true,
        baseline_version: "system_prompt_v2+baseline".to_string(),
        candidate_version: "system_prompt_v2+candidate".to_string(),
        rollout_percent: 20,
        min_samples_per_version: 25,
        min_success_gain: 0.03,
        max_sign_test_p_value: 0.10,
        activated_at: None,
    };

    let insights = build_prompt_insights(&metrics, Some(&canary_state));

    assert!(insights
        .regressions
        .iter()
        .any(|line| line.contains("End-to-end experience success is down")));
    assert!(insights
        .regressions
        .iter()
        .any(|line| line.contains("Tool success is down")));
    assert!(insights
        .regressions
        .iter()
        .any(|line| line.contains("p95 latency regressed")));
}

#[test]
fn build_prompt_optimization_opportunities_include_change_preview() {
    let summary = PromptTelemetrySummary {
        sample_count: 8,
        p95_final_prompt_chars: 31_862,
        p95_tool_schema_chars: 59_986,
        p95_estimated_total_request_chars: 93_042,
        top_sections: vec![PromptTelemetrySectionSummary {
            section: "runtime_access_summary".to_string(),
            samples: 8,
            avg_chars: 1_640.0,
            p50_chars: 1_540,
            p95_chars: 1_962,
        }],
        ..PromptTelemetrySummary::default()
    };

    let proposals =
        build_prompt_optimization_opportunities(&summary, &PromptOptimizationReviewState::new());
    let proposal = proposals
        .iter()
        .find(|item| item.id == "prompt-opt-runtime-summary-compact")
        .expect("runtime summary proposal should exist");

    assert!(proposal
        .change_preview
        .before
        .iter()
        .any(|line| line.contains("1,962 chars")));
    assert!(proposal
        .change_preview
        .after
        .iter()
        .any(|line| line.contains("compact runtime-access profile")));
    assert!(proposal
        .change_preview
        .impact_estimate
        .iter()
        .any(|line| line.contains("6.2%")));
}

#[test]
fn build_learning_candidate_summary_includes_strategy_preview() {
    let candidate = crate::storage::learning_candidate::Model {
        id: "candidate-strategy-1".to_string(),
        candidate_type: "strategy".to_string(),
        subject_key: "pattern-1".to_string(),
        title: "Strategy candidate".to_string(),
        summary: Some("Generated from repeated procedural success.".to_string()),
        project_id: None,
        conversation_id: None,
        pattern_id: Some("pattern-1".to_string()),
        evidence_refs: serde_json::json!(["pattern-1"]),
        proposed_content: serde_json::json!({
            "version": "learned-strategy-abc123",
            "default_guidance": [
                "Prefer proven local procedures before improvising a new tool plan."
            ],
            "task_guidance": {
                "research": [
                    "When the request matches `research`, prefer the learned procedure `Investigate`.",
                    "When the environment matches, start with tools in this order: search -> fetch."
                ]
            }
        }),
        confidence: 0.88,
        approval_status: "draft".to_string(),
        review_notes: None,
        reviewed_at: None,
        approved_ref: None,
        created_at: "2026-04-20T00:00:00Z".to_string(),
        updated_at: "2026-04-20T00:00:00Z".to_string(),
    };

    let summary = build_learning_candidate_summary(&candidate, None, None);
    let preview = summary
        .get("proposed_content_preview")
        .and_then(|value| value.as_object())
        .expect("strategy preview should be present");

    assert_eq!(
        preview
            .get("strategy_version")
            .and_then(|value| value.as_str()),
        Some("learned-strategy-abc123")
    );
    assert!(preview
        .get("default_guidance")
        .and_then(|value| value.as_array())
        .expect("default guidance")
        .iter()
        .filter_map(|value| value.as_str())
        .any(|line| line.contains("Prefer proven local procedures")));
    assert!(preview
        .get("task_guidance")
        .and_then(|value| value.as_array())
        .expect("task guidance")
        .iter()
        .filter_map(|value| value.as_str())
        .any(|line| line.starts_with("research:")));
}

#[test]
fn build_learning_candidate_summary_masks_sensitive_memory_preview() {
    let candidate = crate::storage::learning_candidate::Model {
        id: "memory-candidate-1".to_string(),
        candidate_type: "memory_update".to_string(),
        subject_key: "home_base".to_string(),
        title: "Review memory update".to_string(),
        summary: Some("Replace stale location memory.".to_string()),
        project_id: None,
        conversation_id: None,
        pattern_id: None,
        evidence_refs: serde_json::json!(["capture-1"]),
        proposed_content: serde_json::json!({
            "operation_type": "update",
            "semantic_key": "home_base",
            "value": "ssh-key: super-secret-value",
            "memory_kind": "location",
            "scope": "global",
            "durability": "permanent",
            "looks_sensitive": true,
            "sensitive_reason": "credential-like content"
        }),
        confidence: 0.91,
        approval_status: "draft".to_string(),
        review_notes: None,
        reviewed_at: None,
        approved_ref: None,
        created_at: "2026-04-20T00:00:00Z".to_string(),
        updated_at: "2026-04-20T00:00:00Z".to_string(),
    };

    let summary = build_learning_candidate_summary(&candidate, None, None);
    let preview = summary
        .get("proposed_content_preview")
        .and_then(|value| value.as_object())
        .expect("memory preview should be present");

    assert_eq!(
        preview
            .get("operation_type")
            .and_then(|value| value.as_str()),
        Some("update")
    );
    assert_eq!(
        preview.get("semantic_key").and_then(|value| value.as_str()),
        Some("home_base")
    );
    assert_eq!(preview.get("value_preview"), Some(&serde_json::Value::Null));
    assert_eq!(
        preview
            .get("looks_sensitive")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

fn test_model_slot(provider: LlmProvider) -> ModelSlot {
    ModelSlot {
        id: "slot-1".to_string(),
        label: "Primary".to_string(),
        role: ModelRole::Primary,
        provider,
        enabled: true,
        capability_tier: crate::core::config::ModelCapabilityTier::Balanced,
        cost_tier: crate::core::config::ModelCostTier::Medium,
        auto_escalate: true,
        escalation_rank: 0,
        health_scope: crate::core::config::ModelHealthScope::Provider,
    }
}

#[test]
fn model_slot_api_key_reuse_requires_matching_provider_scope() {
    let existing_slot = test_model_slot(LlmProvider::OpenAI {
        api_key: "existing-key".to_string(),
        model: "gpt-5.4".to_string(),
        base_url: None,
    });

    let same_provider_request = ModelSlotRequest {
        label: "Primary".to_string(),
        role: "primary".to_string(),
        provider: "openai".to_string(),
        model: "gpt-5.4-mini".to_string(),
        base_url: None,
        api_key: None,
        clear_api_key: None,
        enabled: Some(true),
    };
    assert!(can_reuse_model_slot_api_key(&existing_slot, &same_provider_request).unwrap());

    let switched_provider_request = ModelSlotRequest {
        provider: "openrouter".to_string(),
        base_url: Some(OPENROUTER_API_BASE_URL.to_string()),
        ..same_provider_request
    };
    assert!(!can_reuse_model_slot_api_key(&existing_slot, &switched_provider_request).unwrap());
}

#[test]
fn openrouter_default_base_url_still_counts_as_same_scope() {
    let existing_slot = test_model_slot(LlmProvider::OpenAI {
        api_key: "existing-key".to_string(),
        model: "openrouter/model".to_string(),
        base_url: Some(OPENROUTER_API_BASE_URL.to_string()),
    });

    let request = ModelSlotRequest {
        label: "Primary".to_string(),
        role: "primary".to_string(),
        provider: "openrouter".to_string(),
        model: "openrouter/another-model".to_string(),
        base_url: None,
        api_key: None,
        clear_api_key: None,
        enabled: Some(true),
    };

    assert!(can_reuse_model_slot_api_key(&existing_slot, &request).unwrap());
}

#[test]
fn explicit_clear_api_key_disables_reuse_even_for_same_scope() {
    let existing_slot = test_model_slot(LlmProvider::OpenAI {
        api_key: "existing-key".to_string(),
        model: "gpt-5.4".to_string(),
        base_url: None,
    });

    let request = ModelSlotRequest {
        label: "Primary".to_string(),
        role: "primary".to_string(),
        provider: "openai".to_string(),
        model: "gpt-5.4-mini".to_string(),
        base_url: None,
        api_key: None,
        clear_api_key: Some(true),
        enabled: Some(true),
    };

    assert!(!can_reuse_model_slot_api_key(&existing_slot, &request).unwrap());
}

#[tokio::test]
async fn provider_from_model_slot_request_rejects_openrouter_without_key() {
    let request = ModelSlotRequest {
        label: "Primary".to_string(),
        role: "primary".to_string(),
        provider: "openrouter".to_string(),
        model: "z-ai/glm-5.1".to_string(),
        base_url: None,
        api_key: None,
        clear_api_key: None,
        enabled: Some(true),
    };

    let error = provider_from_model_slot_request(&request, None)
        .await
        .expect_err("openrouter should require an API key");
    assert!(error.contains("API key is required"));
}

#[tokio::test]
async fn provider_from_model_slot_request_rejects_openai_compatible_without_base_url() {
    let request = ModelSlotRequest {
        label: "Primary".to_string(),
        role: "primary".to_string(),
        provider: "openai-compatible".to_string(),
        model: "local-model".to_string(),
        base_url: None,
        api_key: None,
        clear_api_key: None,
        enabled: Some(true),
    };

    let error = provider_from_model_slot_request(&request, None)
        .await
        .expect_err("openai-compatible slots should require an explicit base URL");
    assert!(error.contains("Base URL is required"));
}

#[test]
fn analytics_range_all_maps_to_full_history_window() {
    assert!(parse_range_param(Some(&"all".to_string())) >= chrono::Duration::days(365 * 100));
}

#[test]
fn analytics_openrouter_estimate_includes_request_charge() {
    let mut prices = HashMap::new();
    prices.insert(
        "openai/gpt-4".to_string(),
        OpenRouterModelPricing {
            prompt_per_token: 0.1,
            completion_per_token: 0.2,
            request_per_request: 0.3,
        },
    );

    assert_eq!(
        estimate_cost_usd("openrouter", "openai/gpt-4", 2, 3, &prices),
        Some(1.1)
    );
}

#[test]
fn analytics_openrouter_estimate_skips_generic_openai_compatible_rows() {
    let mut prices = HashMap::new();
    prices.insert(
        "openai/gpt-4".to_string(),
        OpenRouterModelPricing {
            prompt_per_token: 0.1,
            completion_per_token: 0.2,
            request_per_request: 0.3,
        },
    );

    assert_eq!(
        estimate_cost_usd("openai-compatible", "openai/gpt-4", 2, 3, &prices),
        None
    );
}

#[tokio::test]
async fn verified_ui_session_request_accepts_same_origin_cookie() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let now = auth::unix_now_ts();
    let token = "session-token";
    state.ui_sessions.write().await.insert(
        token.to_string(),
        UiSessionRecord {
            issued_at: now,
            expires_at: now + UI_SESSION_TTL_SECS,
            last_seen_at: now,
            source: "test".to_string(),
            client_hint: None,
        },
    );

    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, HeaderValue::from_static("localhost:8990"));
    headers.insert(
        header::REFERER,
        HeaderValue::from_static("http://localhost:8990/ui/chat"),
    );
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!(
            "{}={}",
            crate::branding::SESSION_COOKIE_NAME,
            token
        ))
        .unwrap(),
    );

    assert!(
        auth::is_verified_ui_session_request(
            &state,
            &headers,
            loopback_addr(),
            state.deployment_mode,
        )
        .await
    );
}

#[tokio::test]
async fn verified_ui_session_request_rejects_app_referer() {
    let (state, _config_dir, _data_dir) = build_test_state().await;
    let now = auth::unix_now_ts();
    let token = "session-token";
    state.ui_sessions.write().await.insert(
        token.to_string(),
        UiSessionRecord {
            issued_at: now,
            expires_at: now + UI_SESSION_TTL_SECS,
            last_seen_at: now,
            source: "test".to_string(),
            client_hint: None,
        },
    );

    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, HeaderValue::from_static("localhost:8990"));
    headers.insert(
        header::REFERER,
        HeaderValue::from_static("http://localhost:8990/apps/demo"),
    );
    headers.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!(
            "{}={}",
            crate::branding::SESSION_COOKIE_NAME,
            token
        ))
        .unwrap(),
    );

    assert!(
        !auth::is_verified_ui_session_request(
            &state,
            &headers,
            loopback_addr(),
            state.deployment_mode,
        )
        .await
    );
}

#[test]
fn missing_input_detection_ignores_autonomy_attention_notifications() {
    let autonomy_attention = crate::storage::entities::notification::Model {
        id: "notif-1".to_string(),
        title: "Autonomy Needs Attention".to_string(),
        body: "Auto Mode is ASSIST | Waiting on you: 1 approvals, 1 missing input".to_string(),
        level: "warning".to_string(),
        source: "autonomy_attention".to_string(),
        read: false,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let workflow_input = crate::storage::entities::notification::Model {
        id: "notif-2".to_string(),
        title: "Missing input".to_string(),
        body: "API key required to continue.".to_string(),
        level: "warning".to_string(),
        source: "workflow_inputs".to_string(),
        read: false,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    assert!(!notification_represents_missing_input(&autonomy_attention));
    assert!(notification_represents_missing_input(&workflow_input));
}
