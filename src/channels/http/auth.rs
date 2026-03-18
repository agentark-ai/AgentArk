use super::*;
use std::net::IpAddr;
use std::sync::OnceLock;

const LOCAL_UI_BOOTSTRAP_TTL_SECS: i64 = 2 * 60;
const LOCAL_UI_BOOTSTRAP_MAX_TOKENS: usize = 128;

#[derive(Debug, Deserialize)]
pub(super) struct LocalUiBootstrapRequest {
    token: String,
}

pub(super) fn unix_now_ts() -> i64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn mask_api_key_value(key: &str) -> String {
    if key.len() > 8 {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    } else if key.is_empty() {
        String::new()
    } else {
        "****".to_string()
    }
}

pub(super) async fn sync_http_api_key_state(
    state: &AppState,
    force_refresh: bool,
) -> std::result::Result<(Option<crate::core::config::HttpApiKeyInfo>, bool), String> {
    let now = unix_now_ts();
    if !force_refresh {
        let cached_key = state.api_key.read().await.clone();
        let cached_exp = *state.api_key_expires_at.read().await;
        if let (Some(key), Some(expires_at)) = (cached_key, cached_exp) {
            if expires_at > now {
                let mut session_guard = state.session_token.write().await;
                if session_guard.is_none() {
                    *session_guard = Some(generate_ephemeral_token());
                }
                return Ok((
                    Some(crate::core::config::HttpApiKeyInfo {
                        key,
                        issued_at: now,
                        expires_at,
                    }),
                    false,
                ));
            }
        }
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let secure_config =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))
            .map_err(|e| format!("Config error: {}", e))?;
    let (info, rotated) = secure_config
        .ensure_api_key_info()
        .map_err(|e| format!("Failed to sync API key state: {}", e))?;

    {
        let mut key_guard = state.api_key.write().await;
        *key_guard = info.as_ref().map(|k| k.key.clone());
    }
    {
        let mut exp_guard = state.api_key_expires_at.write().await;
        *exp_guard = info.as_ref().map(|k| k.expires_at);
    }
    {
        let mut agent = state.agent.write().await;
        agent.api_key = info.as_ref().map(|k| k.key.clone());
    }
    {
        let mut session_guard = state.session_token.write().await;
        if info.is_some()
            || state.deployment_mode == crate::core::config::DeploymentMode::TrustedLocal
        {
            if session_guard.is_none() {
                *session_guard = Some(generate_ephemeral_token());
            }
        } else {
            *session_guard = None;
        }
    }

    Ok((info, rotated))
}

pub(super) async fn current_ui_session_token(state: &AppState) -> Option<String> {
    state.session_token.read().await.clone()
}

async fn create_local_ui_bootstrap_token(
    state: &AppState,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> Option<String> {
    if !state.local_ui_bootstrap_enabled || !is_trusted_local_ui_request(headers, addr) {
        return None;
    }
    let (info, _rotated) = sync_http_api_key_state(state, false).await.ok()?;
    if info.is_none() && state.deployment_mode != crate::core::config::DeploymentMode::TrustedLocal
    {
        return None;
    }

    let now = unix_now_ts();
    let token = generate_ephemeral_token();
    let mut tokens = state.local_ui_bootstrap_tokens.write().await;
    tokens.retain(|_, expires_at| *expires_at > now);
    if tokens.len() >= LOCAL_UI_BOOTSTRAP_MAX_TOKENS {
        tokens.clear();
    }
    tokens.insert(token.clone(), now + LOCAL_UI_BOOTSTRAP_TTL_SECS);
    Some(token)
}

async fn consume_local_ui_bootstrap_token(state: &AppState, token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return false;
    }
    let now = unix_now_ts();
    let mut tokens = state.local_ui_bootstrap_tokens.write().await;
    tokens.retain(|_, expires_at| *expires_at > now);
    match tokens.remove(trimmed) {
        Some(expires_at) if expires_at > now => true,
        _ => false,
    }
}

pub(super) fn extract_bearer_api_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

pub(super) fn has_valid_bearer_api_key(headers: &HeaderMap, expected_key: Option<&str>) -> bool {
    let Some(expected_key) = expected_key else {
        return false;
    };
    extract_bearer_api_key(headers)
        .as_deref()
        .is_some_and(|provided| provided == expected_key)
}

pub(super) fn has_valid_ui_session_cookie(
    headers: &HeaderMap,
    session_token: Option<&str>,
) -> bool {
    let Some(session_token) = session_token else {
        return false;
    };
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    for part in cookies.split(';') {
        if let Some(value) = part.trim().strip_prefix("agentark_session=") {
            if value == session_token {
                return true;
            }
        }
    }
    false
}

