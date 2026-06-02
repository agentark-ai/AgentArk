use super::*;

pub(super) const HEADER_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const DEFAULT_ORDINARY_ROUTE_TIMEOUT_SECS: u64 = 10 * 60;

#[derive(Clone, Debug)]
pub(super) struct HttpRequestId(pub String);

#[derive(Debug, Serialize)]
struct RequestErrorResponse {
    error: String,
    request_id: String,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

fn request_id_from_headers(headers: &HeaderMap) -> String {
    headers
        .get(HEADER_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

fn json_response_with_request_id(
    status: StatusCode,
    error: &'static str,
    request_id: &str,
) -> Response {
    (
        status,
        Json(RequestErrorResponse {
            error: error.to_string(),
            request_id: request_id.to_string(),
        }),
    )
        .into_response()
}

fn sanitized_internal_error_response(response: Response, request_id: &str) -> Response {
    if response.status() != StatusCode::INTERNAL_SERVER_ERROR {
        return response;
    }

    tracing::warn!(
        request_id = %request_id,
        "Sanitized unexpected internal HTTP error response"
    );

    let (mut parts, _body) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let body = serde_json::to_vec(&RequestErrorResponse {
        error: "Internal server error".to_string(),
        request_id: request_id.to_string(),
    })
    .unwrap_or_else(|_| {
        format!(
            r#"{{"error":"Internal server error","request_id":"{}"}}"#,
            request_id
        )
        .into_bytes()
    });
    Response::from_parts(parts, axum::body::Body::from(body))
}

fn should_log_request_completed(status: StatusCode) -> bool {
    status.is_client_error() || status.is_server_error()
}

pub(super) async fn request_id_and_error_middleware(mut request: Request, next: Next) -> Response {
    let request_id = request_id_from_headers(request.headers());
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    request
        .extensions_mut()
        .insert(HttpRequestId(request_id.clone()));

    let mut response = sanitized_internal_error_response(next.run(request).await, &request_id);
    response.headers_mut().insert(
        HEADER_REQUEST_ID,
        HeaderValue::from_str(&request_id)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
    );
    let status = response.status();
    if should_log_request_completed(status) {
        if status.is_server_error() {
            tracing::warn!(
                request_id = %request_id,
                method = %method,
                path = %path,
                status = status.as_u16(),
                "HTTP request completed"
            );
        } else {
            tracing::debug!(
                request_id = %request_id,
                method = %method,
                path = %path,
                status = status.as_u16(),
                "HTTP request completed"
            );
        }
    }
    response
}

fn configured_ordinary_route_timeout() -> Option<Duration> {
    Some(Duration::from_secs(DEFAULT_ORDINARY_ROUTE_TIMEOUT_SECS))
}

fn has_websocket_upgrade(headers: &HeaderMap) -> bool {
    let connection_upgrade = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket_upgrade = headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    connection_upgrade && websocket_upgrade
}

fn accepts_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().starts_with("text/event-stream"))
        })
        .unwrap_or(false)
}

fn path_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn route_has_terminal_segment(path: &str, terminal: &str) -> bool {
    path_segments(path)
        .last()
        .is_some_and(|segment| segment.trim_matches('*') == terminal)
}

fn route_contains_segment(path: &str, target: &str) -> bool {
    path_segments(path)
        .into_iter()
        .any(|segment| segment.trim_matches('*') == target)
}

fn route_timeout_exempt_by_structured_path(path: &str) -> bool {
    // Classify only protocol/API route metadata. Do not inspect user-authored text.
    if route_has_terminal_segment(path, "stream")
        || route_has_terminal_segment(path, "events")
        || route_contains_segment(path, "proxy")
        || route_contains_segment(path, "apps")
    {
        return true;
    }

    matches!(
        path,
        "/chat"
            | "/tasks/plan"
            | "/tasks/{id}/resume"
            | "/autonomy/skills/execute"
            | "/autonomy/incidents/{id}/execute"
            | "/settings/evolution/dev/action"
            | "/reflect/refresh"
            | "/arkpulse/trigger"
            | "/arkpulse/fix"
            | "/arkpulse/cleanup-preview"
            | "/arkpulse/cleanup"
            | "/code/execute"
            | "/skills/{name}/test"
            | "/skills/import"
            | "/skills/marketplaces/{id}/refresh"
    )
}

