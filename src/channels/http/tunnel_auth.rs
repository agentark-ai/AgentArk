use super::*;

const TUNNEL_LOGIN_ATTEMPT_WINDOW_SECS: u64 = 60;
const TUNNEL_LOGIN_MAX_ATTEMPTS: u32 = 5;
const TUNNEL_LOGIN_PATH: &str = "/tunnel/login";
const TUNNEL_LOGIN_PAGE_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>__PRODUCT_NAME__ Remote Access</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #06111f 0%, #0a1830 55%, #0d223f 100%);
            color: #e0e6f3;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 24px;
        }
        .login-card {
            width: min(100%, 430px);
            background: rgba(8, 16, 34, 0.92);
            border: 1px solid rgba(96, 165, 250, 0.16);
            border-radius: 18px;
            padding: 36px 34px;
            box-shadow: 0 32px 80px rgba(0, 0, 0, 0.35);
            backdrop-filter: blur(20px);
        }
        .brand {
            display: flex;
            align-items: center;
            gap: 14px;
            margin-bottom: 20px;
        }
        .brand img {
            width: 54px;
            height: 54px;
        }
        .eyebrow {
            font-size: 0.72rem;
            letter-spacing: 0.14em;
            text-transform: uppercase;
            color: #7dd3fc;
            margin-bottom: 4px;
        }
        h1 {
            font-size: 1.35rem;
            color: #f8fbff;
            margin-bottom: 10px;
        }
        p {
            font-size: 0.92rem;
            line-height: 1.5;
            color: #a8b3c9;
            margin-bottom: 22px;
        }
        .note {
            background: rgba(34, 197, 94, 0.08);
            border: 1px solid rgba(34, 197, 94, 0.22);
            color: #c5f8d4;
            border-radius: 12px;
            padding: 12px 14px;
            font-size: 0.82rem;
            line-height: 1.45;
            margin-bottom: 18px;
        }
        input {
            width: 100%;
            padding: 13px 14px;
            background: rgba(255, 255, 255, 0.06);
            border: 1px solid rgba(148, 163, 184, 0.22);
            border-radius: 10px;
            color: #f8fbff;
            font-size: 0.95rem;
            outline: none;
            margin-bottom: 14px;
        }
        input:focus {
            border-color: rgba(56, 189, 248, 0.9);
            box-shadow: 0 0 0 3px rgba(56, 189, 248, 0.14);
        }
        button {
            width: 100%;
            padding: 13px 14px;
            border: none;
            border-radius: 10px;
            background: linear-gradient(135deg, #0ea5e9, #2563eb);
            color: #f8fbff;
            font-size: 0.95rem;
            font-weight: 700;
            cursor: pointer;
        }
        button:hover { opacity: 0.95; }
        button:disabled { opacity: 0.55; cursor: wait; }
        #msg {
            display: none;
            margin-top: 14px;
            font-size: 0.84rem;
            line-height: 1.4;
            border-radius: 10px;
            padding: 11px 12px;
        }
        .error {
            display: block;
            background: rgba(239, 68, 68, 0.12);
            border: 1px solid rgba(248, 113, 113, 0.28);
            color: #fecaca;
        }
        .success {
            display: block;
            background: rgba(34, 197, 94, 0.12);
            border: 1px solid rgba(74, 222, 128, 0.28);
            color: #dcfce7;
        }
        .hint {
            margin-top: 16px;
            font-size: 0.78rem;
            color: #7c8aa5;
        }
    </style>
</head>
<body>
    <div class="login-card">
        <div class="brand">
            <img src="/logo.svg" alt="__PRODUCT_NAME__">
            <div>
                <div class="eyebrow">Secure Remote Access</div>
                <h1>Sign in to __PRODUCT_NAME__</h1>
            </div>
        </div>
        <p>This remote access URL opens your full __PRODUCT_NAME__ console. Enter the custom __PRODUCT_NAME__ password you set in Settings to continue.</p>
        <div class="note">__PRODUCT_NAME__ keeps the internal server API key private. Remote access on this link uses your __PRODUCT_NAME__ password and a secure session cookie.</div>
        <form id="login-form">
            <input
                type="password"
                id="password"
                placeholder="__PRODUCT_NAME__ password"
                autocomplete="current-password"
                autofocus
            >
            <button type="submit" id="login-btn">Sign In</button>
            <div id="msg"></div>
        </form>
        <div class="hint">Disable remote access when you no longer need it.</div>
    </div>
    <script>
        const nextTarget = __NEXT_TARGET__;
        const form = document.getElementById('login-form');
        const passwordInput = document.getElementById('password');
        const button = document.getElementById('login-btn');
        const msg = document.getElementById('msg');

        function showMessage(kind, text) {
            msg.className = kind;
            msg.textContent = text;
            msg.style.display = 'block';
        }

        form.onsubmit = async (event) => {
            event.preventDefault();
            const password = passwordInput.value;
            if (!password) {
                showMessage('error', 'Enter your __PRODUCT_NAME__ password.');
                return;
            }

            button.disabled = true;
            button.textContent = 'Signing in...';
            msg.style.display = 'none';

            try {
                const response = await fetch('/tunnel/login', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ password })
                });
                const data = await response.json().catch(() => ({}));
                if (!response.ok) {
                    showMessage('error', data.error || 'Sign-in failed.');
                    button.disabled = false;
                    button.textContent = 'Sign In';
                    passwordInput.select();
                    return;
                }
                showMessage('success', 'Signed in. Opening __PRODUCT_NAME__...');
                window.location.assign(nextTarget);
            } catch (_) {
                showMessage('error', 'Could not reach __PRODUCT_NAME__.');
                button.disabled = false;
                button.textContent = 'Sign In';
            }
        };
    </script>
