use super::*;
use std::time::Duration;

const INTEGRATION_STATUS_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Serialize)]
struct GmailOAuthStartResponse {
    auth_url: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OAuthCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct IntegrationResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub status: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    pub auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_fields: Option<Vec<IntegrationConfigField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_help: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configure_button: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_values: Option<serde_json::Value>,
}

pub(super) async fn gmail_oauth_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    // Try env var first, then fall back to secure config
    let (config_dir, data_dir) = {
        let a = state.agent.read().await;
        (a.config_dir.clone(), a.data_dir.clone())
    };
    let stored_creds = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .ok()
    .and_then(|mgr| mgr.get_custom_secret("gmail_oauth_config").ok().flatten())
    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok());

    let client_id = std::env::var("GMAIL_CLIENT_ID").ok().or_else(|| {
        stored_creds.as_ref().and_then(|v| {
            v.get("client_id")
                .and_then(|c| c.as_str())
                .map(String::from)
        })
    });

    let client_id = match client_id {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Gmail not configured. Add credentials in Settings > Gmail.".to_string(),
                }),
            )
                .into_response();
        }
    };

    let redirect_uri = match oauth_redirect_uri_for_request(&state, &headers, None) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };
    let (state_token, code_challenge) =
        auth::issue_oauth_state_with_pkce(&state, "gmail", Some(redirect_uri.clone())).await;
    let auth_url = format_gmail_auth_url(&client_id, &state_token, &code_challenge, &redirect_uri);

    (StatusCode::OK, Json(GmailOAuthStartResponse { auth_url })).into_response()
}

/// Exchange Gmail authorization code for tokens (called from oauth_callback)
async fn gmail_exchange_code(
    state: &AppState,
    redirect_uri: &str,
    code: &str,
    pkce_verifier: Option<&str>,
) -> Result<(), String> {
    let (config_dir, data_dir) = {
        let a = state.agent.read().await;
        (a.config_dir.clone(), a.data_dir.clone())
    };
    let stored_creds = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .ok()
    .and_then(|mgr| mgr.get_custom_secret("gmail_oauth_config").ok().flatten())
    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok());

    let client_id = std::env::var("GMAIL_CLIENT_ID")
        .ok()
        .or_else(|| {
            stored_creds.as_ref().and_then(|v| {
                v.get("client_id")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .ok_or_else(|| "Gmail client_id not configured".to_string())?;
    let client_secret = std::env::var("GMAIL_CLIENT_SECRET")
        .ok()
        .or_else(|| {
            stored_creds.as_ref().and_then(|v| {
                v.get("client_secret")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .ok_or_else(|| "Gmail client_secret not configured".to_string())?;

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let mut params = vec![
        ("client_id", client_id.as_str().to_string()),
        ("client_secret", client_secret.as_str().to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("grant_type", "authorization_code".to_string()),
    ];
    if let Some(verifier) = pkce_verifier {
        params.push(("code_verifier", verifier.to_string()));
    }

    let resp = http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Token exchange failed ({})", status));
    }

    #[derive(Deserialize)]
    struct TokenResp {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: i64,
    }

    let token: TokenResp = resp
        .json()
        .await
        .map_err(|e| format!("Invalid token response: {}", e))?;

    let now = chrono::Utc::now().timestamp();
    let tokens = serde_json::json!({
        "access_token": token.access_token,
        "refresh_token": token.refresh_token.unwrap_or_default(),
        "expires_at": now + token.expires_in
    });

    // Store tokens encrypted via SecureConfigManager
    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .map_err(|e| format!("Secure storage error: {}", e))?;
    let payload = serde_json::to_string(&tokens).unwrap_or_default();
    manager
        .set_custom_secret("gmail_tokens", Some(payload))
        .map_err(|e| format!("Failed to save tokens: {}", e))?;
    set_builtin_integration_enabled(&config_dir, &data_dir, &["gmail"], true)?;
    set_builtin_integration_user_disabled(&config_dir, &data_dir, &["gmail"], false)?;

    Ok(())
}

pub(super) async fn gmail_status(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(_) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"connected": false})),
            )
                .into_response();
        }
    };
    let payload: Option<String> = manager
        .get_custom_secret("gmail_tokens")
        .unwrap_or_default();
    let payload = match payload {
        Some(v) => v,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"connected": false})),
            )
                .into_response();
        }
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).unwrap_or_else(|_| serde_json::json!({}));
    // Check for refresh_token presence - access tokens expire after ~1 hour
    // but as long as we have a refresh_token, we can auto-renew
    let has_refresh = parsed
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let expires_at = parsed
        .get("expires_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let validation_error = if has_refresh {
        validate_gmail_oauth_connection(&config_dir).await.err()
    } else {
        None
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "connected": has_refresh && validation_error.is_none(),
            "expires_at": expires_at,
            "error": validation_error
        })),
    )
        .into_response()
}

pub(super) async fn gmail_test(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Gmail not connected yet".to_string(),
                }),
            )
                .into_response();
        }
    };
    if manager
        .get_custom_secret("gmail_tokens")
        .ok()
        .flatten()
        .is_none()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Gmail not connected yet".to_string(),
            }),
        )
            .into_response();
    }

    let access_token = match crate::actions::gmail::ensure_access_token(&config_dir).await {
        Ok(token) => token,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Failed to refresh Gmail token: {}", e),
                }),
            )
                .into_response();
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to build client: {}", e),
                }),
            )
                .into_response();
        }
    };

    let resp = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
        .bearer_auth(access_token)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Gmail test failed: {}", e),
                }),
            )
                .into_response();
        }
    };

    if !resp.status().is_success() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Gmail test failed: {}", resp.status()),
            }),
        )
            .into_response();
    }

    #[derive(Deserialize)]
    struct GmailProfileResp {
        #[serde(default)]
        email_address: String,
    }

    let profile = resp
        .json::<GmailProfileResp>()
        .await
        .unwrap_or(GmailProfileResp {
            email_address: "".to_string(),
        });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "email": profile.email_address
        })),
    )
        .into_response()
}

pub(super) fn integration_enabled_key(id: &str) -> String {
    crate::integrations::integration_enabled_key(id)
}

pub(super) fn integration_user_disabled_key(id: &str) -> String {
    crate::integrations::integration_user_disabled_key(id)
}

pub(super) async fn refresh_connected_action_surfaces(state: &AppState, reason: &'static str) {
    let agent = { state.agent.read().await.clone() };
    agent.refresh_action_catalog_index(reason).await;
}

fn set_builtin_integration_enabled(
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    integration_ids: &[&str],
    enabled: bool,
) -> Result<(), String> {
    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        config_dir,
        Some(data_dir),
    )
    .map_err(|e| format!("Secure storage error: {}", e))?;
    let value = enabled.to_string();
    for integration_id in integration_ids {
        manager
            .set_custom_secret(
                &integration_enabled_key(integration_id),
                Some(value.clone()),
            )
            .map_err(|e| {
                format!(
                    "Failed to update integration state for {}: {}",
                    integration_id, e
                )
            })?;
    }
    Ok(())
}

fn set_builtin_integration_user_disabled(
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    integration_ids: &[&str],
    disabled: bool,
) -> Result<(), String> {
    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        config_dir,
        Some(data_dir),
    )
    .map_err(|e| format!("Secure storage error: {}", e))?;
    let value = disabled.to_string();
    for integration_id in integration_ids {
        manager
            .set_custom_secret(
                &integration_user_disabled_key(integration_id),
                Some(value.clone()),
            )
            .map_err(|e| {
                format!(
                    "Failed to update manual integration state for {}: {}",
                    integration_id, e
                )
            })?;
    }
    Ok(())
}

fn builtin_runtime_integration_ids_for_service(
    config_dir: &std::path::Path,
    service_id: &str,
) -> Vec<&'static str> {
    match service_id {
        "gmail" => vec!["gmail"],
        "google_calendar" | "calendar" => vec!["google_calendar"],
        "google_workspace" => {
            let mut ids = vec!["google_workspace"];
            let granted =
                crate::actions::google_workspace::granted_bundles(config_dir).unwrap_or_default();
            if granted.iter().any(|bundle| bundle == "gmail") {
                ids.push("gmail");
            }
            if granted.iter().any(|bundle| bundle == "calendar") {
                ids.push("google_calendar");
            }
            ids
        }
        _ => Vec::new(),
    }
}

pub(super) fn parse_boolish(value: &str) -> Option<bool> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() {
        return None;
    }
    match v.as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

pub(super) fn stored_secret(
    manager: Option<&crate::core::runtime::config::SecureConfigManager>,
    key: &str,
) -> Option<String> {
    manager.and_then(|mgr| mgr.get_custom_secret(key).ok().flatten())
}

fn oauth_pair_from_json_str(value: Option<String>) -> Option<(String, String)> {
    let payload = value?;
    let parsed = serde_json::from_str::<serde_json::Value>(&payload).ok()?;
    let client_id = parsed
        .get("client_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let client_secret = parsed
        .get("client_secret")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    Some((client_id, client_secret))
}

pub(super) fn oauth_has_refresh_token(value: Option<String>) -> bool {
    let Some(payload) = value else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|parsed| {
            parsed
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .map(str::to_string)
        })
        .is_some_and(|token| !token.is_empty())
}

pub(super) async fn validate_gmail_oauth_connection(
    config_dir: &std::path::Path,
) -> std::result::Result<(), String> {
    let access_token = crate::actions::gmail::ensure_access_token(config_dir)
        .await
        .map_err(|e| format!("Failed to refresh Gmail token: {}", e))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    let response = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Gmail validation failed: {}", e))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Gmail API error: {}", response.status()))
    }
}

pub(super) async fn validate_calendar_oauth_connection(
    config_dir: &std::path::Path,
) -> std::result::Result<(), String> {
    let access_token = crate::actions::calendar::ensure_access_token(config_dir)
        .await
        .map_err(|e| format!("Failed to refresh Calendar token: {}", e))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    let response = client
        .get("https://www.googleapis.com/calendar/v3/calendars/primary")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Calendar validation failed: {}", e))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Calendar API error: {}", response.status()))
    }
}

