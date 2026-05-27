use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
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
    if info.is_none() && state.deployment_mode != crate::core::config::DeploymentMode::TrustedLocal
    {
        state.ui_sessions.write().await.clear();
    }

    Ok((info, rotated))
}

async fn create_local_ui_bootstrap_token(
    state: &AppState,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> Option<String> {
    if !state.local_ui_bootstrap_enabled || !is_trusted_local_ui_api_request(headers, addr) {
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
    matches!(tokens.remove(trimmed), Some(expires_at) if expires_at > now)
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
        .is_some_and(|provided| {
            crate::security::constant_time_eq(provided.as_bytes(), expected_key.as_bytes())
        })
}

fn extract_ui_session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookies = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    for part in cookies.split(';') {
        if let Some(value) = part
            .trim()
            .strip_prefix(&format!("{}=", crate::branding::SESSION_COOKIE_NAME))
        {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn session_client_hint(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(160).collect())
}

async fn create_ui_session(state: &AppState, headers: &HeaderMap, source: &str) -> String {
    let now = unix_now_ts();
    let mut sessions = state.ui_sessions.write().await;
    sessions.retain(|_, record| record.expires_at > now);
    while sessions.len() >= crate::channels::http::UI_SESSION_MAX_TRACKED {
        let Some(oldest_token) = sessions
            .iter()
            .min_by_key(|(_, record)| record.last_seen_at)
            .map(|(token, _)| token.clone())
        else {
            break;
        };
        sessions.remove(&oldest_token);
    }

    let token = generate_ephemeral_token();
    sessions.insert(
        token.clone(),
        crate::channels::http::UiSessionRecord {
            issued_at: now,
            expires_at: now + crate::channels::http::UI_SESSION_TTL_SECS,
            last_seen_at: now,
            source: source.to_string(),
            client_hint: session_client_hint(headers),
        },
    );
    token
}

pub(super) async fn has_valid_ui_session_cookie(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = extract_ui_session_cookie(headers) else {
        return false;
    };
    let now = unix_now_ts();
    let mut sessions = state.ui_sessions.write().await;
    sessions.retain(|_, record| record.expires_at > now);
    if let Some(record) = sessions.get_mut(&token) {
        record.last_seen_at = now;
        return true;
    }
    false
}

pub(super) async fn revoke_ui_session(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = extract_ui_session_cookie(headers) else {
        return false;
    };
    state.ui_sessions.write().await.remove(&token).is_some()
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

fn parsed_same_origin_referer_path(headers: &HeaderMap) -> Option<String> {
    let referer = headers.get(header::REFERER)?.to_str().ok()?.trim();
    if referer.is_empty() {
        return None;
    }
    let url = reqwest::Url::parse(referer).ok()?;
    let referer_origin = normalize_origin(referer)?;
    let request_origin = extract_request_origin(headers)?;
    if referer_origin != request_origin {
        return None;
    }
    Some(url.path().to_string())
}

fn is_ui_shell_path(path: &str) -> bool {
    path == "/" || path == "/ui" || path == "/ui/" || path.starts_with("/ui/")
}

fn is_app_shell_path(path: &str) -> bool {
    path == "/apps" || path == "/apps/" || path.starts_with("/apps/")
}

fn request_has_ui_referer(headers: &HeaderMap) -> bool {
    parsed_same_origin_referer_path(headers)
        .as_deref()
        .is_some_and(is_ui_shell_path)
}

fn request_has_app_referer(headers: &HeaderMap) -> bool {
    parsed_same_origin_referer_path(headers)
        .as_deref()
        .is_some_and(is_app_shell_path)
}

fn is_browser_navigation_request(headers: &HeaderMap) -> bool {
    let mode_is_navigate = headers
        .get("sec-fetch-mode")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("navigate"));
    let dest_is_document = headers
        .get("sec-fetch-dest")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("document"));
    mode_is_navigate || dest_is_document
}

fn is_trusted_local_ui_navigation_request(headers: &HeaderMap, addr: SocketAddr) -> bool {
    is_trusted_local_ui_request(headers, addr)
        && !request_has_app_referer(headers)
        && (is_browser_navigation_request(headers) || headers.get(header::REFERER).is_none())
}

fn is_trusted_local_ui_api_request(headers: &HeaderMap, addr: SocketAddr) -> bool {
    is_trusted_local_ui_request(headers, addr)
        && request_has_ui_referer(headers)
        && !request_has_app_referer(headers)
}

fn valid_ui_session_request_context(
    headers: &HeaderMap,
    addr: SocketAddr,
    deployment_mode: crate::core::config::DeploymentMode,
) -> bool {
    if deployment_mode == crate::core::config::DeploymentMode::TrustedLocal {
        return is_trusted_local_ui_api_request(headers, addr);
    }

    if request_has_app_referer(headers) {
        return false;
    }

    request_has_ui_referer(headers)
}

pub(super) async fn is_verified_ui_session_request(
    state: &AppState,
    headers: &HeaderMap,
    addr: SocketAddr,
    deployment_mode: crate::core::config::DeploymentMode,
) -> bool {
    if !valid_ui_session_request_context(headers, addr, deployment_mode) {
        return false;
    }

    has_valid_ui_session_cookie(state, headers).await
}

fn is_public_arkorbit_runtime_asset(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() == 7
        && parts[0].is_empty()
        && parts[1] == "api"
        && parts[2] == "arkorbit"
        && parts[3] == "mod"
        && uuid::Uuid::parse_str(parts[4]).is_ok()
        && parts[5] == "runtime"
        && parts[6] == "host.js"
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
    if has_valid_ui_session_cookie(state, headers).await {
        return false;
    }
    if is_trusted_local_ui_navigation_request(headers, addr) {
        return true;
    }

    let expected_key = match sync_http_api_key_state(state, false).await {
        Ok((info, _)) => info.map(|k| k.key),
        Err(_) => None,
    };
    has_valid_bearer_api_key(headers, expected_key.as_deref())
}

async fn apply_new_ui_session_cookie(
    state: &AppState,
    headers: &HeaderMap,
    response: &mut Response,
    secure: bool,
    source: &str,
) {
    let token = create_ui_session(state, headers, source).await;
    apply_session_cookie(response, Some(token.as_str()), secure);
}

pub(super) async fn issue_oauth_state(
    state: &AppState,
    service_id: &str,
    redirect_uri: Option<String>,
) -> String {
    store_oauth_state(
        state,
        PendingOAuthTarget::Integration {
            service_id: service_id.to_string(),
        },
        None,
        redirect_uri,
    )
    .await
}

pub(super) async fn issue_oauth_state_with_pkce(
    state: &AppState,
    service_id: &str,
    redirect_uri: Option<String>,
) -> (String, String) {
    let verifier = generate_ephemeral_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let token = store_oauth_state(
        state,
        PendingOAuthTarget::Integration {
            service_id: service_id.to_string(),
        },
        Some(verifier),
        redirect_uri,
    )
    .await;
    (token, challenge)
}

pub(super) async fn issue_auth_profile_oauth_state_with_pkce(
    state: &AppState,
    profile_id: &str,
    redirect_uri: Option<String>,
) -> (String, String) {
    let verifier = generate_ephemeral_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let token = store_oauth_state(
        state,
        PendingOAuthTarget::AuthProfile {
            profile_id: profile_id.to_string(),
        },
        Some(verifier),
        redirect_uri,
    )
    .await;
    (token, challenge)
}

async fn store_oauth_state(
    state: &AppState,
    target: PendingOAuthTarget,
    pkce_verifier: Option<String>,
    redirect_uri: Option<String>,
) -> String {
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
            target,
            expires_at: now + OAUTH_STATE_TTL_SECS,
            pkce_verifier,
            redirect_uri,
        },
    );
    token
}

