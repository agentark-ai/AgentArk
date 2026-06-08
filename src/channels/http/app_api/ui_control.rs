use super::*;

/// Serve the compiled V2 web UI. Issues the session cookie only for trusted UI bootstrap flows.
pub(super) async fn web_ui(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut response = if let Some(index_html) = read_frontend_index_html().await {
        Html(index_html).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "UI assets are missing. Build frontend assets to continue.",
        )
            .into_response()
    };
    if auth::should_issue_ui_session_cookie(&state, &headers, addr).await {
        let session_token = generate_ephemeral_token();
        {
            let now = auth::unix_now_ts();
            let mut sessions = state.ui_sessions.write().await;
            sessions.retain(|_, record| record.expires_at > now);
            sessions.insert(
                session_token.clone(),
                UiSessionRecord {
                    issued_at: now,
                    expires_at: now + UI_SESSION_TTL_SECS,
                    last_seen_at: now,
                    source: "ui_navigation".to_string(),
                    client_hint: headers
                        .get(header::USER_AGENT)
                        .and_then(|value| value.to_str().ok())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.chars().take(160).collect()),
                },
            );
        }
        auth::apply_session_cookie(
            &mut response,
            Some(session_token.as_str()),
            state.cookie_secure_default || auth::is_https_forwarded(&headers),
        );
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL_FRONTEND_HTML),
    );
    response
}

/// Serve the compiled V2 UI directly.
pub(super) async fn web_ui_v2(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut response = if let Some(index_html) = read_frontend_index_html().await {
        Html(index_html).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "UI assets are missing. Build frontend assets to continue.",
        )
            .into_response()
    };
    if auth::should_issue_ui_session_cookie(&state, &headers, addr).await {
        let session_token = generate_ephemeral_token();
        {
            let now = auth::unix_now_ts();
            let mut sessions = state.ui_sessions.write().await;
            sessions.retain(|_, record| record.expires_at > now);
            sessions.insert(
                session_token.clone(),
                UiSessionRecord {
                    issued_at: now,
                    expires_at: now + UI_SESSION_TTL_SECS,
                    last_seen_at: now,
                    source: "ui_navigation".to_string(),
                    client_hint: headers
                        .get(header::USER_AGENT)
                        .and_then(|value| value.to_str().ok())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.chars().take(160).collect()),
                },
            );
        }
        auth::apply_session_cookie(
            &mut response,
            Some(session_token.as_str()),
            state.cookie_secure_default || auth::is_https_forwarded(&headers),
        );
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL_FRONTEND_HTML),
    );
    response
}

pub(super) fn normalize_host_for_compare(raw: &str) -> String {
    let host = raw.trim().trim_matches('"').trim_end_matches('.');
    if host.is_empty() {
        return String::new();
    }
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            return host[1..end].to_ascii_lowercase();
        }
    }
    if let Some(idx) = host.rfind(':') {
        let left = &host[..idx];
        if !left.is_empty() && !left.contains(':') {
            return left.to_ascii_lowercase();
        }
    }
    host.to_ascii_lowercase()
}

pub(super) fn extract_request_host(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())?;
    let first = raw.split(',').next()?.trim();
    let normalized = normalize_host_for_compare(first);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(super) fn extract_request_origin(headers: &HeaderMap) -> Option<String> {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("http");
    let authority = extract_request_authority(headers)?;
    normalize_origin(&format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.trim()
    ))
}

pub(super) fn extract_request_authority(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .map(|value| value.trim_matches('"').trim_end_matches('.'))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn oauth_redirect_uri_for_request(
    state: &AppState,
    headers: &HeaderMap,
    explicit_redirect_uri: Option<&str>,
) -> std::result::Result<String, String> {
    if let Some(raw) = explicit_redirect_uri
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let parsed = reqwest::Url::parse(raw)
            .map_err(|error| format!("Invalid OAuth redirect URI '{}': {}", raw, error))?;
        if state.deployment_mode == DeploymentMode::InternetFacing
            && !parsed.scheme().eq_ignore_ascii_case("https")
        {
            return Err("OAuth redirect URIs must use HTTPS in internet-facing mode.".to_string());
        }
        return Ok(parsed.to_string());
    }

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(if state.cookie_secure_default {
            "https"
        } else {
            "http"
        });
    if state.deployment_mode == DeploymentMode::InternetFacing
        && !scheme.eq_ignore_ascii_case("https")
    {
        return Err("OAuth callbacks must arrive over HTTPS in internet-facing mode.".to_string());
    }

    let authority = extract_request_authority(headers).or_else(|| {
        if state.deployment_mode == DeploymentMode::TrustedLocal {
            Some("localhost:8990".to_string())
        } else {
            None
        }
    });
    let Some(authority) = authority else {
        return Err(
            "Could not determine the external host for OAuth callback routing.".to_string(),
        );
    };
    Ok(format!(
        "{}://{}/oauth/callback",
        scheme.to_ascii_lowercase(),
        authority.trim_end_matches('/')
    ))
}