pub(super) fn is_local_request_host(headers: &HeaderMap) -> bool {
    extract_request_host(headers).is_some_and(|host| {
        host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "::1"
    })
}

pub(super) fn is_trusted_local_ui_request(headers: &HeaderMap, addr: SocketAddr) -> bool {
    if !is_local_request_host(headers) {
        return false;
    }
    let ip = addr.ip();
    ip.is_loopback() || is_container_host_gateway_ip(ip)
}

fn is_container_host_gateway_ip(ip: IpAddr) -> bool {
    static CONTAINER_GATEWAY_IP: OnceLock<Option<IpAddr>> = OnceLock::new();
    *CONTAINER_GATEWAY_IP.get_or_init(detect_container_default_gateway_ip) == Some(ip)
}

fn detect_container_default_gateway_ip() -> Option<IpAddr> {
    if let Ok(value) = std::env::var("AGENTARK_DOCKER_HOST_GATEWAY_IP") {
        if let Ok(parsed) = value.trim().parse::<IpAddr>() {
            return Some(parsed);
        }
    }

    let routes = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in routes.lines().skip(1) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 || fields[1] != "00000000" {
            continue;
        }
        let gateway = u32::from_str_radix(fields[2], 16).ok()?;
        let octets = gateway.to_le_bytes();
        return Some(IpAddr::V4(std::net::Ipv4Addr::new(
            octets[0], octets[1], octets[2], octets[3],
        )));
    }
    None
}

pub(super) async fn should_issue_ui_session_cookie(
    state: &AppState,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> bool {
    if is_trusted_local_ui_request(headers, addr) {
        return true;
    }

    let expected_key = match sync_http_api_key_state(state, false).await {
        Ok((info, _)) => info.map(|k| k.key),
        Err(_) => None,
    };
    has_valid_bearer_api_key(headers, expected_key.as_deref())
}

pub(super) async fn issue_oauth_state(state: &AppState, service_id: &str) -> String {
    let token = generate_ephemeral_token();
    let now = unix_now_ts();
    let mut oauth_states = state.oauth_states.write().await;
    oauth_states.retain(|_, entry| entry.expires_at > now);
    if oauth_states.len() > 1024 {
        oauth_states.clear();
    }
    oauth_states.insert(
        token.clone(),
        PendingOAuthState {
            service_id: service_id.to_string(),
            expires_at: now + OAUTH_STATE_TTL_SECS,
        },
    );
    token
}

pub(super) async fn consume_oauth_state(state: &AppState, state_token: &str) -> Option<String> {
    let now = unix_now_ts();
    let mut oauth_states = state.oauth_states.write().await;
    oauth_states.retain(|_, entry| entry.expires_at > now);
    oauth_states
        .remove(state_token)
        .and_then(|entry| (entry.expires_at > now).then_some(entry.service_id))
}

pub(super) async fn auth_middleware(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    let expected_key = match sync_http_api_key_state(&state, false).await {
        Ok((info, _rotated)) => info.map(|k| k.key),
        Err(e) => {
            state
                .security_events
                .auth_failures
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            spawn_security_log(
                state.agent.clone(),
                "auth_failure",
                "high",
                format!(
                    "Failed to validate API key state for {} {}: {}",
                    method, path, e
                ),
                Some(format!("ip={}", ip)),
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "API authentication is temporarily unavailable.".to_string(),
                }),
            )
                .into_response();
        }
    };

    // If API key is missing, fail closed unless explicitly overridden.
    let Some(expected_key) = expected_key else {
        let session_token = current_ui_session_token(&state).await;
        if state.deployment_mode == crate::core::config::DeploymentMode::TrustedLocal
            && is_trusted_local_ui_request(request.headers(), addr)
            && has_valid_ui_session_cookie(request.headers(), session_token.as_deref())
        {
            return next.run(request).await;
        }

        if state.allow_insecure_no_auth {
            if !MISSING_API_KEY_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "Protected routes are running without API auth because AGENTARK_INSECURE_NO_AUTH=true"
                );
            }
            return next.run(request).await;
        }

        if !MISSING_API_KEY_WARNED.swap(true, Ordering::Relaxed) {
            tracing::error!("Blocking protected routes because HTTP API key is not configured");
        }
        state
            .security_events
            .auth_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        spawn_security_log(
            state.agent.clone(),
            "auth_failure",
            "high",
            format!(
                "Blocked {} {} because HTTP API key is not configured",
                method, path
            ),
            Some(format!("ip={}", ip)),
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: if state.deployment_mode
                    == crate::core::config::DeploymentMode::TrustedLocal
                {
                    "API authentication is not configured. Regenerate an API key from a trusted session, or set AGENTARK_INSECURE_NO_AUTH=true temporarily."
                        .to_string()
                } else {
                    "API authentication is not configured. Regenerate an API key from a trusted session."
                        .to_string()
                },
            }),
        )
            .into_response();
    };

    if has_valid_bearer_api_key(request.headers(), Some(expected_key.as_str())) {
        return next.run(request).await;
    }

    if extract_bearer_api_key(request.headers()).is_some() {
        state
            .security_events
            .auth_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        spawn_security_log(
            state.agent.clone(),
            "auth_failure",
            "medium",
            format!("Invalid API key for {} {}", method, path),
            Some(format!("ip={}; auth=bearer", ip)),
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid API key".to_string(),
            }),
        )
            .into_response();
    }

    let session_token = current_ui_session_token(&state).await;
    if has_valid_ui_session_cookie(request.headers(), session_token.as_deref()) {
        return next.run(request).await;
    }

    state
        .security_events
        .auth_failures
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    spawn_security_log(
        state.agent.clone(),
        "auth_failure",
        "medium",
        format!("Missing or invalid credentials for {} {}", method, path),
        Some(format!("ip={}", ip)),
    );
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Missing Authorization: Bearer <api_key> header".to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn bootstrap_ui_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let expected_key = match sync_http_api_key_state(&state, false).await {
        Ok((info, _rotated)) => info.map(|k| k.key),
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: format!("API authentication is temporarily unavailable: {}", e),
                }),
            )
                .into_response()
        }
    };

    let Some(expected_key) = expected_key else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "API authentication is not configured.".to_string(),
            }),
        )
            .into_response();
    };

    if !has_valid_bearer_api_key(&headers, Some(expected_key.as_str())) {
        let message = if extract_bearer_api_key(&headers).is_some() {
            "Invalid API key"
        } else {
            "Missing Authorization: Bearer <api_key> header"
        };
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: message.to_string(),
            }),
        )
            .into_response();
    }

    let mut response =
        (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response();
    let session_token = current_ui_session_token(&state).await;
    apply_session_cookie(
        &mut response,
        session_token.as_ref(),
        state.cookie_secure_default || is_https_forwarded(&headers),
    );
    response
}