fn ordinary_route_timeout_applies(request: &Request) -> bool {
    if has_websocket_upgrade(request.headers()) || accepts_event_stream(request.headers()) {
        return false;
    }

    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str())
        .unwrap_or_else(|| request.uri().path());
    !route_timeout_exempt_by_structured_path(path)
}

pub(super) async fn ordinary_route_timeout_middleware(request: Request, next: Next) -> Response {
    match configured_ordinary_route_timeout() {
        Some(timeout) => {
            ordinary_route_timeout_middleware_with_timeout(request, next, timeout).await
        }
        None => next.run(request).await,
    }
}

pub(super) async fn ordinary_route_timeout_middleware_with_timeout(
    request: Request,
    next: Next,
    timeout: Duration,
) -> Response {
    if !ordinary_route_timeout_applies(&request) {
        return next.run(request).await;
    }

    let request_id = request
        .extensions()
        .get::<HttpRequestId>()
        .map(|value| value.0.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!(
                request_id = %request_id,
                timeout_ms = timeout.as_millis(),
                "HTTP ordinary route timed out"
            );
            json_response_with_request_id(
                StatusCode::GATEWAY_TIMEOUT,
                "Request timed out",
                &request_id,
            )
        }
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
    headers.insert(
        HEADER_REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HEADER_PERMISSIONS_POLICY,
        HeaderValue::from_static(
            "camera=(self), microphone=(self), geolocation=(), payment=(), usb=()",
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, middleware, routing::get, Router};
    use tower::ServiceExt;

    async fn internal_error() -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database password leaked through anyhow chain".to_string(),
            }),
        )
            .into_response()
    }

    async fn validation_error() -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid widget id".to_string(),
            }),
        )
            .into_response()
    }

    #[test]
    fn request_completion_logging_suppresses_successful_responses() {
        for status in [
            StatusCode::OK,
            StatusCode::CREATED,
            StatusCode::NO_CONTENT,
            StatusCode::NOT_MODIFIED,
            StatusCode::TEMPORARY_REDIRECT,
        ] {
            assert!(!should_log_request_completed(status));
        }
    }

    #[test]
    fn request_completion_logging_keeps_error_responses() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
        ] {
            assert!(should_log_request_completed(status));
        }
    }

    #[tokio::test]
    async fn request_id_middleware_sanitizes_500_but_preserves_4xx_messages() {
        let app = Router::new()
            .route("/boom", get(internal_error))
            .route("/bad", get(validation_error))
            .layer(middleware::from_fn(request_id_and_error_middleware));

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/boom")
                    .header(HEADER_REQUEST_ID, "req-test-1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers().get(HEADER_REQUEST_ID).unwrap(),
            "req-test-1"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "Internal server error");
        assert_eq!(payload["request_id"], "req-test-1");
        assert!(!String::from_utf8_lossy(&body).contains("database password"));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/bad")
                    .header(HEADER_REQUEST_ID, "req-test-2")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers().get(HEADER_REQUEST_ID).unwrap(),
            "req-test-2"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "Invalid widget id");
    }

    #[tokio::test]
    async fn ordinary_timeout_skips_stream_routes_and_times_out_regular_routes() {
        async fn slow() -> Response {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Json(serde_json::json!({ "status": "ok" })).into_response()
        }

        let app = Router::new()
            .route("/status", get(slow))
            .route("/runs/abc/stream", get(slow))
            .layer(middleware::from_fn(|request, next| async move {
                ordinary_route_timeout_middleware_with_timeout(
                    request,
                    next,
                    std::time::Duration::from_millis(1),
                )
                .await
            }))
            .layer(middleware::from_fn(request_id_and_error_middleware));

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/status")
                    .header(HEADER_REQUEST_ID, "req-timeout")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["request_id"], "req-timeout");

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/runs/abc/stream")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
