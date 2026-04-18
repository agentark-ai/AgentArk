use super::*;
use crate::plugins::registry::{PluginLogsQuery, PluginUpsertRequest};

fn error_response(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn list_plugins(State(state): State<AppState>) -> Response {
    let plugins = {
        let agent = state.agent.read().await;
        agent.plugins.clone()
    };
    let registry = plugins.read().await;
    match registry.list_plugins().await {
        Ok(rows) => Json(serde_json::json!({
            "plugins": rows,
            "count": rows.len(),
            "platform_events": crate::plugins::registry::PluginRegistry::platform_events(),
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn create_plugin(
    State(state): State<AppState>,
    Json(request): Json<PluginUpsertRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let plugins = agent.plugins.clone();
    let mut guard = plugins.write().await;
    match guard.upsert_plugin(&agent.runtime, None, request).await {
        Ok(plugin) => Json(serde_json::json!({ "status": "ok", "plugin": plugin })).into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn update_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginUpsertRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let plugins = agent.plugins.clone();
    let mut guard = plugins.write().await;
    match guard
        .upsert_plugin(&agent.runtime, Some(plugin_id.as_str()), request)
        .await
    {
        Ok(plugin) => Json(serde_json::json!({ "status": "ok", "plugin": plugin })).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn delete_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let plugins = agent.plugins.clone();
    let mut guard = plugins.write().await;
    match guard
        .delete_plugin(&agent.runtime, plugin_id.as_str())
        .await
    {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn refresh_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let plugins = agent.plugins.clone();
    let mut guard = plugins.write().await;
    match guard
        .refresh_plugin(&agent.runtime, plugin_id.as_str())
        .await
    {
        Ok(plugin) => Json(serde_json::json!({ "status": "ok", "plugin": plugin })).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn test_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Response {
    let plugins = {
        let agent = state.agent.read().await;
        agent.plugins.clone()
    };
    let mut guard = plugins.write().await;
    match guard.ping_plugin(plugin_id.as_str()).await {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "result": result })).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            error_response(StatusCode::NOT_FOUND, error)
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn list_plugin_logs(
    State(state): State<AppState>,
    Query(query): Query<PluginLogsQuery>,
) -> Response {
    let plugins = {
        let agent = state.agent.read().await;
        agent.plugins.clone()
    };
    let guard = plugins.read().await;
    match guard
        .list_logs(query.plugin_id.as_deref(), query.limit.unwrap_or(40))
        .await
    {
        Ok(logs) => Json(serde_json::json!({
            "logs": logs,
            "count": logs.len(),
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Agent;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, HeaderMap, Request};
    use axum::routing::{get, post};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, RwLock};
    use tower::ServiceExt;

    #[derive(Clone, Default)]
    struct MockPluginState {
        action_payloads: Arc<Mutex<Vec<Value>>>,
        event_payloads: Arc<Mutex<Vec<(String, Value)>>>,
    }

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
                release_update_cache: Arc::new(RwLock::new(ReleaseUpdateCache::default())),
            },
            config_dir,
            data_dir,
        )
    }

    async fn json_response(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn mock_plugin_token() -> String {
        ["plugin", "-secret", "-token"].concat()
    }

    fn ensure_bearer(headers: &HeaderMap) -> Result<(), StatusCode> {
        let Some(value) = headers.get(header::AUTHORIZATION) else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        let Ok(text) = value.to_str() else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if text.trim() != format!("Bearer {}", mock_plugin_token()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(())
    }

    async fn start_mock_plugin_server() -> (String, MockPluginState, tokio::task::JoinHandle<()>) {
        async fn manifest(headers: HeaderMap) -> Response {
            if let Err(status) = ensure_bearer(&headers) {
                return status.into_response();
            }
            Json(serde_json::json!({
                "sdk_version": "agentark-plugin/v1",
                "id": "ops-plugin",
                "name": "Ops Plugin",
                "version": "1.0.0",
                "description": "Mock plugin for tests",
                "actions": [{
                    "name": "echo",
                    "title": "Echo",
                    "description": format!(
                        "Echo back text from {}",
                        crate::branding::PRODUCT_NAME
                    ),
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" }
                        }
                    },
                    "capabilities": ["plugin"]
                }],
                "events": ["webhook.received", "task.completed", "task.failed", "approval.requested"]
            }))
            .into_response()
        }

        async fn ping(headers: HeaderMap) -> Response {
            if let Err(status) = ensure_bearer(&headers) {
                return status.into_response();
            }
            Json(serde_json::json!({ "ok": true, "message": "pong" })).into_response()
        }

        async fn action_echo(
            State(state): State<MockPluginState>,
            headers: HeaderMap,
            Json(payload): Json<Value>,
        ) -> Response {
            if let Err(status) = ensure_bearer(&headers) {
                return status.into_response();
            }
            state.action_payloads.lock().await.push(payload.clone());
            let text = payload
                .get("arguments")
                .and_then(|value| value.get("text"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            Json(serde_json::json!({
                "message": format!("Echo: {}", text)
            }))
            .into_response()
        }

        async fn event_sink(
            State(state): State<MockPluginState>,
            Path(event_name): Path<String>,
            headers: HeaderMap,
            Json(payload): Json<Value>,
        ) -> Response {
            if let Err(status) = ensure_bearer(&headers) {
                return status.into_response();
            }
            state
                .event_payloads
                .lock()
                .await
                .push((event_name, payload));
            Json(serde_json::json!({ "ok": true })).into_response()
        }

        let state = MockPluginState::default();
        let app = Router::new()
            .route("/agentark/manifest", get(manifest))
            .route("/agentark/ping", get(ping))
            .route("/agentark/actions/echo", post(action_echo))
            .route("/agentark/events/{event_name}", post(event_sink))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), state, handle)
    }

    #[tokio::test]
    async fn plugin_sdk_routes_work_end_to_end() {
        let (state, config_dir, data_dir) = build_test_state().await;
        let (base_url, mock_plugin, server_handle) = start_mock_plugin_server().await;
        let router = Router::new()
            .route("/plugins", get(list_plugins).post(create_plugin))
            .route("/plugins/logs", get(list_plugin_logs))
            .route(
                "/plugins/{id}",
                axum::routing::put(update_plugin).delete(delete_plugin),
            )
            .route("/plugins/{id}/refresh", post(refresh_plugin))
            .route("/plugins/{id}/test", post(test_plugin))
            .with_state(state.clone());

        let create_request = Request::builder()
            .method("POST")
            .uri("/plugins")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "base_url": base_url,
                    "auth_mode": "bearer",
                    "token": mock_plugin_token(),
                    "subscribed_events": ["webhook.received", "task.completed"],
                })
                .to_string(),
            ))
            .unwrap();
        let create_response = router.clone().oneshot(create_request).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_body = json_response(create_response).await;
        let plugin = create_body.get("plugin").cloned().unwrap_or_default();
        let plugin_id = plugin
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        assert_eq!(plugin_id, "ops-plugin");
        assert_eq!(
            plugin
                .get("token_configured")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(!create_body.to_string().contains(&mock_plugin_token()));

        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            config_dir.path(),
            Some(data_dir.path()),
        )
        .unwrap();
        assert_eq!(
            manager
                .get_custom_secret("plugin_sdk_secret:ops-plugin")
                .unwrap(),
            Some(mock_plugin_token())
        );

        let list_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/plugins")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_body = json_response(list_response).await;
        assert_eq!(
            list_body.get("count").and_then(|value| value.as_u64()),
            Some(1)
        );
        assert!(list_body.to_string().contains("plugin__ops-plugin__echo"));

        let action_result = {
            let agent = state.agent.read().await;
            agent
                .runtime
                .execute_action(
                    "plugin__ops-plugin__echo",
                    &serde_json::json!({ "text": "hello" }),
                )
                .await
                .unwrap()
        };
        assert_eq!(action_result, "Echo: hello");

        {
            let agent = state.agent.read().await;
            agent
                .dispatch_plugin_event(
                    "task.completed",
                    serde_json::json!({
                        "task": { "id": "task-1" },
                        "result": "done"
                    }),
                )
                .await
                .unwrap();
        }

        let test_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/plugins/ops-plugin/test")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(test_response.status(), StatusCode::OK);

        let refresh_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/plugins/ops-plugin/refresh")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refresh_response.status(), StatusCode::OK);

        let logs_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/plugins/logs?plugin_id=ops-plugin&limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let logs_body = json_response(logs_response).await;
        let logs = logs_body
            .get("logs")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(logs
            .iter()
            .any(|entry| entry.get("kind").and_then(|v| v.as_str()) == Some("manifest")));
        assert!(logs
            .iter()
            .any(|entry| entry.get("kind").and_then(|v| v.as_str()) == Some("action")));
        assert!(logs
            .iter()
            .any(|entry| entry.get("kind").and_then(|v| v.as_str()) == Some("event")));
        assert!(logs
            .iter()
            .any(|entry| entry.get("kind").and_then(|v| v.as_str()) == Some("ping")));

        let action_payloads = mock_plugin.action_payloads.lock().await.clone();
        assert_eq!(action_payloads.len(), 1);
        assert_eq!(
            action_payloads[0]
                .get("arguments")
                .and_then(|value| value.get("text"))
                .and_then(|value| value.as_str()),
            Some("hello")
        );

        let event_payloads = mock_plugin.event_payloads.lock().await.clone();
        assert!(event_payloads.iter().any(|(name, payload)| {
            name == "task.completed"
                && payload
                    .get("task")
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_str())
                    == Some("task-1")
        }));

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/plugins/ops-plugin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::OK);
        {
            let agent = state.agent.read().await;
            let error = agent
                .runtime
                .execute_action(
                    "plugin__ops-plugin__echo",
                    &serde_json::json!({ "text": "bye" }),
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("Unknown action"));
        }

        server_handle.abort();
    }

    #[tokio::test]
    async fn plugin_sdk_missing_plugin_routes_return_not_found() {
        let (state, _config_dir, _data_dir) = build_test_state().await;
        let router = Router::new()
            .route(
                "/plugins/{id}",
                axum::routing::put(update_plugin).delete(delete_plugin),
            )
            .route("/plugins/{id}/refresh", post(refresh_plugin))
            .route("/plugins/{id}/test", post(test_plugin))
            .with_state(state);

        let update_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/plugins/missing-plugin")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "base_url": "https://plugins.example.com",
                            "auth_mode": "none"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_response.status(), StatusCode::NOT_FOUND);

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/plugins/missing-plugin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NOT_FOUND);

        let refresh_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/plugins/missing-plugin/refresh")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refresh_response.status(), StatusCode::NOT_FOUND);

        let test_response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/plugins/missing-plugin/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(test_response.status(), StatusCode::NOT_FOUND);
    }
}