pub(super) fn gmail_oauth_pair(
    manager: Option<&crate::core::runtime::config::SecureConfigManager>,
) -> Option<(String, String)> {
    if let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("GMAIL_CLIENT_ID"),
        std::env::var("GMAIL_CLIENT_SECRET"),
    ) {
        if !client_id.trim().is_empty() && !client_secret.trim().is_empty() {
            return Some((client_id, client_secret));
        }
    }
    oauth_pair_from_json_str(stored_secret(manager, "gmail_oauth_config"))
}

pub(super) fn calendar_oauth_pair(
    manager: Option<&crate::core::runtime::config::SecureConfigManager>,
) -> Option<(String, String)> {
    if let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("CALENDAR_CLIENT_ID"),
        std::env::var("CALENDAR_CLIENT_SECRET"),
    ) {
        if !client_id.trim().is_empty() && !client_secret.trim().is_empty() {
            return Some((client_id, client_secret));
        }
    }
    oauth_pair_from_json_str(stored_secret(manager, "calendar_oauth_config"))
        .or_else(|| gmail_oauth_pair(manager))
}

fn google_workspace_bundle_text(config_dir: &std::path::Path) -> String {
    crate::actions::google_workspace::load_saved_bundles(config_dir)
        .unwrap_or_else(|_| crate::actions::google_workspace::default_bundles())
        .join(", ")
}

fn google_workspace_config_values(config_dir: &std::path::Path) -> serde_json::Value {
    let saved_client =
        crate::actions::google_workspace::load_saved_workspace_client_config(config_dir)
            .ok()
            .flatten();
    let oauth_client_configured =
        crate::actions::google_workspace::load_workspace_client_config(config_dir)
            .ok()
            .flatten()
            .is_some();
    let oauth_client_source =
        crate::actions::google_workspace::workspace_client_config_source(config_dir)
            .ok()
            .flatten()
            .unwrap_or("none");
    serde_json::json!({
        "client_id": saved_client
            .as_ref()
            .map(|config| config.client_id.clone())
            .unwrap_or_default(),
        "client_secret_configured": saved_client.is_some(),
        "service_bundles": google_workspace_bundle_text(config_dir),
        "oauth_client_configured": oauth_client_configured,
        "oauth_client_source": oauth_client_source
    })
}

fn google_workspace_status_detail(
    granted: &[String],
    missing: &[String],
    pending: &[String],
) -> Option<String> {
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(|bundle| crate::actions::google_workspace::bundle_label(bundle))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!(
            "Reconnect Google Workspace to grant access for {}.",
            list
        ));
    }
    if !pending.is_empty() {
        let list = pending
            .iter()
            .map(|bundle| crate::actions::google_workspace::bundle_label(bundle))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!(
            "{} requested additional Google Workspace access for {}. Reconnect to approve it.",
            crate::branding::PRODUCT_NAME,
            list
        ));
    }
    if !granted.is_empty() {
        let list = granted
            .iter()
            .map(|bundle| crate::actions::google_workspace::bundle_label(bundle))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!("Connected bundles: {}.", list));
    }
    None
}

fn google_workspace_test_payload_ok(payload: &serde_json::Value) -> bool {
    payload
        .get("status")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("ok"))
}