pub(super) async fn consume_oauth_state(
    state: &AppState,
    state_token: &str,
) -> Option<PendingOAuthState> {
    let now = unix_now_ts();
    let mut oauth_states = state.oauth_states.write().await;
    oauth_states.retain(|_, entry| entry.expires_at > now);
    oauth_states
        .remove(state_token)
        .and_then(|entry| (entry.expires_at > now).then_some(entry))
}

pub(super) async fn auth_middleware(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    if request.method() == Method::GET && is_public_arkorbit_runtime_asset(&path) {
        return next.run(request).await;
    }

    let expected_key = match sync_http_api_key_state(&state, false).await {
        Ok((info, _rotated)) => info.map(|k| k.key),
        Err(e) => {
            state.security_events.record_auth_failure();
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

    // If API key is missing, fail closed.
    let Some(expected_key) = expected_key else {
        if state.deployment_mode == crate::core::config::DeploymentMode::TrustedLocal
            && is_verified_ui_session_request(
                &state,
                request.headers(),
                addr,
                state.deployment_mode,
            )
            .await
        {
            request
                .extensions_mut()
                .insert(crate::actions::ActionCallerPrincipal::local_admin(
                    "trusted_local_ui_session",
                ));
            return next.run(request).await;
        }

        if !MISSING_API_KEY_WARNED.swap(true, Ordering::Relaxed) {
            tracing::error!("Blocking protected routes because HTTP API key is not configured");
        }
        state.security_events.record_auth_failure();
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
                error: "API authentication is not configured. Regenerate an API key from a trusted session."
                    .to_string(),
            }),
        )
            .into_response();
    };

    if has_valid_bearer_api_key(request.headers(), Some(expected_key.as_str())) {
        request
            .extensions_mut()
            .insert(crate::actions::ActionCallerPrincipal::local_admin(
                "api_key",
            ));
        return next.run(request).await;
    }

    if extract_bearer_api_key(request.headers()).is_some() {
        state.security_events.record_auth_failure();
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

    if is_verified_ui_session_request(&state, request.headers(), addr, state.deployment_mode).await
    {
        request
            .extensions_mut()
            .insert(crate::actions::ActionCallerPrincipal::local_admin(
                "ui_session",
            ));
        return next.run(request).await;
    }

    state.security_events.record_auth_failure();
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
                .into_response();
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
    apply_new_ui_session_cookie(
        &state,
        &headers,
        &mut response,
        state.cookie_secure_default || is_https_forwarded(&headers),
        "api_bootstrap",
    )
    .await;
    response
}