pub(super) fn request_matches_active_tunnel(headers: &HeaderMap, tunnel_url: Option<&str>) -> bool {
    let Some(request_host) = extract_request_host(headers) else {
        return false;
    };

    let Some(url) = tunnel_url else {
        return false;
    };
    if let Ok(parsed) = reqwest::Url::parse(url) {
        if let Some(tunnel_host) = parsed.host_str() {
            return normalize_host_for_compare(tunnel_host) == request_host;
        }
    }
    false
}

pub(super) fn redirect_to_selected_tunnel_app(app_id: &str) -> Response {
    let location = format!("/apps/{}/", app_id);
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(axum::body::Body::empty())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub(super) fn public_app_id_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/apps/")?;
    let app_id = rest.split('/').next().unwrap_or_default().trim();
    if is_valid_app_id(app_id) {
        Some(app_id.to_string())
    } else {
        None
    }
}

pub(super) fn is_public_app_tunnel_path(path: &str, exposed_app_ids: &HashSet<String>) -> bool {
    if path == "/public/proxy/raw" || path == "/public/proxy/raw/" {
        return !exposed_app_ids.is_empty();
    }
    if path == "/apps" || path == "/apps/" {
        return exposed_app_ids.len() == 1;
    }
    public_app_id_from_path(path)
        .as_deref()
        .is_some_and(|app_id| exposed_app_ids.contains(app_id))
}

pub(super) async fn tunnel_exposure_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();

    let (tunnel_url, selected_app_id, exposed_app_ids, control_plane_enabled, companion_enabled) = {
        let tunnel = state.tunnel.read().await;
        let mut exposed_app_ids = tunnel.exposed_app_ids.clone();
        if let Some(app_id) = tunnel
            .selected_app_id
            .as_deref()
            .filter(|id| is_valid_app_id(id))
        {
            exposed_app_ids.insert(app_id.to_string());
        }
        (
            tunnel.url.clone(),
            tunnel.selected_app_id.clone(),
            exposed_app_ids,
            tunnel.control_plane_enabled,
            tunnel.companion_enabled,
        )
    };

    if !request_matches_active_tunnel(request.headers(), tunnel_url.as_deref()) {
        return next.run(request).await;
    }

    if path == "/health" || path == "/readiness" {
        return StatusCode::NOT_FOUND.into_response();
    }

    if companion_enabled && (path == "/companion/ws" || path == "/companion/web") {
        return next.run(request).await;
    }

    if !exposed_app_ids.is_empty() {
        let selected_app_id = selected_app_id
            .as_deref()
            .filter(|id| exposed_app_ids.contains(*id))
            .map(ToString::to_string)
            .or_else(|| {
                let mut ids: Vec<String> = exposed_app_ids.iter().cloned().collect();
                ids.sort();
                ids.into_iter().next()
            });
        if path == "/"
            || path == "/ui"
            || path == "/ui/"
            || path == "/ui/v2"
            || path == "/apps"
            || path == "/apps/"
        {
            if let Some(app_id) = selected_app_id.as_deref() {
                return redirect_to_selected_tunnel_app(app_id);
            }
            return StatusCode::NOT_FOUND.into_response();
        }
        if is_public_app_tunnel_path(path, &exposed_app_ids) {
            return next.run(request).await;
        }

        return StatusCode::NOT_FOUND.into_response();
    }

    if !control_plane_enabled {
        if companion_enabled && (path == "/companion/ws" || path == "/companion/web") {
            return next.run(request).await;
        }
        return StatusCode::NOT_FOUND.into_response();
    }

    if tunnel_auth::is_public_tunnel_login_path(path)
        || tunnel_auth::is_public_tunnel_login_asset_path(path)
    {
        return next.run(request).await;
    }

    if tunnel_auth::is_control_plane_tunnel_authenticated(&state, request.headers()).await {
        return next.run(request).await;
    }

    if request.method() == Method::GET || request.method() == Method::HEAD {
        return tunnel_auth::redirect_to_tunnel_login(request.uri());
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Remote tunnel login required".to_string(),
        }),
    )
        .into_response()
}