fn google_workspace_test_issue_detail(payload: &serde_json::Value) -> Option<String> {
    let checks = payload.get("checks")?.as_object()?;
    let issues = checks
        .iter()
        .filter_map(|(key, value)| {
            let text = value.as_str()?.trim();
            let lowered = text.to_ascii_lowercase();
            if lowered.contains("failed")
                || lowered.contains("unavailable")
                || lowered.contains("needs additional access")
                || lowered.contains("reconnect")
            {
                Some(format!("{}: {}", key.replace('_', " "), text))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if issues.is_empty() {
        None
    } else {
        Some(format!("Health checks: {}", issues.join(" | ")))
    }
}

fn google_workspace_configured(config_dir: &std::path::Path) -> bool {
    crate::actions::google_workspace::load_workspace_client_config(config_dir)
        .ok()
        .flatten()
        .is_some()
}

fn integration_uses_config_only_status(id: &str) -> bool {
    matches!(
        id,
        "twitter"
            | "google_places"
            | "twilio"
            | "ordering"
            | "garmin"
            | "whoop"
            | "ga4"
            | "gsc"
            | "social_analytics"
            | "moltbook"
            | "vercel"
    )
}

fn config_only_status_detail(id: &str, enabled: bool) -> String {
    let name = id.replace('_', " ");
    if enabled {
        format!(
            "{} credentials are saved and agent use is enabled, but {} has not run a live health probe for this connector yet.",
            name,
            crate::branding::PRODUCT_NAME
        )
    } else {
        format!(
            "{} credentials are saved, but this connector is still waiting for explicit enablement and live use before {} can confirm connectivity.",
            name,
            crate::branding::PRODUCT_NAME
        )
    }
}

fn format_gmail_auth_url(
    client_id: &str,
    state_token: &str,
    code_challenge: &str,
    redirect_uri: &str,
) -> String {
    let scope =
        "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send";
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(scope),
        urlencoding::encode(state_token),
        urlencoding::encode(code_challenge),
    )
}

fn format_calendar_auth_url(
    client_id: &str,
    state_token: &str,
    code_challenge: &str,
    redirect_uri: &str,
) -> String {
    let scope = "https://www.googleapis.com/auth/calendar";
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(scope),
        urlencoding::encode(state_token),
        urlencoding::encode(code_challenge),
    )
}

async fn build_gmail_auth_url(
    state: &AppState,
    headers: &HeaderMap,
    manager: Option<&crate::core::runtime::config::SecureConfigManager>,
) -> std::result::Result<String, String> {
    let (client_id, _) = gmail_oauth_pair(manager)
        .ok_or_else(|| "Gmail not configured. Add Google OAuth credentials first.".to_string())?;
    let redirect_uri = oauth_redirect_uri_for_request(state, headers, None)?;
    let (state_token, code_challenge) =
        auth::issue_oauth_state_with_pkce(state, "gmail", Some(redirect_uri.clone())).await;
    Ok(format_gmail_auth_url(
        &client_id,
        &state_token,
        &code_challenge,
        &redirect_uri,
    ))
}

async fn build_calendar_auth_url(
    state: &AppState,
    headers: &HeaderMap,
    manager: Option<&crate::core::runtime::config::SecureConfigManager>,
) -> std::result::Result<String, String> {
    let (client_id, _) = calendar_oauth_pair(manager).ok_or_else(|| {
        "Google Calendar not configured. Add Google OAuth credentials first.".to_string()
    })?;
    let redirect_uri = oauth_redirect_uri_for_request(state, headers, None)?;
    let (state_token, code_challenge) =
        auth::issue_oauth_state_with_pkce(state, "google_calendar", Some(redirect_uri.clone()))
            .await;
    Ok(format_calendar_auth_url(
        &client_id,
        &state_token,
        &code_challenge,
        &redirect_uri,
    ))
}

async fn build_google_workspace_auth_url(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<String, String> {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    let redirect_uri = oauth_redirect_uri_for_request(state, headers, None)?;
    let (state_token, code_challenge) =
        auth::issue_oauth_state_with_pkce(state, "google_workspace", Some(redirect_uri.clone()))
            .await;
    crate::actions::google_workspace::build_auth_url(
        &config_dir,
        &state_token,
        &code_challenge,
        &redirect_uri,
    )
    .map_err(|e| e.to_string())
}

pub(super) fn external_integration_config(
    id: &str,
) -> Option<(Vec<IntegrationConfigField>, Option<String>, Option<String>)> {
    // This section is "External Integrations" in the Settings UI.
    // Keep these auth fields aligned with how the integration actually loads secrets.
    match id {
        "google_workspace" => Some((
            vec![
                IntegrationConfigField {
                    key: "client_id".to_string(),
                    label: "Google OAuth Client ID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("1234567890-xxxxx.apps.googleusercontent.com".to_string()),
                    required: false,
                    options: None,
                },
                IntegrationConfigField {
                    key: "client_secret".to_string(),
                    label: "Google OAuth Client Secret".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("GOCSPX-...".to_string()),
                    required: false,
                    options: None,
                },
                IntegrationConfigField {
                    key: "service_bundles".to_string(),
                    label: "Workspace Bundles".to_string(),
                    input_type: "textarea".to_string(),
                    placeholder: Some("gmail, calendar".to_string()),
                    required: false,
                    options: None,
                },
            ],
            Some(format!(
                "Enter the Google OAuth client ID and client secret for this {} instance, choose the Workspace bundles you want, then continue with Google in your browser.",
                crate::branding::PRODUCT_NAME
            )),
            Some("Save Setup".to_string()),
        )),
        "gmail" => Some((
            vec![
                IntegrationConfigField {
                    key: "client_id".to_string(),
                    label: "Google OAuth Client ID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("1234567890-xxxxx.apps.googleusercontent.com".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "client_secret".to_string(),
                    label: "Google OAuth Client Secret".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("GOCSPX-...".to_string()),
                    required: true,
                    options: None,
                },
            ],
            Some("Use a Google OAuth desktop/web client with Gmail API access enabled. Save the client credentials, then click Connect to complete sign-in.".to_string()),
            Some("Save Credentials".to_string()),
        )),
        "google_calendar" => Some((
            vec![
                IntegrationConfigField {
                    key: "client_id".to_string(),
                    label: "Google OAuth Client ID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("1234567890-xxxxx.apps.googleusercontent.com".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "client_secret".to_string(),
                    label: "Google OAuth Client Secret".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("GOCSPX-...".to_string()),
                    required: true,
                    options: None,
                },
            ],
            Some("Use a Google OAuth client with Calendar API access enabled. Save the client credentials, then click Connect to finish sign-in.".to_string()),
            Some("Save Credentials".to_string()),
        )),
        "github" => Some((
            vec![IntegrationConfigField {
                key: "token".to_string(),
                label: "Personal Access Token".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("ghp_...".to_string()),
                required: true,
                options: None,
            }],
            Some("Create a GitHub personal access token and paste it here. It will be stored encrypted. This is for the GitHub API connector; local git operations in the workspace work separately and do not require this token.".to_string()),
            Some("Save Token".to_string()),
        )),
        "vercel" => Some((
            vec![
                IntegrationConfigField {
                    key: "token".to_string(),
                    label: "Vercel Access Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("vercel token".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "team_id".to_string(),
                    label: "Team ID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("team_...".to_string()),
                    required: false,
                    options: None,
                },
                IntegrationConfigField {
                    key: "project_id".to_string(),
                    label: "Default Project ID or Name".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("my-vercel-project".to_string()),
                    required: false,
                    options: None,
                },
            ],
            Some("Paste a Vercel access token for app publishing. Team and project are optional defaults; individual deploys can override them.".to_string()),
            Some("Save Vercel".to_string()),
        )),
        "notion" => Some((
            vec![IntegrationConfigField {
                key: "token".to_string(),
                label: "Integration Token".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("secret_...".to_string()),
                required: true,
                options: None,
            }],
            Some("Paste your Notion integration token. It will be stored encrypted.".to_string()),
            Some("Save Token".to_string()),
        )),
        "twitter" => Some((
            vec![IntegrationConfigField {
                key: "bearer_token".to_string(),
                label: "Bearer Token".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("AAAAAAAA...".to_string()),
                required: true,
                options: None,
            }],
            Some("Paste your X (Twitter) API bearer token. It will be stored encrypted.".to_string()),
            Some("Save Token".to_string()),
        )),
        "onepassword" => Some((
            vec![
                IntegrationConfigField {
                    key: "host".to_string(),
                    label: "Connect Host".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("http://localhost:8080".to_string()),
                    required: false,
                    options: None,
                },
                IntegrationConfigField {
                    key: "token".to_string(),
                    label: "Connect Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("op_connect_...".to_string()),
                    required: true,
                    options: None,
                },
            ],
            Some("Configure 1Password Connect host and token. Token is stored encrypted.".to_string()),
            Some("Save".to_string()),
        )),
        "google_places" => Some((
            vec![IntegrationConfigField {
                key: "api_key".to_string(),
                label: "API Key".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("AIza...".to_string()),
                required: true,
                options: None,
            }],
            Some("Google Places API key. Stored encrypted.".to_string()),
            Some("Save Key".to_string()),
        )),
        "twilio" => Some((
            vec![
                IntegrationConfigField {
                    key: "account_sid".to_string(),
                    label: "Account SID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("AC...".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "auth_token".to_string(),
                    label: "Auth Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("...".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "from_number".to_string(),
                    label: "From Number".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("+15551234567".to_string()),
                    required: true,
                    options: None,
                },
            ],
            Some("Twilio credentials (Account SID, Auth Token, and a verified From number). Stored encrypted.".to_string()),
            Some("Save".to_string()),
        )),
        "ordering" => Some((
            vec![IntegrationConfigField {
                key: "config_json".to_string(),
                label: "Ordering Config (JSON)".to_string(),
                input_type: "textarea".to_string(),
                placeholder: Some("{\"provider\":\"shopify\",\"access_token\":\"...\",\"store_url\":\"https://YOUR.myshopify.com\"}".to_string()),
                required: true,
                options: None,
            }],
            Some("JSON config stored encrypted as `ordering_config`. Supports Shopify or Webhook providers.".to_string()),
            Some("Save".to_string()),
        )),
        "garmin" => Some((
            vec![
                IntegrationConfigField {
                    key: "token".to_string(),
                    label: "Garmin Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("garmin_...".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "api_base".to_string(),
                    label: "API Base URL".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("https://apis.garmin.com/wellness-api/rest".to_string()),
                    required: false,
                    options: None,
                },
            ],
            Some("Garmin token and optional API base URL. Stored encrypted.".to_string()),
            Some("Save".to_string()),
        )),
        "whoop" => Some((
            vec![IntegrationConfigField {
                key: "token".to_string(),
                label: "WHOOP Access Token".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("whoop_...".to_string()),
                required: true,
                options: None,
            }],
            Some("WHOOP API access token. Stored encrypted.".to_string()),
            Some("Save Token".to_string()),
        )),
        "ga4" => Some((
            vec![
                IntegrationConfigField {
                    key: "access_token".to_string(),
                    label: "GA4 Access Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("ya29....".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "property_id".to_string(),
                    label: "GA4 Property ID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("123456789".to_string()),
                    required: false,
                    options: None,
                },
            ],
            Some("Google Analytics 4 credentials. Token and property id are stored encrypted.".to_string()),
            Some("Save".to_string()),
        )),
        "gsc" => Some((
            vec![
                IntegrationConfigField {
                    key: "access_token".to_string(),
                    label: "GSC Access Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("ya29....".to_string()),
                    required: true,
                    options: None,
                },
                IntegrationConfigField {
                    key: "site_url".to_string(),
                    label: "Site URL".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("sc-domain:example.com or https://example.com/".to_string()),
                    required: false,
                    options: None,
                },
            ],
            Some("Google Search Console credentials. Stored encrypted.".to_string()),
            Some("Save".to_string()),
        )),
        "social_analytics" => Some((
            vec![
                IntegrationConfigField {
                    key: "social_twitter_bearer_token".to_string(),
                    label: "Twitter Bearer Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("AAAAAAAA...".to_string()),
                    required: false,
                    options: None,
                },
                IntegrationConfigField {
                    key: "social_ga4_access_token".to_string(),
                    label: "GA4 Access Token".to_string(),
                    input_type: "password".to_string(),
                    placeholder: Some("ya29....".to_string()),
                    required: false,
                    options: None,
                },
                IntegrationConfigField {
                    key: "social_ga4_property_id".to_string(),
                    label: "GA4 Property ID".to_string(),
                    input_type: "text".to_string(),
                    placeholder: Some("123456789".to_string()),
                    required: false,
                    options: None,
                },
            ],
            Some("Social analytics uses Twitter and/or GA4 credentials to build cross-source summaries. Stored encrypted.".to_string()),
            Some("Save".to_string()),
        )),
        "moltbook" => Some((
            vec![IntegrationConfigField {
                key: "api_key".to_string(),
                label: "Moltbook API Key".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("moltbook_xxx".to_string()),
                required: true,
                options: None,
            }],
            Some("Moltbook API key. Stored encrypted. Only sent to https://www.moltbook.com/api/v1/*".to_string()),
            Some("Save Key".to_string()),
        )),
        _ => None,
    }
}

fn normalize_oauth_callback_integration_id(service_id: &str) -> &str {
    if service_id == "calendar" {
        "google_calendar"
    } else {
        service_id
    }
}

fn oauth_callback_signal_script(service_id: &str, status: &str, detail: &str) -> String {
    let payload = serde_json::json!({
        "type": "oauth_callback",
        "service_id": service_id,
        "integration_id": normalize_oauth_callback_integration_id(service_id),
        "status": status,
        "detail": detail,
    });
    let payload_json = serde_json::to_string(&payload)
        .unwrap_or_else(|_| "{\"type\":\"oauth_callback\"}".to_string())
        .replace("</", "<\\/");
    format!(
        r#"<script>
(function () {{
  const payload = {payload_json};
  try {{
    window.localStorage.setItem("agentark:oauth-callback", JSON.stringify(payload));
  }} catch (_) {{}}
  try {{
    if ("BroadcastChannel" in window) {{
      const channel = new BroadcastChannel("agentark-oauth");
      channel.postMessage(payload);
      channel.close();
    }}
  }} catch (_) {{}}
  try {{
    window.opener?.postMessage(payload, window.location.origin);
  }} catch (_) {{}}
  setTimeout(() => window.close(), 1200);
}})();
</script>"#
    )
}

fn oauth_profile_callback_signal_script(profile_id: &str, status: &str, detail: &str) -> String {
    let payload = serde_json::json!({
        "type": "oauth_callback",
        "service_id": "auth_profile",
        "auth_profile_id": profile_id,
        "status": status,
        "detail": detail,
    });
    let payload_json = serde_json::to_string(&payload)
        .unwrap_or_else(|_| "{\"type\":\"oauth_callback\"}".to_string())
        .replace("</", "<\\/");
    format!(
        r#"<script>
(function () {{
  const payload = {payload_json};
  try {{
    window.localStorage.setItem("agentark:oauth-callback", JSON.stringify(payload));
  }} catch (_) {{}}
  try {{
    if ("BroadcastChannel" in window) {{
      const channel = new BroadcastChannel("agentark-oauth");
      channel.postMessage(payload);
      channel.close();
    }}
  }} catch (_) {{}}
  try {{
    window.opener?.postMessage(payload, window.location.origin);
  }} catch (_) {{}}
  setTimeout(() => window.close(), 1200);
}})();
</script>"#
    )
}

/// Handle OAuth callback from providers (Google, etc.)
pub(super) async fn oauth_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<OAuthCallbackParams>,
) -> Response {
    let Some(state_token) = params.state.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Html(
                r#"<!DOCTYPE html>
<html>
<head><title>OAuth Failed</title>
<style>
body { font-family: system-ui; background: #1a1a2e; color: #eee; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }
.card { background: #16213e; padding: 2rem; border-radius: 12px; text-align: center; max-width: 400px; }
.error { color: #ff6b6b; }
</style>
</head>
<body>
<div class="card">
<h2 class="error">Authorization Failed</h2>
<p>This authorization request is invalid or has expired. Start the sign-in flow again from __PRODUCT_NAME__.</p>
<p><a href="/" style="color: #00d9ff;">Return to __PRODUCT_NAME__</a></p>
</div>
</body>
</html>"#
                    .replace("__PRODUCT_NAME__", crate::branding::PRODUCT_NAME),
            ),
        )
            .into_response();
    };

    let Some(oauth_state) = auth::consume_oauth_state(&state, state_token).await else {
        return (
            StatusCode::BAD_REQUEST,
            Html(
                r#"<!DOCTYPE html>
<html>
<head><title>OAuth Failed</title>
<style>
body { font-family: system-ui; background: #1a1a2e; color: #eee; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }
.card { background: #16213e; padding: 2rem; border-radius: 12px; text-align: center; max-width: 400px; }
.error { color: #ff6b6b; }
</style>
</head>
<body>
<div class="card">
<h2 class="error">Authorization Failed</h2>
<p>This authorization request is invalid or has expired. Start the sign-in flow again from __PRODUCT_NAME__.</p>
<p><a href="/" style="color: #00d9ff;">Return to __PRODUCT_NAME__</a></p>
</div>
</body>
</html>"#
                    .replace("__PRODUCT_NAME__", crate::branding::PRODUCT_NAME),
            ),
        )
            .into_response();
    };

    let redirect_uri_result = oauth_state
        .redirect_uri
        .clone()
        .map(Ok)
        .unwrap_or_else(|| oauth_redirect_uri_for_request(&state, &headers, None));
    let pkce_verifier = oauth_state.pkce_verifier;
    let oauth_target = oauth_state.target;

    if let Some(error) = params.error {
        let signal = match &oauth_target {
            PendingOAuthTarget::Integration { service_id } => {
                oauth_callback_signal_script(service_id, "error", &error)
            }
            PendingOAuthTarget::AuthProfile { profile_id } => {
                oauth_profile_callback_signal_script(profile_id, "error", &error)
            }
        };
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head><title>OAuth Failed</title>
<style>
body {{ font-family: system-ui; background: #1a1a2e; color: #eee; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }}
.card {{ background: #16213e; padding: 2rem; border-radius: 12px; text-align: center; max-width: 400px; }}
.error {{ color: #ff6b6b; }}
</style>
</head>
<body>
<div class="card">
<h2 class="error"> Authorization Failed</h2>
<p>{}</p>
<p><a href="/" style="color: #00d9ff;">Return to __PRODUCT_NAME__</a></p>
</div>
{}
</body>
</html>"#,
            escape_html(&error),
            signal
        )
        .replace("__PRODUCT_NAME__", crate::branding::PRODUCT_NAME);
        return (StatusCode::OK, Html(html)).into_response();
    }

    let code = match params.code {
        Some(c) => c,
        None => {
            return (StatusCode::BAD_REQUEST, Html("Missing authorization code")).into_response();
        }
    };

    let (callback_label, result) = match &oauth_target {
        PendingOAuthTarget::Integration { service_id } => {
            let result = match service_id.as_str() {
                "gmail" => match redirect_uri_result.as_ref() {
                    Ok(redirect_uri) => gmail_exchange_code(
                        &state,
                        redirect_uri.as_str(),
                        &code,
                        pkce_verifier.as_deref(),
                    )
                    .await
                    .map(|_| serde_json::json!({"status": "connected"}))
                    .map_err(|e| anyhow::anyhow!(e)),
                    Err(error) => Err(anyhow::anyhow!(error.clone())),
                },
                "google_workspace" => {
                    let config_dir = { state.agent.read().await.config_dir.clone() };
                    match redirect_uri_result.as_ref() {
                        Ok(redirect_uri) => crate::actions::google_workspace::exchange_code(
                            &config_dir,
                            redirect_uri.as_str(),
                            &code,
                            pkce_verifier.as_deref(),
                        )
                        .await
                        .map(|tokens| {
                            serde_json::json!({
                                "status": "connected",
                                "granted_bundles": tokens.granted_bundles
                            })
                        })
                        .map_err(|e| anyhow::anyhow!(e)),
                        Err(error) => Err(anyhow::anyhow!(error.clone())),
                    }
                }
                "google_calendar" | "calendar" => match redirect_uri_result.as_ref() {
                    Ok(redirect_uri) => calendar_exchange_code(
                        &state,
                        redirect_uri.as_str(),
                        &code,
                        pkce_verifier.as_deref(),
                    )
                    .await
                    .map(|_| serde_json::json!({"status": "connected"}))
                    .map_err(|e| anyhow::anyhow!(e)),
                    Err(error) => Err(anyhow::anyhow!(error.clone())),
                },
                _ => {
                    let agent = state.agent.read().await;
                    if let Some(integration) = agent.integrations.get(service_id) {
                        integration
                            .execute("auth_callback", &serde_json::json!({"code": code}))
                            .await
                    } else {
                        Err(anyhow::anyhow!("Unknown service: {}", service_id))
                    }
                }
            };
            (service_id.clone(), result)
        }
        PendingOAuthTarget::AuthProfile { profile_id } => {
            let storage = { state.agent.read().await.storage.clone() };
            let result = match redirect_uri_result.as_ref() {
                Ok(redirect_uri) => {
                    crate::core::connectivity::auth_profiles::AuthProfileControlPlane::complete_oauth_callback(
                        &storage,
                        profile_id,
                        &code,
                        pkce_verifier.as_deref(),
                        Some(redirect_uri.as_str()),
                    )
                    .await
                    .map(|profile| {
                        serde_json::json!({
                            "status": "connected",
                            "profile_id": profile.id,
                            "kind": profile.kind,
                        })
                    })
                }
                Err(error) => Err(anyhow::anyhow!(error.clone())),
            };
            (profile_id.clone(), result)
        }
    };

    match result {
        Ok(_) => {
            let mut callback_warnings = Vec::new();
            if matches!(
                &oauth_target,
                PendingOAuthTarget::Integration { service_id }
                    if matches!(
                        service_id.as_str(),
                        "gmail" | "google_calendar" | "calendar" | "google_workspace"
                    )
            ) {
                let (config_dir, data_dir) = {
                    let agent = state.agent.read().await;
                    (agent.config_dir.clone(), agent.data_dir.clone())
                };
                let service_id = match &oauth_target {
                    PendingOAuthTarget::Integration { service_id } => service_id.as_str(),
                    PendingOAuthTarget::AuthProfile { .. } => "",
                };
                let integration_ids =
                    builtin_runtime_integration_ids_for_service(&config_dir, service_id);
                if let Err(error) =
                    set_builtin_integration_enabled(&config_dir, &data_dir, &integration_ids, true)
                {
                    tracing::warn!(
                        "oauth_callback: connected '{}' but failed to enable runtime actions: {}",
                        service_id,
                        error
                    );
                    callback_warnings.push(format!(
                        "Connected, but enabling runtime actions failed: {}",
                        error
                    ));
                }
                if let Err(error) = set_builtin_integration_user_disabled(
                    &config_dir,
                    &data_dir,
                    &integration_ids,
                    false,
                ) {
                    tracing::warn!(
                        "oauth_callback: connected '{}' but failed to clear manual disable markers: {}",
                        service_id,
                        error
                    );
                    callback_warnings.push(format!(
                        "Connected, but clearing manual disable markers failed: {}",
                        error
                    ));
                }
            }
            let (extension_packs, runtime, agent_for_catalog) = {
                let agent = state.agent.read().await;
                (
                    agent.extension_packs.clone(),
                    agent.runtime.clone(),
                    agent.clone(),
                )
            };
            if let Err(error) = extension_packs.read().await.sync_to_runtime(&runtime).await {
                tracing::warn!(
                    "oauth_callback: connected '{}' but failed to sync extension-pack actions: {}",
                    callback_label,
                    error
                );
                callback_warnings.push(format!(
                    "Connected, but extension-pack actions could not hot-sync: {}",
                    error
                ));
            }
            agent_for_catalog
                .refresh_action_catalog_index("oauth_connection_state_changed")
                .await;
            if let PendingOAuthTarget::AuthProfile { profile_id } = &oauth_target {
                agent_for_catalog.spawn_custom_api_auth_profile_ready_reconcile(
                    profile_id.clone(),
                    "auth_profile_ready",
                );
            }
            let callback_detail = callback_warnings.join(" ");
            let signal = match &oauth_target {
                PendingOAuthTarget::Integration { service_id } => {
                    oauth_callback_signal_script(service_id, "connected", &callback_detail)
                }
                PendingOAuthTarget::AuthProfile { profile_id } => {
                    oauth_profile_callback_signal_script(profile_id, "connected", &callback_detail)
                }
            };
            let html = format!(
                r#"<!DOCTYPE html>
<html>
<head><title>Connected!</title>
<style>
body {{ font-family: system-ui; background: #1a1a2e; color: #eee; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }}
.card {{ background: #16213e; padding: 2rem; border-radius: 12px; text-align: center; max-width: 400px; }}
.success {{ color: #00d9ff; }}
</style>
</head>
<body>
<div class="card">
<h2 class="success"> {} Connected!</h2>
<p>You can close this window and return to __PRODUCT_NAME__.</p>
<p><a href="/" style="color: #00d9ff;">Return to __PRODUCT_NAME__</a></p>
</div>
{}
</body>
</html>"#,
                escape_html(&callback_label.replace('_', " ")),
                signal
            )
            .replace("__PRODUCT_NAME__", crate::branding::PRODUCT_NAME);
            (StatusCode::OK, Html(html)).into_response()
        }
        Err(e) => {
            let error_text = e.to_string();
            let signal = match &oauth_target {
                PendingOAuthTarget::Integration { service_id } => {
                    oauth_callback_signal_script(service_id, "error", &error_text)
                }
                PendingOAuthTarget::AuthProfile { profile_id } => {
                    oauth_profile_callback_signal_script(profile_id, "error", &error_text)
                }
            };
            let html = format!(
                r#"<!DOCTYPE html>
<html>
<head><title>Connection Failed</title>
<style>
body {{ font-family: system-ui; background: #1a1a2e; color: #eee; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }}
.card {{ background: #16213e; padding: 2rem; border-radius: 12px; text-align: center; max-width: 400px; }}
.error {{ color: #ff6b6b; }}
</style>
</head>
<body>
<div class="card">
<h2 class="error"> Connection Failed</h2>
<p>{}</p>
<p><a href="/" style="color: #00d9ff;">Return to __PRODUCT_NAME__</a></p>
</div>
{}
</body>
</html>"#,
                escape_html(&error_text),
                signal
            )
            .replace("__PRODUCT_NAME__", crate::branding::PRODUCT_NAME);
            (StatusCode::OK, Html(html)).into_response()
        }
    }
}

pub(super) async fn collect_integrations(agent: &crate::core::Agent) -> Vec<IntegrationResponse> {
    let config_dir = agent.config_dir.clone();
    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )
    .ok();

    let mut integrations = Vec::new();

    if let Some((config_fields, config_help, configure_button)) =
        external_integration_config("google_workspace")
    {
        let configured = google_workspace_configured(&config_dir);
        let granted =
            crate::actions::google_workspace::granted_bundles(&config_dir).unwrap_or_default();
        let missing = crate::actions::google_workspace::missing_selected_bundles(&config_dir)
            .unwrap_or_default();
        let pending =
            crate::actions::google_workspace::load_pending_bundles(&config_dir).unwrap_or_default();
        let (status, status_detail) = if !configured && granted.is_empty() {
            (
                "not_configured",
                Some(
                    "Enter the Google OAuth client ID and client secret in Google Workspace, then continue with Google."
                        .to_string(),
                ),
            )
        } else if granted.is_empty() {
            (
                "needs_auth",
                Some(
                    "Choose the Workspace bundles you want, then continue with Google to finish browser sign-in."
                        .to_string(),
                ),
            )
        } else if !missing.is_empty() {
            (
                "needs_auth",
                google_workspace_status_detail(&granted, &missing, &pending),
            )
        } else {
            match tokio::time::timeout(
                INTEGRATION_STATUS_TIMEOUT,
                google_workspace_test_connection(&config_dir),
            )
            .await
            {
                Err(_) => (
                    "error",
                    Some("Google Workspace status check timed out.".to_string()),
                ),
                Ok(Ok(payload)) if google_workspace_test_payload_ok(&payload) => (
                    "connected",
                    google_workspace_status_detail(&granted, &missing, &pending),
                ),
                Ok(Ok(payload)) => (
                    "error",
                    google_workspace_test_issue_detail(&payload)
                        .or_else(|| google_workspace_status_detail(&granted, &missing, &pending))
                        .or_else(|| {
                            Some("Google Workspace health checks reported warnings.".to_string())
                        }),
                ),
                Ok(Err(error)) => ("error", Some(error)),
            }
        };
        let enabled =
            crate::integrations::effective_integration_enabled(&config_dir, "google_workspace");
        integrations.push(IntegrationResponse {
            id: "google_workspace".to_string(),
            name: "Google Workspace".to_string(),
            description: format!(
                "Connect Google Workspace once, then let {} use Gmail, Calendar, read Drive/Docs/Sheets/Chat/Admin data, and access the broader gws CLI surface from the same credential set.",
                crate::branding::PRODUCT_NAME
            ),
            icon: "".to_string(),
            status: status.to_string(),
            enabled,
            status_detail,
            auth_url: None,
            config_fields: Some(config_fields),
            config_help,
            configure_button,
            config_values: Some(google_workspace_config_values(&config_dir)),
        });
    }

    if let Some((config_fields, config_help, configure_button)) =
        external_integration_config("vercel")
    {
        let status =
            crate::actions::vercel::vercel_connection_status(&config_dir, &agent.data_dir).await;
        let enabled = crate::integrations::effective_integration_enabled(&config_dir, "vercel");
        let status_label = if status.connected {
            "connected"
        } else if status.token_configured {
            "error"
        } else {
            "not_configured"
        };
        let status_detail = if let Some(error) = status.error.clone() {
            Some(error)
        } else if status.connected {
            status
                .username
                .as_ref()
                .or(status.email.as_ref())
                .map(|identity| format!("Connected as {}.", identity))
        } else {
            Some("Add a Vercel access token to publish apps externally.".to_string())
        };
        integrations.push(IntegrationResponse {
            id: "vercel".to_string(),
            name: "Vercel".to_string(),
            description: "Publish AgentArk apps to Vercel preview or production deployments."
                .to_string(),
            icon: "".to_string(),
            status: status_label.to_string(),
            enabled,
            status_detail,
            auth_url: None,
            config_fields: Some(config_fields),
            config_help,
            configure_button,
            config_values: Some(crate::actions::vercel::load_vercel_config_values(
                &config_dir,
                &agent.data_dir,
            )),
        });
    }

    for info in agent.integrations.list().await {
        if info.id == "moltbook"
            || info.id == "gmail"
            || info.id == "google_calendar"
            || info.id == "vercel"
        {
            continue;
        }

        // Settings UI "External Integrations" section only supports these today.
        let meta = match external_integration_config(&info.id) {
            Some(m) => m,
            None => continue,
        };

        let enabled = crate::integrations::effective_integration_enabled(&config_dir, &info.id);
        let config_only = integration_uses_config_only_status(&info.id);

        let (status_str, status_detail) = if info.id == "google_calendar" {
            let configured = calendar_oauth_pair(manager.as_ref()).is_some();
            let has_refresh_token =
                oauth_has_refresh_token(stored_secret(manager.as_ref(), "calendar_tokens"));
            if has_refresh_token {
                match tokio::time::timeout(
                    INTEGRATION_STATUS_TIMEOUT,
                    validate_calendar_oauth_connection(&config_dir),
                )
                .await
                {
                    Err(_) => (
                        "error",
                        Some("Calendar status check timed out.".to_string()),
                    ),
                    Ok(Ok(())) => ("connected", None),
                    Ok(Err(error)) => ("error", Some(error)),
                }
            } else if configured {
                (
                    "needs_auth",
                    Some("Google sign-in required to finish connecting Calendar.".to_string()),
                )
            } else {
                ("not_configured", None)
            }
        } else {
            match &info.status {
                crate::integrations::IntegrationStatus::NotConfigured => ("not_configured", None),
                crate::integrations::IntegrationStatus::NeedsAuth => ("needs_auth", None),
                crate::integrations::IntegrationStatus::Connected if config_only => (
                    "configured",
                    Some(config_only_status_detail(&info.id, enabled)),
                ),
                crate::integrations::IntegrationStatus::Connected => ("connected", None),
                crate::integrations::IntegrationStatus::Error(e) => ("error", Some(e.clone())),
            }
        };

        let auth_url = None;

        let (config_fields, config_help, configure_button) = meta;
        integrations.push(IntegrationResponse {
            id: info.id,
            name: info.name,
            description: info.description,
            icon: info.icon,
            status: status_str.to_string(),
            enabled,
            status_detail,
            auth_url,
            config_fields: Some(config_fields),
            config_help,
            configure_button,
            config_values: None,
        });
    }

    integrations
}

/// List all integrations with their status
pub(super) async fn list_integrations(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let integrations = collect_integrations(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "integrations": integrations })),
    )
        .into_response()
}

/// Get auth URL for a specific integration
pub(super) async fn get_integration_auth_url(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if id == "gmail" || id == "google_calendar" || id == "google_workspace" {
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        )
        .ok();
        let auth_url = if id == "gmail" {
            build_gmail_auth_url(&state, &headers, manager.as_ref()).await
        } else if id == "google_workspace" {
            build_google_workspace_auth_url(&state, &headers).await
        } else {
            build_calendar_auth_url(&state, &headers, manager.as_ref()).await
        };
        return match auth_url {
            Ok(url) => (
                StatusCode::OK,
                Json(serde_json::json!({ "auth_url": url.clone(), "url": url })),
            )
                .into_response(),
            Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
        };
    }

    let agent = state.agent.read().await;

    match agent.integrations.get(&id) {
        Some(integration) => {
            let state_token = auth::issue_oauth_state(&state, &id, None).await;
            match integration
                .execute("get_auth_url", &serde_json::json!({"state": state_token}))
                .await
            {
                Ok(result) => (StatusCode::OK, Json(result)).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not found", id),
            }),
        )
            .into_response(),
    }
}

/// Disconnect an integration
pub(super) async fn disconnect_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    // Token/config based integrations don't implement a runtime "disconnect" action.
    // Disconnect here means clearing stored secrets, which updates status immediately.
    if external_integration_config(&id).is_some() {
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        ) {
            Ok(m) => m,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Config error: {}", e),
                    }),
                )
                    .into_response();
            }
        };

        let keys: &[&str] = match id.as_str() {
            "google_workspace" => &[
                crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
                crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
                crate::actions::google_workspace::GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY,
                "gmail_tokens",
                "calendar_tokens",
            ],
            "gmail" => &["gmail_tokens"],
            "google_calendar" => &["calendar_oauth_config", "calendar_tokens"],
            "github" => &["github_token"],
            "vercel" => &[
                crate::actions::vercel::VERCEL_TOKEN_SECRET_KEY,
                crate::actions::vercel::VERCEL_TEAM_ID_SECRET_KEY,
                crate::actions::vercel::VERCEL_PROJECT_ID_SECRET_KEY,
            ],
            "notion" => &["notion_token"],
            "twitter" => &["twitter_bearer_token"],
            "onepassword" => &["onepassword_token", "onepassword_host"],
            "google_places" => &["google_places_api_key"],
            "twilio" => &[
                "twilio_account_sid",
                "twilio_auth_token",
                "twilio_from_number",
                "twilio_config",
            ],
            "ordering" => &[
                "ordering_config",
                "shopify_access_token",
                "shopify_store_url",
                "ordering_webhook_url",
            ],
            "garmin" => &["garmin_token", "garmin_api_base"],
            "whoop" => &["whoop_token"],
            "ga4" => &["ga4_access_token", "ga4_property_id"],
            "gsc" => &["gsc_access_token", "gsc_site_url"],
            "social_analytics" => &[
                "social_twitter_bearer_token",
                "social_ga4_access_token",
                "social_ga4_property_id",
            ],
            "moltbook" => &["moltbook_api_key"],
            _ => &[],
        };

        for key in keys {
            let _ = manager.set_custom_secret(key, None);
        }
        if id == "vercel" {
            let _ = crate::actions::vercel::clear_vercel_config(&config_dir, &data_dir);
        }
        // Also disable the integration (do not auto-use it after disconnect).
        let _ = manager.set_custom_secret(&integration_enabled_key(&id), Some("false".to_string()));
        let _ = manager.set_custom_secret(
            &integration_user_disabled_key(&id),
            Some("false".to_string()),
        );

        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Disconnected"})),
        )
            .into_response();
    }

    if id == "vercel" {
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        let status = crate::actions::vercel::vercel_connection_status(&config_dir, &data_dir).await;
        let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        ) {
            Ok(manager) => manager,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Config error: {}", error),
                    }),
                )
                    .into_response();
            }
        };
        if status.connected {
            let _ = manager
                .set_custom_secret(&integration_enabled_key("vercel"), Some("true".to_string()));
            let _ = manager.set_custom_secret(
                &integration_user_disabled_key("vercel"),
                Some("false".to_string()),
            );
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","enabled":true,"connected":true})),
            )
                .into_response();
        }
        let _ = manager.set_custom_secret(
            &integration_enabled_key("vercel"),
            Some("false".to_string()),
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: status
                    .error
                    .unwrap_or_else(|| "Connect Vercel first.".to_string()),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;

    match agent.integrations.get(&id) {
        Some(integration) => {
            match integration
                .execute("disconnect", &serde_json::json!({}))
                .await
            {
                Ok(_) => (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "ok", "message": "Disconnected"})),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not found", id),
            }),
        )
            .into_response(),
    }
}

async fn configure_vercel(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let token = request
        .get("token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let team_id = request
        .get("team_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let project_id = request
        .get("project_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    let existing = crate::actions::vercel::load_vercel_config_values(&config_dir, &data_dir);
    let has_existing_token = existing
        .get("token_configured")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if token.is_none() && !has_existing_token {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Missing token".to_string(),
            }),
        )
            .into_response();
    }

    if let Some(token) = token.as_deref() {
        if let Err(error) = crate::actions::vercel::validate_vercel_token_value(token).await {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    }

    if let Err(error) = crate::actions::vercel::store_vercel_config(
        &config_dir,
        &data_dir,
        token,
        team_id,
        project_id,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save Vercel config: {}", error),
            }),
        )
            .into_response();
    }

    let status = crate::actions::vercel::vercel_connection_status(&config_dir, &data_dir).await;
    if status.connected {
        if let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        ) {
            let _ = manager
                .set_custom_secret(&integration_enabled_key("vercel"), Some("true".to_string()));
            let _ = manager.set_custom_secret(
                &integration_user_disabled_key("vercel"),
                Some("false".to_string()),
            );
        }
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "connected": true,
                "enabled": true,
                "details": status,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: status
                    .error
                    .unwrap_or_else(|| "Vercel connection test failed".to_string()),
            }),
        )
            .into_response()
    }
}