pub(super) async fn issue_local_ui_bootstrap_token(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !state.local_ui_bootstrap_enabled || !is_trusted_local_ui_request(&headers, addr) {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Local seamless bootstrap is only available from localhost access."
                    .to_string(),
            }),
        )
            .into_response();
    }

    match create_local_ui_bootstrap_token(&state, &headers, addr).await {
        Some(token) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "token": token,
                "expires_in": LOCAL_UI_BOOTSTRAP_TTL_SECS,
            })),
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Local UI bootstrap is temporarily unavailable.".to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn bootstrap_local_ui_session(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<LocalUiBootstrapRequest>,
) -> Response {
    if !state.local_ui_bootstrap_enabled || !is_trusted_local_ui_request(&headers, addr) {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Local seamless bootstrap is only available from localhost access."
                    .to_string(),
            }),
        )
            .into_response();
    }

    let (info, _rotated) = match sync_http_api_key_state(&state, false).await {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: format!("API authentication is temporarily unavailable: {}", e),
                }),
            )
                .into_response();
        }
    };
    if info.is_none() && state.deployment_mode != crate::core::config::DeploymentMode::TrustedLocal
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "API authentication is not configured.".to_string(),
            }),
        )
            .into_response();
    }

    if !consume_local_ui_bootstrap_token(&state, &request.token).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid or expired local bootstrap token.".to_string(),
            }),
        )
            .into_response();
    }

    let mut response =
        (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response();
    let session_token = current_ui_session_token(&state).await;
    apply_session_cookie(
        &mut response,
        session_token.as_ref(),
        state.cookie_secure_default || is_https_forwarded(&headers),
    );
    response
}

pub(super) fn is_https_forwarded(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

pub(super) fn apply_session_cookie(response: &mut Response, token: Option<&String>, secure: bool) {
    if let Some(token) = token {
        let secure_attr = if secure { "; Secure" } else { "" };
        let cookie = format!(
            "agentark_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400{}",
            token, secure_attr
        );
        if let Ok(val) = cookie.parse() {
            response.headers_mut().insert(header::SET_COOKIE, val);
        }
    }
}