pub(super) async fn issue_local_ui_bootstrap_token(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !state.local_ui_bootstrap_enabled || !is_trusted_local_ui_api_request(&headers, addr) {
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
    if !state.local_ui_bootstrap_enabled || !is_trusted_local_ui_api_request(&headers, addr) {
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
    apply_new_ui_session_cookie(
        &state,
        &headers,
        &mut response,
        state.cookie_secure_default || is_https_forwarded(&headers),
        "local_bootstrap",
    )
    .await;
    response
}

pub(super) fn is_https_forwarded(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

pub(super) fn apply_session_cookie(response: &mut Response, token: Option<&str>, secure: bool) {
    if let Some(token) = token {
        let secure_attr = if secure { "; Secure" } else { "" };
        let cookie = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400{}",
            crate::branding::SESSION_COOKIE_NAME,
            token,
            secure_attr
        );
        if let Ok(val) = cookie.parse() {
            response.headers_mut().insert(header::SET_COOKIE, val);
        }
    }
}

pub(super) fn clear_session_cookie(response: &mut Response, secure: bool) {
    let secure_attr = if secure { "; Secure" } else { "" };
    let cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        crate::branding::SESSION_COOKIE_NAME,
        secure_attr
    );
    if let Ok(val) = cookie.parse() {
        response.headers_mut().insert(header::SET_COOKIE, val);
    }
}

pub(super) async fn logout_ui_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let secure = state.cookie_secure_default || is_https_forwarded(&headers);
    let _ = revoke_ui_session(&state, &headers).await;
    let mut response =
        (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response();
    clear_session_cookie(&mut response, secure);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn loopback_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 45678)
    }

    #[test]
    fn trusted_local_ui_api_request_accepts_exact_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:8990"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("http://localhost:8990/ui/v2"),
        );
        assert!(is_trusted_local_ui_api_request(&headers, loopback_addr()));
    }

    #[test]
    fn trusted_local_ui_api_request_rejects_different_local_port() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:8990"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("http://localhost:3000/ui/v2"),
        );
        assert!(!is_trusted_local_ui_api_request(&headers, loopback_addr()));
    }

    #[test]
    fn arkorbit_runtime_host_is_the_only_public_orbit_module() {
        let orbit_id = uuid::Uuid::new_v4();
        assert!(is_public_arkorbit_runtime_asset(&format!(
            "/api/arkorbit/mod/{}/runtime/host.js",
            orbit_id
        )));
        assert!(!is_public_arkorbit_runtime_asset(&format!(
            "/api/arkorbit/mod/{}/markdown/index.js",
            orbit_id
        )));
        assert!(!is_public_arkorbit_runtime_asset(
            "/api/arkorbit/mod/not-a-uuid/runtime/host.js"
        ));
    }
}