/// Configure an external integration (store encrypted secrets)
pub(super) async fn configure_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    if external_integration_config(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not supported here", id),
            }),
        )
            .into_response();
    }

    if id == "gmail" {
        return configure_gmail(State(state), Json(request)).await;
    }
    if id == "google_calendar" {
        return configure_calendar(State(state), Json(request)).await;
    }
    if id == "google_workspace" {
        return configure_google_workspace(State(state), Json(request)).await;
    }
    if id == "vercel" {
        return configure_vercel(State(state), Json(request)).await;
    }
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let set = |k: &str, v: Option<String>| -> Result<(), String> {
        manager
            .set_custom_secret(k, v)
            .map_err(|e| format!("Failed to save {}: {}", k, e))
    };

    let require_str = |key: &str| -> Result<String, Response> {
        match request
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            Some(v) => Ok(v.to_string()),
            None => Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Missing {}", key),
                }),
            )
                .into_response()),
        }
    };

    let opt_str = |key: &str| -> Option<String> {
        request
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };

    let result: Result<(), Response> = (|| {
        match id.as_str() {
            "github" => {
                let token = require_str("token")?;
                set("github_token", Some(token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "notion" => {
                let token = require_str("token")?;
                set("notion_token", Some(token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "twitter" => {
                let token = require_str("bearer_token")?;
                set("twitter_bearer_token", Some(token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "onepassword" => {
                let token = require_str("token")?;
                let host = opt_str("host");
                set("onepassword_token", Some(token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                // Only override host if provided; otherwise keep existing/default.
                if let Some(h) = host {
                    set("onepassword_host", Some(h)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
            }
            "google_places" => {
                let api_key = require_str("api_key")?;
                set("google_places_api_key", Some(api_key)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "twilio" => {
                let sid = require_str("account_sid")?;
                let token = require_str("auth_token")?;
                let from = require_str("from_number")?;
                set("twilio_account_sid", Some(sid.clone())).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                set("twilio_auth_token", Some(token.clone())).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                set("twilio_from_number", Some(from.clone())).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                // Backward compatibility with older single-secret config formats.
                let cfg = serde_json::json!({
                    "account_sid": sid,
                    "auth_token": token,
                    "from_number": from,
                });
                set("twilio_config", Some(cfg.to_string())).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "ordering" => {
                let cfg = require_str("config_json")?;
                set("ordering_config", Some(cfg)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "garmin" => {
                let token = require_str("token")?;
                set("garmin_token", Some(token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                if let Some(api_base) = opt_str("api_base") {
                    set("garmin_api_base", Some(api_base)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
            }
            "whoop" => {
                let token = require_str("token")?;
                set("whoop_token", Some(token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            "ga4" => {
                let access_token = require_str("access_token")?;
                set("ga4_access_token", Some(access_token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                if let Some(property_id) = opt_str("property_id") {
                    set("ga4_property_id", Some(property_id)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
            }
            "gsc" => {
                let access_token = require_str("access_token")?;
                set("gsc_access_token", Some(access_token)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
                if let Some(site_url) = opt_str("site_url") {
                    set("gsc_site_url", Some(site_url)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
            }
            "social_analytics" => {
                let has_any = opt_str("social_twitter_bearer_token").is_some()
                    || opt_str("social_ga4_access_token").is_some()
                    || opt_str("social_ga4_property_id").is_some();
                if !has_any {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Provide at least one social analytics credential".to_string(),
                        }),
                    )
                        .into_response());
                }
                if let Some(twitter_token) = opt_str("social_twitter_bearer_token") {
                    set("social_twitter_bearer_token", Some(twitter_token)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
                if let Some(ga4_token) = opt_str("social_ga4_access_token") {
                    set("social_ga4_access_token", Some(ga4_token)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
                if let Some(ga4_property) = opt_str("social_ga4_property_id") {
                    set("social_ga4_property_id", Some(ga4_property)).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: e }),
                        )
                            .into_response()
                    })?;
                }
            }
            "moltbook" => {
                let api_key = require_str("api_key")?;
                set("moltbook_api_key", Some(api_key)).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response()
                })?;
            }
            _ => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("Integration '{}' not supported here", id),
                    }),
                )
                    .into_response());
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            // Validate connectivity (lightweight call) and only enable on success.
            let agent = state.agent.read().await;
            if let Some(integration) = agent.integrations.get(&id) {
                let status = integration.status().await;
                let config_only = integration_uses_config_only_status(&id);
                match status {
                    crate::integrations::IntegrationStatus::Connected => {
                        if let Ok(manager) =
                            crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                                &agent.config_dir,
                                Some(&agent.data_dir),
                            )
                        {
                            let _ = manager.set_custom_secret(
                                &integration_enabled_key(&id),
                                Some((!config_only).to_string()),
                            );
                            let _ = manager.set_custom_secret(
                                &integration_user_disabled_key(&id),
                                Some("false".to_string()),
                            );
                        }
                        if !config_only {
                            let sync_ctx =
                                crate::core::connectivity::integration_sync::context_from_agent(
                                    &agent,
                                    Some(state.agent.clone()),
                                );
                            if let Err(error) =
                                crate::core::connectivity::integration_sync::ensure_default_enabled(
                                    &sync_ctx, &id,
                                )
                                .await
                            {
                                tracing::warn!(
                                    "Failed to enable default background sync for {}: {}",
                                    id,
                                    error
                                );
                            }
                        }
                        agent
                            .refresh_action_catalog_index("prebuilt_integration_connected")
                            .await;
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "status": "ok",
                                "enabled": !config_only,
                                "connected": !config_only,
                                "configured": config_only,
                                "status_hint": if config_only { "configured" } else { "connected" },
                            })),
                        )
                            .into_response()
                    }
                    crate::integrations::IntegrationStatus::Error(e) => {
                        // Disable on failure.
                        if let Ok(manager) =
                            crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                                &agent.config_dir,
                                Some(&agent.data_dir),
                            )
                        {
                            let _ = manager.set_custom_secret(
                                &integration_enabled_key(&id),
                                Some("false".to_string()),
                            );
                            let _ = manager.set_custom_secret(
                                &integration_user_disabled_key(&id),
                                Some("false".to_string()),
                            );
                        }
                        agent
                            .refresh_action_catalog_index("prebuilt_integration_disabled")
                            .await;
                        (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: format!("Connection test failed: {}", e),
                            }),
                        )
                            .into_response()
                    }
                    other => {
                        // Any non-connected state should remain disabled.
                        if let Ok(manager) =
                            crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                                &agent.config_dir,
                                Some(&agent.data_dir),
                            )
                        {
                            let _ = manager.set_custom_secret(
                                &integration_enabled_key(&id),
                                Some("false".to_string()),
                            );
                            let _ = manager.set_custom_secret(
                                &integration_user_disabled_key(&id),
                                Some("false".to_string()),
                            );
                        }
                        agent
                            .refresh_action_catalog_index("prebuilt_integration_disabled")
                            .await;
                        (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: format!("Integration not ready: {:?}", other),
                            }),
                        )
                            .into_response()
                    }
                }
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("Integration '{}' not found", id),
                    }),
                )
                    .into_response()
            }
        }
        Err(resp) => resp,
    }
}

/// Enable an integration (only succeeds if it is currently connected)
pub(super) async fn enable_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if external_integration_config(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not supported here", id),
            }),
        )
            .into_response();
    }

    if id == "gmail" || id == "google_calendar" || id == "google_workspace" {
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        ) {
            Ok(m) => m,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Config error: {}", e),
                    }),
                )
                    .into_response();
            }
        };
        let has_refresh_token = if id == "gmail" {
            oauth_has_refresh_token(stored_secret(Some(&manager), "gmail_tokens"))
        } else if id == "google_workspace" {
            crate::actions::google_workspace::summarize_connection_status(&config_dir)
                .map(|(connected, granted, missing)| {
                    connected && !granted.is_empty() && missing.is_empty()
                })
                .unwrap_or(false)
        } else {
            oauth_has_refresh_token(stored_secret(Some(&manager), "calendar_tokens"))
        };
        let validation = if !has_refresh_token {
            Err("Complete Google sign-in first.".to_string())
        } else if id == "gmail" {
            validate_gmail_oauth_connection(&config_dir).await
        } else if id == "google_workspace" {
            google_workspace_test_connection(&config_dir)
                .await
                .and_then(|payload| {
                    if google_workspace_test_payload_ok(&payload) {
                        Ok(())
                    } else {
                        Err(
                            google_workspace_test_issue_detail(&payload).unwrap_or_else(|| {
                                "Google Workspace health checks reported warnings.".to_string()
                            }),
                        )
                    }
                })
        } else {
            validate_calendar_oauth_connection(&config_dir).await
        };
        if has_refresh_token && validation.is_ok() {
            let integration_ids = builtin_runtime_integration_ids_for_service(&config_dir, &id);
            for integration_id in &integration_ids {
                let _ = manager.set_custom_secret(
                    &integration_enabled_key(integration_id),
                    Some("true".to_string()),
                );
                let _ = manager.set_custom_secret(
                    &integration_user_disabled_key(integration_id),
                    Some("false".to_string()),
                );
            }
            {
                let agent = state.agent.read().await;
                let sync_ctx = crate::core::connectivity::integration_sync::context_from_agent(
                    &agent,
                    Some(state.agent.clone()),
                );
                if let Err(error) =
                    crate::core::connectivity::integration_sync::ensure_default_enabled(
                        &sync_ctx, &id,
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to enable default background sync for {}: {}",
                        id,
                        error
                    );
                }
            }
            refresh_connected_action_surfaces(&state, "prebuilt_integration_enabled").await;
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","enabled":true,"connected":true})),
            )
                .into_response();
        }
        let integration_ids = builtin_runtime_integration_ids_for_service(&config_dir, &id);
        for integration_id in &integration_ids {
            let _ = manager.set_custom_secret(
                &integration_enabled_key(integration_id),
                Some("false".to_string()),
            );
            let _ = manager.set_custom_secret(
                &integration_user_disabled_key(integration_id),
                Some("false".to_string()),
            );
        }
        refresh_connected_action_surfaces(&state, "prebuilt_integration_disabled").await;
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: if !has_refresh_token {
                    "Complete Google sign-in first.".to_string()
                } else {
                    validation
                        .err()
                        .unwrap_or_else(|| "Google connection validation failed.".to_string())
                },
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    let Some(integration) = agent.integrations.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not found", id),
            }),
        )
            .into_response();
    };

    match integration.status().await {
        crate::integrations::IntegrationStatus::Connected => {
            let config_only = integration_uses_config_only_status(&id);
            if let Ok(manager) =
                crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                    &agent.config_dir,
                    Some(&agent.data_dir),
                )
            {
                let _ = manager
                    .set_custom_secret(&integration_enabled_key(&id), Some("true".to_string()));
                let _ = manager.set_custom_secret(
                    &integration_user_disabled_key(&id),
                    Some("false".to_string()),
                );
            }
            let sync_ctx = crate::core::connectivity::integration_sync::context_from_agent(
                &agent,
                Some(state.agent.clone()),
            );
            if let Err(error) =
                crate::core::connectivity::integration_sync::ensure_default_enabled(&sync_ctx, &id)
                    .await
            {
                tracing::warn!(
                    "Failed to enable default background sync for {}: {}",
                    id,
                    error
                );
            }
            agent
                .refresh_action_catalog_index("prebuilt_integration_enabled")
                .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status":"ok",
                    "enabled":true,
                    "connected": !config_only,
                    "configured": config_only,
                    "status_hint": if config_only { "configured" } else { "connected" },
                })),
            )
                .into_response()
        }
        crate::integrations::IntegrationStatus::Error(e) => {
            if let Ok(manager) =
                crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                    &agent.config_dir,
                    Some(&agent.data_dir),
                )
            {
                let _ = manager
                    .set_custom_secret(&integration_enabled_key(&id), Some("false".to_string()));
                let _ = manager.set_custom_secret(
                    &integration_user_disabled_key(&id),
                    Some("false".to_string()),
                );
            }
            agent
                .refresh_action_catalog_index("prebuilt_integration_disabled")
                .await;
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Connection test failed: {}", e),
                }),
            )
                .into_response()
        }
        other => {
            if let Ok(manager) =
                crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                    &agent.config_dir,
                    Some(&agent.data_dir),
                )
            {
                let _ = manager
                    .set_custom_secret(&integration_enabled_key(&id), Some("false".to_string()));
                let _ = manager.set_custom_secret(
                    &integration_user_disabled_key(&id),
                    Some("false".to_string()),
                );
            }
            agent
                .refresh_action_catalog_index("prebuilt_integration_disabled")
                .await;
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Integration not ready: {:?}", other),
                }),
            )
                .into_response()
        }
    }
}