</body>
</html>
"##;

#[derive(Debug)]
pub(super) enum ControlPlaneTunnelError {
    CustomPasswordRequired,
    InsecureNoAuthMode,
    HttpsProviderRequired,
    AuthUnavailable(String),
}

impl ControlPlaneTunnelError {
    pub(super) fn message(&self) -> String {
        match self {
            Self::CustomPasswordRequired => {
                format!(
                    "Set a custom {} password before enabling remote access.",
                    crate::branding::PRODUCT_NAME
                )
            }
            Self::InsecureNoAuthMode => {
                "Disable insecure no-auth mode before enabling remote access.".to_string()
            }
            Self::HttpsProviderRequired => format!(
                "Full {} remote access requires an HTTPS-capable provider. Choose Cloudflare, ngrok, Tailscale Funnel, or Tailscale Private.",
                crate::branding::PRODUCT_NAME
            ),
            Self::AuthUnavailable(detail) => {
                format!("Failed to prepare secure remote access: {}", detail)
            }
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::CustomPasswordRequired => "custom_password_required",
            Self::InsecureNoAuthMode => "insecure_no_auth_enabled",
            Self::HttpsProviderRequired => "https_provider_required",
            Self::AuthUnavailable(_) => "auth_unavailable",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            Self::CustomPasswordRequired | Self::InsecureNoAuthMode => StatusCode::FORBIDDEN,
            Self::HttpsProviderRequired => StatusCode::BAD_REQUEST,
            Self::AuthUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    pub(super) fn into_response(self) -> Response {
        let status = self.status_code();
        let code = self.code();
        let message = self.message();
        let requires_custom_password = matches!(&self, Self::CustomPasswordRequired);
        (
            status,
            Json(serde_json::json!({
                "error": message,
                "code": code,
                "public_link_blocked": true,
                "requires_custom_password": requires_custom_password,
            })),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TunnelLoginQuery {
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TunnelLoginRequest {
    password: String,
}

pub(super) fn is_public_tunnel_login_path(path: &str) -> bool {
    path == TUNNEL_LOGIN_PATH || path == "/tunnel/login/"
}

pub(super) fn is_public_tunnel_login_asset_path(path: &str) -> bool {
    matches!(
        path,
        "/logo.svg" | "/logo.png" | "/logo.jpg" | "/favicon.ico"
    )
}

pub(super) async fn control_plane_tunnel_is_active(state: &AppState) -> bool {
    let tunnel = state.tunnel.read().await;
    tunnel.active && tunnel.selected_app_id.is_none() && tunnel.control_plane_enabled
}

pub(super) async fn ensure_control_plane_tunnel_ready(
    state: &AppState,
    provider: TunnelProviderKind,
) -> Result<(), ControlPlaneTunnelError> {
    if matches!(provider, TunnelProviderKind::Bore) {
        return Err(ControlPlaneTunnelError::HttpsProviderRequired);
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);
    if !master_mgr.is_password_set() || bootstrap_active {
        return Err(ControlPlaneTunnelError::CustomPasswordRequired);
    }
    if state.allow_insecure_no_auth {
        return Err(ControlPlaneTunnelError::InsecureNoAuthMode);
    }

    let (info, _rotated) = auth::sync_http_api_key_state(state, true)
        .await
        .map_err(ControlPlaneTunnelError::AuthUnavailable)?;
    if info.is_none() {
        return Err(ControlPlaneTunnelError::AuthUnavailable(
            "API authentication is not configured.".to_string(),
        ));
    }

    Ok(())
}

pub(super) async fn is_control_plane_tunnel_authenticated(
    state: &AppState,
    headers: &HeaderMap,
) -> bool {
    let expected_key = match auth::sync_http_api_key_state(state, false).await {
        Ok((info, _rotated)) => info.map(|k| k.key),
        Err(_) => None,
    };
    if auth::has_valid_bearer_api_key(headers, expected_key.as_deref()) {
        return true;
    }

    auth::has_valid_ui_session_cookie(state, headers).await
}

pub(super) fn redirect_to_tunnel_login(uri: &Uri) -> Response {
    let requested_target = match uri.path() {
        "/" | "/ui" | "/ui/" | "/ui/v2" => "/ui/v2".to_string(),
        _ => sanitize_next_target(uri.path_and_query().map(|value| value.as_str())),
    };
    let location = format!(
        "{}?next={}",
        TUNNEL_LOGIN_PATH,
        urlencoding::encode(&requested_target)
    );
    redirect_to_path(&location)
}

pub(super) async fn tunnel_login_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TunnelLoginQuery>,
) -> Response {
    if !request_matches_active_control_plane_tunnel(&state, &headers).await {
        return StatusCode::NOT_FOUND.into_response();
    }

    let next_target = sanitize_next_target(query.next.as_deref());
    if is_control_plane_tunnel_authenticated(&state, &headers).await {
        return redirect_to_path(&next_target);
    }

    let next_json =
        serde_json::to_string(&next_target).unwrap_or_else(|_| "\"/ui/v2\"".to_string());
    Html(
        crate::branding::render_template(TUNNEL_LOGIN_PAGE_TEMPLATE)
            .replace("__NEXT_TARGET__", &next_json),
    )
    .into_response()
}

pub(super) async fn tunnel_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<TunnelLoginRequest>,
) -> Response {
    if !request_matches_active_control_plane_tunnel(&state, &headers).await {
        return StatusCode::NOT_FOUND.into_response();
    }

    let client_ip = forwarded_client_ip(&headers).unwrap_or_else(|| addr.ip().to_string());
    if let Some(wait_seconds) = login_rate_limit_wait(&state, &client_ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": format!("Too many attempts. Try again in {} seconds.", wait_seconds)
            })),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);
    if !master_mgr.is_password_set() || bootstrap_active {
        return ControlPlaneTunnelError::CustomPasswordRequired.into_response();
    }

    if master_mgr.unlock(request.password.trim()).is_err() {
        register_login_failure(&state, &client_ip).await;
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": format!("Incorrect {} password", crate::branding::PRODUCT_NAME)
            })),
        )
            .into_response();
    }

    clear_login_failures(&state, &client_ip).await;
    let (info, _rotated) = match auth::sync_http_api_key_state(&state, true).await {
        Ok(result) => result,
        Err(detail) => return ControlPlaneTunnelError::AuthUnavailable(detail).into_response(),
    };
    if info.is_none() {
        return ControlPlaneTunnelError::AuthUnavailable(
            "API authentication is not configured.".to_string(),
        )
        .into_response();
    }

    let mut response =
        (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response();
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
                source: "tunnel_login".to_string(),
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
    response
}

