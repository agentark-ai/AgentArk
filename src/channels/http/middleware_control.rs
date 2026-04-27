use super::*;

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

/// Persist a security event in background without blocking the current request.
pub(super) fn spawn_security_log(
    agent: SharedAgent,
    event_type: &str,
    severity: &str,
    message: String,
    source: Option<String>,
) {
    let event_type = event_type.to_string();
    let severity = severity.to_string();
    crate::spawn_logged!("src/channels/http.rs:3632", async move {
        let log = crate::storage::security_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            event_type,
            severity,
            message,
            source,
            count: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let agent_guard = agent.read().await;
        if let Err(e) = agent_guard.storage.insert_security_log(&log).await {
            tracing::debug!("Failed to persist security log entry: {}", e);
        }
    });
}

/// Rate limit middleware applies tiered limits per route prefix.
pub(super) async fn rate_limit_middleware(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();
    let path = request.uri().path().to_string();
    let method = request.method().to_string();

    if auth::is_verified_ui_session_request(&state, request.headers(), addr, state.deployment_mode)
        .await
    {
        return next.run(request).await;
    }

    let limiter = state.tiered_rate_limiter.select_for_path(&path);

    if !limiter.check_rate_limit(&ip).await {
        state.security_events.record_rate_limit_hit();
        spawn_security_log(
            state.agent.clone(),
            "rate_limit",
            "low",
            format!("Rate limit exceeded for {} {}", method, path),
            Some(format!("ip={}", ip)),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: "Rate limit exceeded".to_string(),
            }),
        )
            .into_response();
    }

    next.run(request).await
}

pub(super) async fn metrics_middleware(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let route_path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string());
    let raw_path = request.uri().path().to_string();
    let started = Instant::now();
    let response = next.run(request).await;
    let path = route_path.unwrap_or(raw_path);
    crate::metrics::observe_http_request(
        method.as_str(),
        &path,
        response.status().as_u16(),
        started.elapsed(),
    );
    response
}

pub(super) async fn security_headers_middleware(
    State(state): State<AppState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(HEADER_X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        HEADER_X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HEADER_CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'; base-uri 'self'; object-src 'none'"),
    );
    if state.deployment_mode == DeploymentMode::InternetFacing {
        headers.insert(
            HEADER_STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

pub(super) fn validate_chat_message_size(message: &str) -> Option<Response> {
    if message.len() <= MAX_CHAT_MESSAGE_BYTES {
        return None;
    }
    Some(
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ErrorResponse {
                error: format!(
                    "Chat message exceeds the {} byte limit",
                    MAX_CHAT_MESSAGE_BYTES
                ),
            }),
        )
            .into_response(),
    )
}

pub(super) fn sanitize_content_disposition_filename(raw: &str) -> String {
    let sanitized = raw.replace(['\r', '\n', '"'], "_");
    if sanitized.trim().is_empty() {
        "download".to_string()
    } else {
        sanitized
    }
}