/// Disable an integration (keeps stored credentials but prevents agent usage)
pub(super) async fn disable_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if external_integration_config(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not supported here", id),
            }),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    if let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        let integration_ids = if matches!(
            id.as_str(),
            "gmail" | "google_calendar" | "calendar" | "google_workspace"
        ) {
            builtin_runtime_integration_ids_for_service(&config_dir, &id)
        } else {
            vec![id.as_str()]
        };
        for integration_id in integration_ids {
            let _ = manager.set_custom_secret(
                &integration_enabled_key(integration_id),
                Some("false".to_string()),
            );
            let _ = manager.set_custom_secret(
                &integration_user_disabled_key(integration_id),
                Some("true".to_string()),
            );
        }
    }
    refresh_connected_action_surfaces(&state, "prebuilt_integration_disabled").await;
    (
        StatusCode::OK,
        Json(serde_json::json!({"status":"ok","enabled":false})),
    )
        .into_response()
}

/// Test an integration connection; if test fails, the integration is disabled.
pub(super) async fn test_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if external_integration_config(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not supported here", id),
            }),
        )
            .into_response();
    }

    if id == "gmail" {
        return gmail_test(State(state)).await;
    }
    if id == "google_calendar" {
        return calendar_test(State(state)).await;
    }
    if id == "google_workspace" {
        let config_dir = { state.agent.read().await.config_dir.clone() };
        return match google_workspace_test_connection(&config_dir).await {
            Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
            Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
        };
    }
    if id == "vercel" {
        let (config_dir, data_dir) = {
            let agent = state.agent.read().await;
            (agent.config_dir.clone(), agent.data_dir.clone())
        };
        let status = crate::actions::vercel::vercel_connection_status(&config_dir, &data_dir).await;
        if status.connected {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","connected":true,"details": status})),
            )
                .into_response();
        }
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: status
                    .error
                    .unwrap_or_else(|| "Vercel is not connected.".to_string()),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    let Some(integration) = agent.integrations.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Integration '{}' not found", id),
            }),
        )
            .into_response();
    };
    match integration.status().await {
        crate::integrations::IntegrationStatus::Connected if integration_uses_config_only_status(&id) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status":"configured",
                "connected":false,
                "detail": config_only_status_detail(&id, crate::integrations::effective_integration_enabled(&agent.config_dir, &id)),
            })),
        )
            .into_response(),
        crate::integrations::IntegrationStatus::Connected => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","connected":true})),
        )
            .into_response(),
        crate::integrations::IntegrationStatus::Error(e) => {
            if let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                &agent.config_dir,
                Some(&agent.data_dir),
            ) {
                let _ = manager
                    .set_custom_secret(&integration_enabled_key(&id), Some("false".to_string()));
            }
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Connection test failed: {}", e),
                }),
            )
                .into_response()
        }
        other => {
            if let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                &agent.config_dir,
                Some(&agent.data_dir),
            ) {
                let _ = manager
                    .set_custom_secret(&integration_enabled_key(&id), Some("false".to_string()));
            }
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Integration not ready: {:?}", other),
                }),
            )
                .into_response()
        }
    }
}