async fn request_matches_active_control_plane_tunnel(
    state: &AppState,
    headers: &HeaderMap,
) -> bool {
    let (active, tunnel_url, selected_app_id, control_plane_enabled) = {
        let tunnel = state.tunnel.read().await;
        (
            tunnel.active,
            tunnel.url.clone(),
            tunnel.selected_app_id.clone(),
            tunnel.control_plane_enabled,
        )
    };
    active
        && selected_app_id.is_none()
        && control_plane_enabled
        && request_matches_active_tunnel(headers, tunnel_url.as_deref())
}

fn sanitize_next_target(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or("/ui/v2").trim();
    if candidate.is_empty()
        || !candidate.starts_with('/')
        || candidate.starts_with("//")
        || candidate.starts_with("/tunnel/login")
    {
        "/ui/v2".to_string()
    } else {
        candidate.to_string()
    }
}

fn redirect_to_path(path: &str) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, path)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-real-ip"))
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

async fn login_rate_limit_wait(state: &AppState, client_ip: &str) -> Option<u64> {
    let attempts = state.remote_login_attempts.read().await;
    let (count, since) = attempts.get(client_ip)?;
    if since.elapsed().as_secs() < TUNNEL_LOGIN_ATTEMPT_WINDOW_SECS
        && *count >= TUNNEL_LOGIN_MAX_ATTEMPTS
    {
        Some(TUNNEL_LOGIN_ATTEMPT_WINDOW_SECS.saturating_sub(since.elapsed().as_secs()))
    } else {
        None
    }
}

async fn register_login_failure(state: &AppState, client_ip: &str) {
    let mut attempts = state.remote_login_attempts.write().await;
    attempts.retain(|_, (_, since)| since.elapsed().as_secs() < TUNNEL_LOGIN_ATTEMPT_WINDOW_SECS);
    let entry = attempts
        .entry(client_ip.to_string())
        .or_insert((0, Instant::now()));
    if entry.1.elapsed().as_secs() >= TUNNEL_LOGIN_ATTEMPT_WINDOW_SECS {
        *entry = (1, Instant::now());
    } else {
        entry.0 += 1;
    }
}

async fn clear_login_failures(state: &AppState, client_ip: &str) {
    let mut attempts = state.remote_login_attempts.write().await;
    attempts.remove(client_ip);
}