async fn google_workspace_test_connection(
    config_dir: &std::path::Path,
) -> Result<serde_json::Value, String> {
    let checks = crate::actions::google_workspace::test_selected_bundles(config_dir)
        .await
        .map_err(|e| e.to_string())?;
    let mut ok = true;
    for message in checks.values() {
        let lowered = message.to_ascii_lowercase();
        if lowered.contains("failed")
            || lowered.contains("unavailable")
            || lowered.contains("needs additional access")
            || lowered.contains("reconnect")
        {
            ok = false;
            break;
        }
    }
    Ok(serde_json::json!({
        "status": if ok { "ok" } else { "warning" },
        "checks": checks
    }))
}

pub(super) async fn configure_google_workspace(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let credentials_json = request
        .get("credentials_json")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let manual_client_id = request
        .get("client_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let manual_client_secret = request
        .get("client_secret")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let selected_bundles = request
        .get("service_bundles")
        .map(crate::actions::google_workspace::parse_bundle_list)
        .unwrap_or_else(|| {
            crate::actions::google_workspace::load_saved_bundles(&config_dir)
                .unwrap_or_else(|_| crate::actions::google_workspace::default_bundles())
        });

    let existing_saved =
        crate::actions::google_workspace::load_saved_workspace_client_config(&config_dir)
            .ok()
            .flatten();
    let next_config = if let Some(raw) = credentials_json {
        match crate::actions::google_workspace::parse_credentials_json(raw) {
            Ok(config) => Some(config),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    } else if let (Some(client_id), Some(client_secret)) = (manual_client_id, manual_client_secret)
    {
        Some(
            crate::actions::google_workspace::GoogleWorkspaceClientConfig {
                client_id: client_id.to_string(),
                client_secret: client_secret.to_string(),
            },
        )
    } else {
        None
    };

    let credentials_changed = next_config.as_ref().is_some_and(|next| {
        existing_saved.as_ref().is_none_or(|existing| {
            existing.client_id != next.client_id || existing.client_secret != next.client_secret
        })
    });

    if let Some(next_config) = next_config.as_ref() {
        if let Err(error) =
            crate::actions::google_workspace::save_workspace_client_config(&config_dir, next_config)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to save Google OAuth client: {}", error),
                }),
            )
                .into_response();
        }
    }
    if let Err(error) =
        crate::actions::google_workspace::save_selected_bundles(&config_dir, &selected_bundles)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save Google Workspace bundles: {}", error),
            }),
        )
            .into_response();
    }
    if let Ok(pending) = crate::actions::google_workspace::load_pending_bundles(&config_dir) {
        let retained = pending
            .into_iter()
            .filter(|bundle| !selected_bundles.iter().any(|selected| selected == bundle))
            .collect::<Vec<_>>();
        if let Err(error) =
            crate::actions::google_workspace::save_pending_bundles(&config_dir, &retained)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!(
                        "Failed to update pending Google Workspace bundle requests: {}",
                        error
                    ),
                }),
            )
                .into_response();
        }
    }
    if credentials_changed {
        for key in [
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            crate::actions::google_workspace::GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY,
            "gmail_tokens",
            "calendar_tokens",
        ] {
            if let Err(error) = manager.set_custom_secret(key, None) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!(
                            "Failed to reset Google OAuth state after client change: {}",
                            error
                        ),
                    }),
                )
                    .into_response();
            }
        }
        for key in [
            integration_enabled_key("google_workspace"),
            integration_enabled_key("gmail"),
            integration_enabled_key("google_calendar"),
        ] {
            if let Err(error) = manager.set_custom_secret(&key, Some("false".to_string())) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!(
                            "Failed to reset Google OAuth state after client change: {}",
                            error
                        ),
                    }),
                )
                    .into_response();
            }
        }
    }
    let reconnect_required = credentials_changed
        || !google_workspace_configured(&config_dir)
        || crate::actions::google_workspace::missing_selected_bundles(&config_dir)
            .map(|missing| !missing.is_empty())
            .unwrap_or(true)
        || crate::actions::google_workspace::granted_bundles(&config_dir)
            .map(|granted| granted.is_empty())
            .unwrap_or(true);
    let integration_ids = if reconnect_required {
        vec!["google_workspace"]
    } else {
        builtin_runtime_integration_ids_for_service(&config_dir, "google_workspace")
    };
    for integration_id in integration_ids {
        let _ = manager.set_custom_secret(
            &integration_enabled_key(integration_id),
            Some((!reconnect_required).to_string()),
        );
        let _ = manager.set_custom_secret(
            &integration_user_disabled_key(integration_id),
            Some("false".to_string()),
        );
    }
    refresh_connected_action_surfaces(&state, "prebuilt_integration_configured").await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "selected_bundles": selected_bundles,
            "credentials_saved": next_config.is_some()
        })),
    )
        .into_response()
}

/// Configure Gmail OAuth credentials
pub(super) async fn configure_gmail(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let client_id = match request.get("client_id").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing client_id".to_string(),
                }),
            )
                .into_response();
        }
    };
    let client_secret = match request.get("client_secret").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing client_secret".to_string(),
                }),
            )
                .into_response();
        }
    };

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let creds = serde_json::json!({
        "client_id": client_id,
        "client_secret": client_secret
    });

    match manager.set_custom_secret("gmail_oauth_config", Some(creds.to_string())) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

// ==================== Calendar API ====================

/// Configure Calendar OAuth credentials (same Google project, calendar scope)
pub(super) async fn configure_calendar(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let client_id = match request.get("client_id").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing client_id".to_string(),
                }),
            )
                .into_response();
        }
    };
    let client_secret = match request.get("client_secret").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing client_secret".to_string(),
                }),
            )
                .into_response();
        }
    };

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let creds = serde_json::json!({
        "client_id": client_id,
        "client_secret": client_secret
    });

    match manager.set_custom_secret("calendar_oauth_config", Some(creds.to_string())) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn calendar_oauth_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let (config_dir, data_dir) = {
        let a = state.agent.read().await;
        (a.config_dir.clone(), a.data_dir.clone())
    };
    let stored_creds = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .ok()
    .and_then(|mgr| {
        mgr.get_custom_secret("calendar_oauth_config")
            .ok()
            .flatten()
    })
    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok());

    // Fall back to Gmail credentials (same Google project)
    let gmail_creds = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .ok()
    .and_then(|mgr| mgr.get_custom_secret("gmail_oauth_config").ok().flatten())
    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok());

    let client_id = std::env::var("CALENDAR_CLIENT_ID")
        .ok()
        .or_else(|| {
            stored_creds.as_ref().and_then(|v| {
                v.get("client_id")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .or_else(|| {
            gmail_creds.as_ref().and_then(|v| {
                v.get("client_id")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        });

    let client_id = match client_id {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "Calendar not configured. Add credentials in Settings > Calendar, or connect Gmail first (same Google project).".to_string() })).into_response(),
    };

    let redirect_uri = match oauth_redirect_uri_for_request(&state, &headers, None) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };
    let (state_token, code_challenge) =
        auth::issue_oauth_state_with_pkce(&state, "google_calendar", Some(redirect_uri.clone()))
            .await;
    let auth_url =
        format_calendar_auth_url(&client_id, &state_token, &code_challenge, &redirect_uri);

    (
        StatusCode::OK,
        Json(serde_json::json!({ "auth_url": auth_url })),
    )
        .into_response()
}

async fn calendar_exchange_code(
    state: &AppState,
    redirect_uri: &str,
    code: &str,
    pkce_verifier: Option<&str>,
) -> Result<(), String> {
    let (config_dir, data_dir) = {
        let a = state.agent.read().await;
        (a.config_dir.clone(), a.data_dir.clone())
    };
    let stored_creds = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .ok()
    .and_then(|mgr| {
        mgr.get_custom_secret("calendar_oauth_config")
            .ok()
            .flatten()
    })
    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok());

    let gmail_creds = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .ok()
    .and_then(|mgr| mgr.get_custom_secret("gmail_oauth_config").ok().flatten())
    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok());

    let client_id = std::env::var("CALENDAR_CLIENT_ID")
        .ok()
        .or_else(|| {
            stored_creds.as_ref().and_then(|v| {
                v.get("client_id")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .or_else(|| {
            gmail_creds.as_ref().and_then(|v| {
                v.get("client_id")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .ok_or_else(|| "Calendar client_id not configured".to_string())?;
    let client_secret = std::env::var("CALENDAR_CLIENT_SECRET")
        .ok()
        .or_else(|| {
            stored_creds.as_ref().and_then(|v| {
                v.get("client_secret")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .or_else(|| {
            gmail_creds.as_ref().and_then(|v| {
                v.get("client_secret")
                    .and_then(|c| c.as_str())
                    .map(String::from)
            })
        })
        .ok_or_else(|| "Calendar client_secret not configured".to_string())?;

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let mut params = vec![
        ("client_id", client_id.as_str().to_string()),
        ("client_secret", client_secret.as_str().to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("grant_type", "authorization_code".to_string()),
    ];
    if let Some(verifier) = pkce_verifier {
        params.push(("code_verifier", verifier.to_string()));
    }

    let resp = http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("Token exchange failed ({})", status));
    }

    #[derive(Deserialize)]
    struct TokenResp {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: i64,
    }

    let token: TokenResp = resp
        .json()
        .await
        .map_err(|e| format!("Invalid token response: {}", e))?;

    let now = chrono::Utc::now().timestamp();
    let tokens = serde_json::json!({
        "access_token": token.access_token,
        "refresh_token": token.refresh_token.unwrap_or_default(),
        "expires_at": now + token.expires_in
    });

    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    )
    .map_err(|e| format!("Secure storage error: {}", e))?;
    let payload = serde_json::to_string(&tokens).unwrap_or_default();
    manager
        .set_custom_secret("calendar_tokens", Some(payload))
        .map_err(|e| format!("Failed to save tokens: {}", e))?;
    set_builtin_integration_enabled(&config_dir, &data_dir, &["google_calendar"], true)?;
    set_builtin_integration_user_disabled(&config_dir, &data_dir, &["google_calendar"], false)?;

    Ok(())
}

pub(super) async fn calendar_status(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(_) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"connected": false})),
            )
                .into_response();
        }
    };
    let payload = match manager.get_custom_secret("calendar_tokens") {
        Ok(Some(v)) => v,
        _ => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"connected": false})),
            )
                .into_response();
        }
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).unwrap_or_else(|_| serde_json::json!({}));
    let has_refresh = parsed
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let expires_at = parsed
        .get("expires_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let validation_error = if has_refresh {
        validate_calendar_oauth_connection(&config_dir).await.err()
    } else {
        None
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "connected": has_refresh && validation_error.is_none(),
            "expires_at": expires_at,
            "error": validation_error
        })),
    )
        .into_response()
}

pub(super) async fn calendar_test(State(state): State<AppState>) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };

    let access_token = match crate::actions::calendar::ensure_access_token(&config_dir).await {
        Ok(token) => token,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Calendar token error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("HTTP error: {}", e),
                }),
            )
                .into_response();
        }
    };

    let resp = client
        .get("https://www.googleapis.com/calendar/v3/calendars/primary")
        .bearer_auth(&access_token)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let data: serde_json::Value = r.json().await.unwrap_or_default();
            let summary = data
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Primary");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "calendar": summary
                })),
            )
                .into_response()
        }
        Ok(r) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Calendar API error: {}", r.status()),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Request failed: {}", e),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_builtin_integration_enabled_persists_enabled_flags() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_dir = temp.path().join("config");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::create_dir_all(&data_dir).expect("data dir");

        set_builtin_integration_enabled(
            &config_dir,
            &data_dir,
            &["google_workspace", "gmail"],
            true,
        )
        .expect("enable integrations");

        let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        )
        .expect("secure manager");
        assert_eq!(
            manager
                .get_custom_secret(&integration_enabled_key("google_workspace"))
                .expect("workspace secret read"),
            Some("true".to_string())
        );
        assert_eq!(
            manager
                .get_custom_secret(&integration_enabled_key("gmail"))
                .expect("gmail secret read"),
            Some("true".to_string())
        );
    }
}
