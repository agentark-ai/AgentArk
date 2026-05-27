//! Generic auth profile control plane for skills, plugins, custom APIs, and MCP servers.
//!
//! Profiles are stored encrypted in the shared storage layer and expose a single
//! resolution path for runtime HTTP-style auth application.

use anyhow::{anyhow, bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::integrations::oauth::{OAuthClient, OAuthConfig, OAuthConfigInput};
use crate::storage::Storage;

const AUTH_PROFILES_KEY: &str = "skill_auth:profiles:v1";
const DEFAULT_OAUTH_REDIRECT_URI: &str = "http://localhost:8990/oauth/callback";

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode auth profile payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode auth profile payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

fn sanitize_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sanitize_string(value: &str) -> String {
    value.trim().to_string()
}

fn sanitize_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn sanitize_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .collect()
}

fn parse_rfc3339(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn timestamp_to_rfc3339(timestamp: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0).map(|value| value.to_rfc3339())
}

fn is_expired(timestamp: Option<&str>) -> bool {
    timestamp
        .and_then(parse_rfc3339)
        .is_some_and(|value| value <= chrono::Utc::now())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfileScope {
    #[default]
    Global,
    User,
    Tenant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfileKind {
    ApiKey,
    Bearer,
    Basic,
    Header,
    Query,
    OAuth2,
    CookieSession,
    BrowserSession,
    ServiceAccount,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfileStatus {
    Ready,
    NeedsConnect,
    NeedsSecret,
    Expired,
    Revoked,
    Disabled,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthCookieRecord {
    pub name: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuth2ProfileConfigRecord {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub use_pkce: bool,
    #[serde(default)]
    pub extra_auth_params: BTreeMap<String, String>,
    #[serde(default)]
    pub extra_token_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuth2TokenRecord {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refreshed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthProfileMaterial {
    ApiKey {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query_name: Option<String>,
    },
    Bearer {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<String>,
    },
    Basic {
        username: String,
        password: String,
    },
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
        value: String,
    },
    OAuth2 {
        config: Box<OAuth2ProfileConfigRecord>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<OAuth2TokenRecord>,
    },
    CookieSession {
        #[serde(default)]
        cookies: Vec<AuthCookieRecord>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
    },
    BrowserSession {
        browser_profile_id: String,
        #[serde(default)]
        cookies: Vec<AuthCookieRecord>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        login_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_captured_at: Option<String>,
    },
    ServiceAccount {
        json: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_header: Option<String>,
    },
}

impl AuthProfileMaterial {
    fn kind(&self) -> AuthProfileKind {
        match self {
            Self::ApiKey { .. } => AuthProfileKind::ApiKey,
            Self::Bearer { .. } => AuthProfileKind::Bearer,
            Self::Basic { .. } => AuthProfileKind::Basic,
            Self::Header { .. } => AuthProfileKind::Header,
            Self::Query { .. } => AuthProfileKind::Query,
            Self::OAuth2 { .. } => AuthProfileKind::OAuth2,
            Self::CookieSession { .. } => AuthProfileKind::CookieSession,
            Self::BrowserSession { .. } => AuthProfileKind::BrowserSession,
            Self::ServiceAccount { .. } => AuthProfileKind::ServiceAccount,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfileRecord {
    pub id: String,
    pub name: String,
    pub kind: AuthProfileKind,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub scope: AuthProfileScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub material: AuthProfileMaterial,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthProfileUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AuthProfileKind>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<AuthProfileScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<AuthProfileMaterial>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthProfileSummary {
    pub total: usize,
    pub ready: usize,
    pub needs_attention: usize,
    pub oauth: usize,
    pub cookie_or_browser: usize,
    pub revoked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthProfileListResponse {
    pub summary: AuthProfileSummary,
    pub profiles: Vec<AuthProfileView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthProfileMaterialView {
    ApiKey {
        configured: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query_name: Option<String>,
    },
    Bearer {
        configured: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<String>,
    },
    Basic {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        password_configured: bool,
    },
    Header {
        name: String,
        configured: bool,
    },
    Query {
        name: String,
        configured: bool,
    },
    OAuth2 {
        auth_url: String,
        token_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redirect_uri: Option<String>,
        #[serde(default)]
        scopes: Vec<String>,
        connected: bool,
        has_refresh_token: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_expires_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_type: Option<String>,
    },
    CookieSession {
        cookie_count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<String>,
    },
    BrowserSession {
        browser_profile_id: String,
        cookie_count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        login_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_captured_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<String>,
    },
    ServiceAccount {
        configured: bool,
        has_access_token: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfileView {
    pub id: String,
    pub name: String,
    pub kind: AuthProfileKind,
    pub enabled: bool,
    pub scope: AuthProfileScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    pub status: AuthProfileStatus,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub material: AuthProfileMaterialView,
}

#[derive(Debug, Clone, Default)]
pub struct HttpAuthOverlay {
    pub headers: BTreeMap<String, String>,
    pub query: BTreeMap<String, String>,
    pub basic: Option<(String, String)>,
    pub browser_profile_id: Option<String>,
}

impl HttpAuthOverlay {
    pub fn cookie_header(&self) -> Option<String> {
        self.headers
            .get("Cookie")
            .or_else(|| self.headers.get("cookie"))
            .cloned()
    }

    pub fn apply_to_url(&self, url: &mut reqwest::Url) {
        if self.query.is_empty() {
            return;
        }
        let mut pairs = url.query_pairs_mut();
        for (key, value) in &self.query {
            pairs.append_pair(key, value);
        }
    }

    pub fn apply_to_request_builder(
        &self,
        mut builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        if let Some((username, password)) = &self.basic {
            builder = builder.basic_auth(username, Some(password));
        }
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        Ok(builder)
    }
}

#[derive(Debug, Clone)]
pub struct AuthProfileResolution {
    pub overlay: HttpAuthOverlay,
}

fn sanitize_cookie(mut cookie: AuthCookieRecord) -> Option<AuthCookieRecord> {
    cookie.name = sanitize_string(&cookie.name);
    cookie.value = cookie.value.trim().to_string();
    cookie.domain = sanitize_text(cookie.domain);
    cookie.path = sanitize_text(cookie.path);
    cookie.expires_at = sanitize_text(cookie.expires_at);
    if cookie.name.is_empty() || cookie.value.trim().is_empty() {
        return None;
    }
    Some(cookie)
}

fn latest_cookie_expiry(cookies: &[AuthCookieRecord]) -> Option<String> {
    cookies
        .iter()
        .filter_map(|cookie| cookie.expires_at.as_deref())
        .filter_map(parse_rfc3339)
        .max()
        .map(|value| value.to_rfc3339())
}

fn active_cookies(cookies: &[AuthCookieRecord]) -> Vec<AuthCookieRecord> {
    cookies
        .iter()
        .filter(|cookie| !is_expired(cookie.expires_at.as_deref()))
        .cloned()
        .collect()
}

fn sanitize_material(material: AuthProfileMaterial) -> AuthProfileMaterial {
    match material {
        AuthProfileMaterial::ApiKey {
            value,
            header,
            query_name,
        } => AuthProfileMaterial::ApiKey {
            value: value.trim().to_string(),
            header: sanitize_text(header),
            query_name: sanitize_text(query_name),
        },
        AuthProfileMaterial::Bearer { token, header } => AuthProfileMaterial::Bearer {
            token: token.trim().to_string(),
            header: sanitize_text(header),
        },
        AuthProfileMaterial::Basic { username, password } => AuthProfileMaterial::Basic {
            username: username.trim().to_string(),
            password: password.trim().to_string(),
        },
        AuthProfileMaterial::Header { name, value } => AuthProfileMaterial::Header {
            name: sanitize_string(&name),
            value: value.trim().to_string(),
        },
        AuthProfileMaterial::Query { name, value } => AuthProfileMaterial::Query {
            name: sanitize_string(&name),
            value: value.trim().to_string(),
        },
        AuthProfileMaterial::OAuth2 {
            mut config,
            mut tokens,
        } => {
            config.client_id = sanitize_string(&config.client_id);
            config.client_secret = config.client_secret.trim().to_string();
            config.auth_url = sanitize_string(&config.auth_url);
            config.token_url = sanitize_string(&config.token_url);
            config.redirect_uri = sanitize_text(config.redirect_uri);
            config.scopes = sanitize_string_list(config.scopes);
            config.auth_header = sanitize_text(config.auth_header);
            config.extra_auth_params = sanitize_headers(config.extra_auth_params);
            config.extra_token_params = sanitize_headers(config.extra_token_params);
            config.prompt = sanitize_text(config.prompt);
            config.access_type = sanitize_text(config.access_type);
            if let Some(token) = tokens.as_mut() {
                token.access_token = token.access_token.trim().to_string();
                token.refresh_token = sanitize_text(token.refresh_token.clone());
                token.expires_at = sanitize_text(token.expires_at.clone());
                token.refresh_expires_at = sanitize_text(token.refresh_expires_at.clone());
                token.token_type = sanitize_text(token.token_type.clone());
                token.scopes = sanitize_string_list(token.scopes.clone());
                token.last_refreshed_at = sanitize_text(token.last_refreshed_at.clone());
            }
            AuthProfileMaterial::OAuth2 {
                config: Box::new(*config),
                tokens,
            }
        }
        AuthProfileMaterial::CookieSession {
            cookies,
            headers,
            origin,
        } => AuthProfileMaterial::CookieSession {
            cookies: cookies.into_iter().filter_map(sanitize_cookie).collect(),
            headers: sanitize_headers(headers),
            origin: sanitize_text(origin),
        },
        AuthProfileMaterial::BrowserSession {
            browser_profile_id,
            cookies,
            headers,
            login_url,
            last_captured_at,
        } => AuthProfileMaterial::BrowserSession {
            browser_profile_id: sanitize_string(&browser_profile_id),
            cookies: cookies.into_iter().filter_map(sanitize_cookie).collect(),
            headers: sanitize_headers(headers),
            login_url: sanitize_text(login_url),
            last_captured_at: sanitize_text(last_captured_at),
        },
        AuthProfileMaterial::ServiceAccount {
            json,
            access_token,
            expires_at,
            auth_header,
        } => AuthProfileMaterial::ServiceAccount {
            json,
            access_token: sanitize_text(access_token),
            expires_at: sanitize_text(expires_at),
            auth_header: sanitize_text(auth_header),
        },
    }
}

fn normalize_profile(mut profile: AuthProfileRecord) -> AuthProfileRecord {
    profile.id = sanitize_string(&profile.id);
    profile.name = sanitize_string(&profile.name);
    profile.user_id = sanitize_text(profile.user_id);
    profile.tenant_id = sanitize_text(profile.tenant_id);
    profile.provider = sanitize_text(profile.provider);
    profile.description = sanitize_text(profile.description);
    profile.created_at = sanitize_string(&profile.created_at);
    profile.updated_at = sanitize_string(&profile.updated_at);
    profile.last_validated_at = sanitize_text(profile.last_validated_at);
    profile.last_used_at = sanitize_text(profile.last_used_at);
    profile.last_error = sanitize_text(profile.last_error);
    profile.revoked_at = sanitize_text(profile.revoked_at);
    profile.material = sanitize_material(profile.material);
    profile.kind = profile.material.kind();
    profile
}

async fn load_profiles(storage: &Storage) -> Result<Vec<AuthProfileRecord>> {
    let profiles: Vec<AuthProfileRecord> = load_json(storage, AUTH_PROFILES_KEY).await?;
    Ok(profiles.into_iter().map(normalize_profile).collect())
}

async fn save_profiles(storage: &Storage, profiles: &[AuthProfileRecord]) -> Result<()> {
    save_json(storage, AUTH_PROFILES_KEY, profiles).await
}

async fn mutate_profile<F, T>(storage: &Storage, id: &str, mut mutate: F) -> Result<T>
where
    F: FnMut(&mut AuthProfileRecord) -> Result<T>,
{
    let mut profiles = load_profiles(storage).await?;
    let Some(index) = profiles.iter().position(|profile| profile.id == id) else {
        bail!("auth profile not found");
    };
    let result = mutate(&mut profiles[index])?;
    profiles[index] = normalize_profile(profiles[index].clone());
    save_profiles(storage, &profiles).await?;
    Ok(result)
}

async fn browser_profile_login_state(
    storage: &Storage,
    browser_profile_id: &str,
) -> Result<Option<crate::core::browser_profiles::BrowserLoginState>> {
    let payload = crate::core::browser_profiles::BrowserProfileControlPlane::list(storage).await?;
    Ok(payload
        .profiles
        .into_iter()
        .find(|profile| profile.id == browser_profile_id)
        .map(|profile| profile.login_state))
}

fn oauth_config(record: &OAuth2ProfileConfigRecord, redirect_uri: &str) -> OAuthConfig {
    OAuthConfig::from_input(OAuthConfigInput {
        client_id: record.client_id.clone(),
        client_secret: record.client_secret.clone(),
        auth_url: record.auth_url.clone(),
        token_url: record.token_url.clone(),
        redirect_uri: record
            .redirect_uri
            .clone()
            .unwrap_or_else(|| redirect_uri.to_string()),
        scopes: record.scopes.clone(),
        extra_auth_params: record.extra_auth_params.clone(),
        extra_token_params: record.extra_token_params.clone(),
        prompt: record.prompt.clone(),
        access_type: record.access_type.clone(),
    })
}

fn oauth_tokens_from_tokens(tokens: &crate::integrations::oauth::OAuthTokens) -> OAuth2TokenRecord {
    OAuth2TokenRecord {
        access_token: tokens.access_token().to_string(),
        refresh_token: tokens.refresh_token().map(|value| value.to_string()),
        expires_at: tokens.expires_at().and_then(timestamp_to_rfc3339),
        refresh_expires_at: None,
        token_type: Some(tokens.token_type().to_string()),
        scopes: tokens
            .scope()
            .map(|value| {
                value
                    .split_whitespace()
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        last_refreshed_at: Some(now_rfc3339()),
    }
}

fn oauth_http_ready(
    config: &OAuth2ProfileConfigRecord,
    tokens: Option<&OAuth2TokenRecord>,
) -> (AuthProfileStatus, bool, Option<String>) {
    if config.client_id.trim().is_empty()
        || config.client_secret.trim().is_empty()
        || config.auth_url.trim().is_empty()
        || config.token_url.trim().is_empty()
    {
        return (
            AuthProfileStatus::NeedsSecret,
            false,
            Some("OAuth client configuration is incomplete.".to_string()),
        );
    }
    let Some(tokens) = tokens else {
        return (
            AuthProfileStatus::NeedsConnect,
            false,
            Some("OAuth profile has not completed the browser callback flow yet.".to_string()),
        );
    };
    if !tokens.access_token.trim().is_empty() && !is_expired(tokens.expires_at.as_deref()) {
        return (AuthProfileStatus::Ready, true, None);
    }
    if tokens
        .refresh_token
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && !is_expired(tokens.refresh_expires_at.as_deref())
    {
        return (
            AuthProfileStatus::Ready,
            true,
            Some("Access token will refresh automatically on next use.".to_string()),
        );
    }
    (
        AuthProfileStatus::Expired,
        false,
        Some("OAuth access is expired and no refresh token is available.".to_string()),
    )
}

async fn profile_http_status(
    storage: &Storage,
    profile: &AuthProfileRecord,
) -> Result<(AuthProfileStatus, bool, Option<String>)> {
    if !profile.enabled {
        return Ok((
            AuthProfileStatus::Disabled,
            false,
            Some("Auth profile is disabled.".to_string()),
        ));
    }
    if profile.revoked_at.is_some() {
        return Ok((
            AuthProfileStatus::Revoked,
            false,
            Some("Auth profile has been revoked.".to_string()),
        ));
    }
    match &profile.material {
        AuthProfileMaterial::ApiKey {
            value,
            header: _,
            query_name: _,
        } => Ok(if value.trim().is_empty() {
            (
                AuthProfileStatus::NeedsSecret,
                false,
                Some("API key value is missing.".to_string()),
            )
        } else {
            (AuthProfileStatus::Ready, true, None)
        }),
        AuthProfileMaterial::Bearer { token, .. } => Ok(if token.trim().is_empty() {
            (
                AuthProfileStatus::NeedsSecret,
                false,
                Some("Bearer token is missing.".to_string()),
            )
        } else {
            (AuthProfileStatus::Ready, true, None)
        }),
        AuthProfileMaterial::Basic { username, password } => Ok(
            if username.trim().is_empty() || password.trim().is_empty() {
                (
                    AuthProfileStatus::NeedsSecret,
                    false,
                    Some("Basic auth username or password is missing.".to_string()),
                )
            } else {
                (AuthProfileStatus::Ready, true, None)
            },
        ),
        AuthProfileMaterial::Header { name, value } => {
            Ok(if name.trim().is_empty() || value.trim().is_empty() {
                (
                    AuthProfileStatus::NeedsSecret,
                    false,
                    Some("Custom header name or value is missing.".to_string()),
                )
            } else {
                (AuthProfileStatus::Ready, true, None)
            })
        }
        AuthProfileMaterial::Query { name, value } => {
            Ok(if name.trim().is_empty() || value.trim().is_empty() {
                (
                    AuthProfileStatus::NeedsSecret,
                    false,
                    Some("Query parameter name or value is missing.".to_string()),
                )
            } else {
                (AuthProfileStatus::Ready, true, None)
            })
        }
        AuthProfileMaterial::OAuth2 { config, tokens } => {
            Ok(oauth_http_ready(config, tokens.as_ref()))
        }
        AuthProfileMaterial::CookieSession { cookies, .. } => {
            let active = active_cookies(cookies);
            if active.is_empty() {
                Ok((
                    AuthProfileStatus::Expired,
                    false,
                    Some("No active session cookies are stored.".to_string()),
                ))
            } else {
                Ok((AuthProfileStatus::Ready, true, None))
            }
        }
        AuthProfileMaterial::BrowserSession {
            browser_profile_id,
            cookies,
            ..
        } => {
            let active = active_cookies(cookies);
            if !active.is_empty() {
                return Ok((AuthProfileStatus::Ready, true, None));
            }
            let login_state = browser_profile_login_state(storage, browser_profile_id).await?;
            match login_state {
                Some(crate::core::browser_profiles::BrowserLoginState::LoggedIn) => Ok((
                    AuthProfileStatus::NeedsConnect,
                    false,
                    Some(
                        "Browser profile is logged in, but no reusable session cookies were captured yet."
                            .to_string(),
                    ),
                )),
                Some(crate::core::browser_profiles::BrowserLoginState::NeedsMfa) => Ok((
                    AuthProfileStatus::NeedsConnect,
                    false,
                    Some("Browser profile still needs MFA before it can provide a session.".to_string()),
                )),
                Some(crate::core::browser_profiles::BrowserLoginState::Expired) => Ok((
                    AuthProfileStatus::Expired,
                    false,
                    Some("Browser session has expired.".to_string()),
                )),
                _ => Ok((
                    AuthProfileStatus::NeedsConnect,
                    false,
                    Some("Browser session is not connected.".to_string()),
                )),
            }
        }
        AuthProfileMaterial::ServiceAccount {
            access_token,
            expires_at,
            ..
        } => Ok(
            if access_token
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty() && !is_expired(expires_at.as_deref()))
            {
                (AuthProfileStatus::Ready, true, None)
            } else {
                (
                AuthProfileStatus::NeedsConnect,
                false,
                Some(
                    "Service account profile needs a valid access token or token broker before use."
                        .to_string(),
                ),
            )
            },
        ),
    }
}

fn material_view(material: &AuthProfileMaterial) -> AuthProfileMaterialView {
    match material {
        AuthProfileMaterial::ApiKey {
            value,
            header,
            query_name,
        } => AuthProfileMaterialView::ApiKey {
            configured: !value.trim().is_empty(),
            header: header.clone(),
            query_name: query_name.clone(),
        },
        AuthProfileMaterial::Bearer { token, header } => AuthProfileMaterialView::Bearer {
            configured: !token.trim().is_empty(),
            header: header.clone(),
        },
        AuthProfileMaterial::Basic { username, password } => AuthProfileMaterialView::Basic {
            username: (!username.trim().is_empty()).then(|| username.clone()),
            password_configured: !password.trim().is_empty(),
        },
        AuthProfileMaterial::Header { name, value } => AuthProfileMaterialView::Header {
            name: name.clone(),
            configured: !value.trim().is_empty(),
        },
        AuthProfileMaterial::Query { name, value } => AuthProfileMaterialView::Query {
            name: name.clone(),
            configured: !value.trim().is_empty(),
        },
        AuthProfileMaterial::OAuth2 { config, tokens } => AuthProfileMaterialView::OAuth2 {
            auth_url: config.auth_url.clone(),
            token_url: config.token_url.clone(),
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            connected: tokens
                .as_ref()
                .is_some_and(|token| !token.access_token.trim().is_empty()),
            has_refresh_token: tokens
                .as_ref()
                .and_then(|token| token.refresh_token.as_deref())
                .is_some_and(|value| !value.trim().is_empty()),
            expires_at: tokens.as_ref().and_then(|token| token.expires_at.clone()),
            refresh_expires_at: tokens
                .as_ref()
                .and_then(|token| token.refresh_expires_at.clone()),
            token_type: tokens.as_ref().and_then(|token| token.token_type.clone()),
        },
        AuthProfileMaterial::CookieSession {
            cookies, origin, ..
        } => {
            let active = active_cookies(cookies);
            AuthProfileMaterialView::CookieSession {
                cookie_count: active.len(),
                origin: origin.clone(),
                expires_at: latest_cookie_expiry(&active),
            }
        }
        AuthProfileMaterial::BrowserSession {
            browser_profile_id,
            cookies,
            login_url,
            last_captured_at,
            ..
        } => {
            let active = active_cookies(cookies);
            AuthProfileMaterialView::BrowserSession {
                browser_profile_id: browser_profile_id.clone(),
                cookie_count: active.len(),
                login_url: login_url.clone(),
                last_captured_at: last_captured_at.clone(),
                expires_at: latest_cookie_expiry(&active),
            }
        }
        AuthProfileMaterial::ServiceAccount {
            access_token,
            expires_at,
            ..
        } => AuthProfileMaterialView::ServiceAccount {
            configured: true,
            has_access_token: access_token
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            expires_at: expires_at.clone(),
        },
    }
}

fn overlay_header_value(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn overlay_query_value(query: &BTreeMap<String, String>, name: &str) -> Option<String> {
    query
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn basic_authorization_header(username: &str, password: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let raw = format!("{}:{}", username, password);
    format!("Basic {}", STANDARD.encode(raw))
}

fn resolve_env_export_source(
    profile: &AuthProfileRecord,
    overlay: &HttpAuthOverlay,
    source: &str,
) -> Result<String> {
    let source = source.trim();
    if source.is_empty() {
        bail!("Auth export source cannot be empty");
    }

    if let Some(name) = source.strip_prefix("header:") {
        return overlay_header_value(&overlay.headers, name.trim()).ok_or_else(|| {
            anyhow!(
                "Auth profile '{}' does not expose header '{}'.",
                profile.name,
                name.trim()
            )
        });
    }

    if let Some(name) = source.strip_prefix("query:") {
        return overlay_query_value(&overlay.query, name.trim()).ok_or_else(|| {
            anyhow!(
                "Auth profile '{}' does not expose query parameter '{}'.",
                profile.name,
                name.trim()
            )
        });
    }

    match source {
        "headers_json" => serde_json::to_string(&overlay.headers)
            .context("failed to encode auth profile headers for env export"),
        "query_json" => serde_json::to_string(&overlay.query)
            .context("failed to encode auth profile query for env export"),
        "cookie_header" | "cookies" => overlay.cookie_header().ok_or_else(|| {
            anyhow!(
                "Auth profile '{}' does not provide any reusable cookies.",
                profile.name
            )
        }),
        "authorization_header" => overlay_header_value(&overlay.headers, "Authorization")
            .or_else(|| {
                overlay
                    .basic
                    .as_ref()
                    .map(|(username, password)| basic_authorization_header(username, password))
            })
            .ok_or_else(|| {
                anyhow!(
                    "Auth profile '{}' does not expose an Authorization header.",
                    profile.name
                )
            }),
        "username" | "basic_username" => overlay
            .basic
            .as_ref()
            .map(|(username, _)| username.clone())
            .or_else(|| match &profile.material {
                AuthProfileMaterial::Basic { username, .. } => Some(username.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                anyhow!(
                    "Auth profile '{}' does not expose a username.",
                    profile.name
                )
            }),
        "password" | "basic_password" => overlay
            .basic
            .as_ref()
            .map(|(_, password)| password.clone())
            .or_else(|| match &profile.material {
                AuthProfileMaterial::Basic { password, .. } => Some(password.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                anyhow!(
                    "Auth profile '{}' does not expose a password.",
                    profile.name
                )
            }),
        "browser_profile_id" => overlay
            .browser_profile_id
            .clone()
            .or_else(|| match &profile.material {
                AuthProfileMaterial::BrowserSession {
                    browser_profile_id, ..
                } => Some(browser_profile_id.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                anyhow!(
                    "Auth profile '{}' is not backed by a browser profile.",
                    profile.name
                )
            }),
        "origin" => match &profile.material {
            AuthProfileMaterial::CookieSession { origin, .. } => origin.clone().ok_or_else(|| {
                anyhow!("Auth profile '{}' does not define an origin.", profile.name)
            }),
            _ => bail!("Auth profile '{}' does not expose an origin.", profile.name),
        },
        "json" | "service_account_json" => match &profile.material {
            AuthProfileMaterial::ServiceAccount { json, .. } => serde_json::to_string(json)
                .context("failed to encode service account JSON for env export"),
            _ => bail!(
                "Auth profile '{}' does not expose service account JSON.",
                profile.name
            ),
        },
        "cookies_json" => match &profile.material {
            AuthProfileMaterial::CookieSession { cookies, .. }
            | AuthProfileMaterial::BrowserSession { cookies, .. } => {
                serde_json::to_string(&active_cookies(cookies))
                    .context("failed to encode cookies for env export")
            }
            _ => bail!(
                "Auth profile '{}' does not expose reusable cookies.",
                profile.name
            ),
        },
        "client_id" => match &profile.material {
            AuthProfileMaterial::OAuth2 { config, .. } => Ok(config.client_id.clone()),
            _ => bail!(
                "Auth profile '{}' does not expose an OAuth client id.",
                profile.name
            ),
        },
        "client_secret" => match &profile.material {
            AuthProfileMaterial::OAuth2 { config, .. } => Ok(config.client_secret.clone()),
            _ => bail!(
                "Auth profile '{}' does not expose an OAuth client secret.",
                profile.name
            ),
        },
        "refresh_token" => match &profile.material {
            AuthProfileMaterial::OAuth2 { tokens, .. } => tokens
                .as_ref()
                .and_then(|tokens| tokens.refresh_token.clone())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!(
                        "Auth profile '{}' does not have a refresh token.",
                        profile.name
                    )
                }),
            _ => bail!(
                "Auth profile '{}' does not expose a refresh token.",
                profile.name
            ),
        },
        "token_type" => match &profile.material {
            AuthProfileMaterial::OAuth2 { tokens, .. } => tokens
                .as_ref()
                .and_then(|tokens| tokens.token_type.clone())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!(
                        "Auth profile '{}' does not have a token type.",
                        profile.name
                    )
                }),
            AuthProfileMaterial::Bearer { .. } | AuthProfileMaterial::ServiceAccount { .. } => {
                Ok("Bearer".to_string())
            }
            _ => bail!(
                "Auth profile '{}' does not expose a token type.",
                profile.name
            ),
        },
        "scope" | "scopes" => match &profile.material {
            AuthProfileMaterial::OAuth2 { config, tokens } => {
                let scopes = tokens
                    .as_ref()
                    .map(|tokens| sanitize_string_list(tokens.scopes.clone()))
                    .unwrap_or_else(|| sanitize_string_list(config.scopes.clone()));
                if scopes.is_empty() {
                    bail!(
                        "Auth profile '{}' does not define any scopes.",
                        profile.name
                    );
                }
                Ok(scopes.join(" "))
            }
            _ => bail!("Auth profile '{}' does not expose scopes.", profile.name),
        },
        "expires_at" => match &profile.material {
            AuthProfileMaterial::OAuth2 { tokens, .. } => tokens
                .as_ref()
                .and_then(|tokens| tokens.expires_at.clone())
                .or_else(|| overlay_header_value(&overlay.headers, "X-Token-Expires-At"))
                .ok_or_else(|| {
                    anyhow!("Auth profile '{}' does not expose an expiry.", profile.name)
                }),
            AuthProfileMaterial::ServiceAccount { expires_at, .. } => {
                expires_at.clone().ok_or_else(|| {
                    anyhow!("Auth profile '{}' does not expose an expiry.", profile.name)
                })
            }
            AuthProfileMaterial::CookieSession { cookies, .. }
            | AuthProfileMaterial::BrowserSession { cookies, .. } => {
                latest_cookie_expiry(&active_cookies(cookies)).ok_or_else(|| {
                    anyhow!(
                        "Auth profile '{}' does not expose a cookie expiry.",
                        profile.name
                    )
                })
            }
            _ => bail!("Auth profile '{}' does not expose an expiry.", profile.name),
        },
        "api_key" => match &profile.material {
            AuthProfileMaterial::ApiKey { value, .. } => Ok(value.clone()),
            _ => bail!("Auth profile '{}' is not an API key profile.", profile.name),
        },
        "token" | "access_token" => match &profile.material {
            AuthProfileMaterial::Bearer { token, .. } => Ok(token.clone()),
            AuthProfileMaterial::OAuth2 { tokens, .. } => tokens
                .as_ref()
                .map(|tokens| tokens.access_token.clone())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("Auth profile '{}' has no access token.", profile.name)),
            AuthProfileMaterial::ServiceAccount { access_token, .. } => access_token
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("Auth profile '{}' has no access token.", profile.name)),
            AuthProfileMaterial::Header { value, .. } => Ok(value.clone()),
            AuthProfileMaterial::Query { value, .. } => Ok(value.clone()),
            AuthProfileMaterial::ApiKey { value, .. } => Ok(value.clone()),
            _ => bail!(
                "Auth profile '{}' does not expose a token-like secret.",
                profile.name
            ),
        },
        "auto" => match &profile.material {
            AuthProfileMaterial::ApiKey { value, .. } => Ok(value.clone()),
            AuthProfileMaterial::Bearer { token, .. } => Ok(token.clone()),
            AuthProfileMaterial::Header { value, .. } => Ok(value.clone()),
            AuthProfileMaterial::Query { value, .. } => Ok(value.clone()),
            AuthProfileMaterial::OAuth2 { tokens, .. } => tokens
                .as_ref()
                .map(|tokens| tokens.access_token.clone())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("Auth profile '{}' has no access token.", profile.name)),
            AuthProfileMaterial::CookieSession { .. }
            | AuthProfileMaterial::BrowserSession { .. } => {
                overlay.cookie_header().ok_or_else(|| {
                    anyhow!(
                        "Auth profile '{}' does not provide any reusable cookies.",
                        profile.name
                    )
                })
            }
            AuthProfileMaterial::Basic { username, password } => {
                Ok(basic_authorization_header(username, password))
            }
            AuthProfileMaterial::ServiceAccount { access_token, .. } => access_token
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("Auth profile '{}' has no access token.", profile.name)),
        },
        other => bail!(
            "Auth profile '{}' does not support env export source '{}'.",
            profile.name,
            other
        ),
    }
}

async fn view_from_profile(
    storage: &Storage,
    profile: &AuthProfileRecord,
) -> Result<AuthProfileView> {
    let (status, ready, blocked_reason) = profile_http_status(storage, profile).await?;
    Ok(AuthProfileView {
        id: profile.id.clone(),
        name: profile.name.clone(),
        kind: profile.kind,
        enabled: profile.enabled,
        scope: profile.scope,
        user_id: profile.user_id.clone(),
        tenant_id: profile.tenant_id.clone(),
        provider: profile.provider.clone(),
        description: profile.description.clone(),
        created_at: profile.created_at.clone(),
        updated_at: profile.updated_at.clone(),
        last_validated_at: profile.last_validated_at.clone(),
        last_used_at: profile.last_used_at.clone(),
        last_error: profile.last_error.clone(),
        revoked_at: profile.revoked_at.clone(),
        status,
        ready,
        blocked_reason,
        metadata: profile.metadata.clone(),
        material: material_view(&profile.material),
    })
}

async fn build_summary(
    storage: &Storage,
    profiles: &[AuthProfileRecord],
) -> Result<AuthProfileSummary> {
    let mut summary = AuthProfileSummary {
        total: profiles.len(),
        ..AuthProfileSummary::default()
    };
    for profile in profiles {
        if matches!(profile.kind, AuthProfileKind::OAuth2) {
            summary.oauth += 1;
        }
        if matches!(
            profile.kind,
            AuthProfileKind::CookieSession | AuthProfileKind::BrowserSession
        ) {
            summary.cookie_or_browser += 1;
        }
        if profile.revoked_at.is_some() {
            summary.revoked += 1;
        }
        let (_, ready, _) = profile_http_status(storage, profile).await?;
        if ready {
            summary.ready += 1;
        } else {
            summary.needs_attention += 1;
        }
    }
    Ok(summary)
}

fn merge_upsert(
    resolved_id: String,
    existing: Option<AuthProfileRecord>,
    input: AuthProfileUpsert,
) -> Result<AuthProfileRecord> {
    let now = now_rfc3339();
    let existing = existing.map(normalize_profile);
    let material = input
        .material
        .or_else(|| existing.as_ref().map(|profile| profile.material.clone()))
        .ok_or_else(|| anyhow!("Auth profile material is required"))?;
    let kind = input
        .kind
        .or_else(|| existing.as_ref().map(|profile| profile.kind))
        .unwrap_or_else(|| material.kind());
    if kind != material.kind() {
        bail!("Auth profile kind does not match material type");
    }

    let created_at = existing
        .as_ref()
        .map(|profile| profile.created_at.clone())
        .unwrap_or_else(|| now.clone());

    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| existing.as_ref().map(|profile| profile.name.clone()))
        .unwrap_or_else(|| "Auth profile".to_string());

    Ok(normalize_profile(AuthProfileRecord {
        id: resolved_id,
        name,
        kind,
        enabled: input.enabled.unwrap_or_else(|| {
            existing
                .as_ref()
                .map(|profile| profile.enabled)
                .unwrap_or(true)
        }),
        scope: input.scope.unwrap_or_else(|| {
            existing
                .as_ref()
                .map(|profile| profile.scope)
                .unwrap_or_default()
        }),
        user_id: input.user_id.or_else(|| {
            existing
                .as_ref()
                .and_then(|profile| profile.user_id.clone())
        }),
        tenant_id: input.tenant_id.or_else(|| {
            existing
                .as_ref()
                .and_then(|profile| profile.tenant_id.clone())
        }),
        provider: input.provider.or_else(|| {
            existing
                .as_ref()
                .and_then(|profile| profile.provider.clone())
        }),
        description: input.description.or_else(|| {
            existing
                .as_ref()
                .and_then(|profile| profile.description.clone())
        }),
        created_at,
        updated_at: now,
        last_validated_at: existing
            .as_ref()
            .and_then(|profile| profile.last_validated_at.clone()),
        last_used_at: existing
            .as_ref()
            .and_then(|profile| profile.last_used_at.clone()),
        last_error: existing
            .as_ref()
            .and_then(|profile| profile.last_error.clone()),
        revoked_at: existing
            .as_ref()
            .and_then(|profile| profile.revoked_at.clone()),
        metadata: input.metadata.or_else(|| {
            existing
                .as_ref()
                .and_then(|profile| profile.metadata.clone())
        }),
        material,
    }))
}

pub struct AuthProfileControlPlane;

impl AuthProfileControlPlane {
    pub async fn list(storage: &Storage) -> Result<AuthProfileListResponse> {
        let mut profiles = load_profiles(storage).await?;
        profiles.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let mut views = Vec::with_capacity(profiles.len());
        for profile in &profiles {
            views.push(view_from_profile(storage, profile).await?);
        }
        Ok(AuthProfileListResponse {
            summary: build_summary(storage, &profiles).await?,
            profiles: views,
        })
    }

    pub async fn get(storage: &Storage, id: &str) -> Result<Option<AuthProfileView>> {
        let profiles = load_profiles(storage).await?;
        let Some(profile) = profiles.into_iter().find(|profile| profile.id == id) else {
            return Ok(None);
        };
        Ok(Some(view_from_profile(storage, &profile).await?))
    }

    pub async fn upsert(storage: &Storage, input: AuthProfileUpsert) -> Result<AuthProfileView> {
        let mut profiles = load_profiles(storage).await?;
        let requested_id = input
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing_index = profiles
            .iter()
            .position(|profile| profile.id == requested_id);
        let next = merge_upsert(
            requested_id.clone(),
            existing_index.and_then(|index| profiles.get(index).cloned()),
            input,
        )?;
        if let Some(index) = existing_index {
            profiles[index] = next.clone();
        } else {
            profiles.push(next.clone());
        }
        save_profiles(storage, &profiles).await?;
        view_from_profile(storage, &next).await
    }

    pub async fn delete(storage: &Storage, id: &str) -> Result<bool> {
        let mut profiles = load_profiles(storage).await?;
        let before = profiles.len();
        profiles.retain(|profile| profile.id != id);
        if profiles.len() == before {
            return Ok(false);
        }
        save_profiles(storage, &profiles).await?;
        Ok(true)
    }

    pub async fn revoke(
        storage: &Storage,
        id: &str,
        reason: Option<String>,
    ) -> Result<AuthProfileView> {
        mutate_profile(storage, id, |profile| {
            profile.revoked_at = Some(now_rfc3339());
            profile.last_error = sanitize_text(reason.clone())
                .or_else(|| Some("Auth profile was revoked.".to_string()));
            profile.updated_at = now_rfc3339();
            match &mut profile.material {
                AuthProfileMaterial::ApiKey { value, .. } => value.clear(),
                AuthProfileMaterial::Bearer { token, .. } => token.clear(),
                AuthProfileMaterial::Basic { password, .. } => password.clear(),
                AuthProfileMaterial::Header { value, .. } => value.clear(),
                AuthProfileMaterial::Query { value, .. } => value.clear(),
                AuthProfileMaterial::OAuth2 { tokens, .. } => *tokens = None,
                AuthProfileMaterial::CookieSession { cookies, .. } => cookies.clear(),
                AuthProfileMaterial::BrowserSession { cookies, .. } => cookies.clear(),
                AuthProfileMaterial::ServiceAccount { access_token, .. } => *access_token = None,
            }
            Ok(())
        })
        .await?;
        Self::get(storage, id)
            .await?
            .ok_or_else(|| anyhow!("auth profile not found after revoke"))
    }

    pub async fn capture_session_material(
        storage: &Storage,
        id: &str,
        cookies: Vec<AuthCookieRecord>,
        headers: BTreeMap<String, String>,
        origin: Option<String>,
        browser_profile_id: Option<String>,
        login_url: Option<String>,
    ) -> Result<AuthProfileView> {
        let captured_at = now_rfc3339();
        let cookies = cookies
            .into_iter()
            .filter_map(sanitize_cookie)
            .collect::<Vec<_>>();
        let headers = sanitize_headers(headers);
        mutate_profile(storage, id, |profile| {
            match &mut profile.material {
                AuthProfileMaterial::CookieSession {
                    cookies: stored_cookies,
                    headers: stored_headers,
                    origin: stored_origin,
                } => {
                    *stored_cookies = cookies.clone();
                    *stored_headers = headers.clone();
                    if let Some(origin) = sanitize_text(origin.clone()) {
                        *stored_origin = Some(origin);
                    }
                }
                AuthProfileMaterial::BrowserSession {
                    browser_profile_id: stored_browser_profile_id,
                    cookies: stored_cookies,
                    headers: stored_headers,
                    login_url: stored_login_url,
                    last_captured_at,
                } => {
                    *stored_cookies = cookies.clone();
                    *stored_headers = headers.clone();
                    if let Some(browser_profile_id) = sanitize_text(browser_profile_id.clone()) {
                        *stored_browser_profile_id = browser_profile_id;
                    }
                    if let Some(login_url) = sanitize_text(login_url.clone()) {
                        *stored_login_url = Some(login_url);
                    }
                    *last_captured_at = Some(captured_at.clone());
                }
                _ => bail!(
                    "Auth profile '{}' is not a cookie/session profile.",
                    profile.name
                ),
            }
            profile.revoked_at = None;
            profile.last_error = None;
            profile.last_validated_at = Some(captured_at.clone());
            profile.updated_at = captured_at.clone();
            Ok(())
        })
        .await?;
        Self::get(storage, id)
            .await?
            .ok_or_else(|| anyhow!("auth profile not found after session capture"))
    }

    pub async fn mark_used(storage: &Storage, id: &str) -> Result<()> {
        mutate_profile(storage, id, |profile| {
            profile.last_used_at = Some(now_rfc3339());
            profile.updated_at = now_rfc3339();
            Ok(())
        })
        .await
    }

    pub async fn oauth_authorization_url(
        storage: &Storage,
        id: &str,
        state_token: &str,
        code_challenge: Option<&str>,
        default_redirect_uri: Option<&str>,
    ) -> Result<String> {
        let profiles = load_profiles(storage).await?;
        let profile = profiles
            .into_iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| anyhow!("Auth profile not found"))?;
        if profile.revoked_at.is_some() {
            bail!("Auth profile has been revoked");
        }
        let AuthProfileMaterial::OAuth2 { config, .. } = &profile.material else {
            bail!("Auth profile '{}' is not an OAuth2 profile", profile.name);
        };
        let oauth = oauth_config(
            config,
            default_redirect_uri.unwrap_or(DEFAULT_OAUTH_REDIRECT_URI),
        );
        Ok(oauth.auth_url_with_pkce(state_token, code_challenge))
    }

    pub async fn complete_oauth_callback(
        storage: &Storage,
        id: &str,
        code: &str,
        pkce_verifier: Option<&str>,
        default_redirect_uri: Option<&str>,
    ) -> Result<AuthProfileView> {
        let profiles = load_profiles(storage).await?;
        let profile = profiles
            .into_iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| anyhow!("Auth profile not found"))?;
        let AuthProfileMaterial::OAuth2 { config, .. } = &profile.material else {
            bail!("Auth profile '{}' is not an OAuth2 profile", profile.name);
        };
        let oauth = oauth_config(
            config,
            default_redirect_uri.unwrap_or(DEFAULT_OAUTH_REDIRECT_URI),
        );
        let client = OAuthClient::new();
        let tokens = client
            .exchange_code_with_pkce(&oauth, code, pkce_verifier)
            .await?;
        let next_tokens = oauth_tokens_from_tokens(&tokens);
        mutate_profile(storage, id, |profile| {
            if let AuthProfileMaterial::OAuth2 { tokens, .. } = &mut profile.material {
                *tokens = Some(next_tokens.clone());
            } else {
                bail!("Auth profile is no longer an OAuth2 profile");
            }
            profile.last_error = None;
            profile.last_validated_at = Some(now_rfc3339());
            profile.revoked_at = None;
            profile.updated_at = now_rfc3339();
            Ok(())
        })
        .await?;
        Self::get(storage, id)
            .await?
            .ok_or_else(|| anyhow!("auth profile not found after OAuth callback"))
    }

    pub async fn resolve_http(storage: &Storage, id: &str) -> Result<AuthProfileResolution> {
        let mut profiles = load_profiles(storage).await?;
        let Some(index) = profiles.iter().position(|profile| profile.id == id) else {
            bail!("Auth profile not found");
        };
        let mut profile = profiles[index].clone();
        let (status, ready, reason) = profile_http_status(storage, &profile).await?;
        if !ready {
            bail!(
                "{}",
                reason.unwrap_or_else(|| format!("Auth profile is not ready: {:?}", status))
            );
        }

        let overlay = match &mut profile.material {
            AuthProfileMaterial::ApiKey {
                value,
                header,
                query_name,
            } => {
                let mut overlay = HttpAuthOverlay::default();
                if let Some(name) = query_name
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    overlay
                        .query
                        .insert(name.to_string(), value.trim().to_string());
                } else {
                    overlay.headers.insert(
                        header
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .unwrap_or("X-API-Key")
                            .to_string(),
                        value.trim().to_string(),
                    );
                }
                overlay
            }
            AuthProfileMaterial::Bearer { token, header } => {
                let mut overlay = HttpAuthOverlay::default();
                let header_name = header
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Authorization")
                    .to_string();
                let header_value = if header_name.eq_ignore_ascii_case("authorization") {
                    format!("Bearer {}", token.trim())
                } else {
                    token.trim().to_string()
                };
                overlay.headers.insert(header_name, header_value);
                overlay
            }
            AuthProfileMaterial::Basic { username, password } => HttpAuthOverlay {
                basic: Some((username.clone(), password.clone())),
                ..HttpAuthOverlay::default()
            },
            AuthProfileMaterial::Header { name, value } => {
                let mut overlay = HttpAuthOverlay::default();
                overlay.headers.insert(name.clone(), value.clone());
                overlay
            }
            AuthProfileMaterial::Query { name, value } => {
                let mut overlay = HttpAuthOverlay::default();
                overlay.query.insert(name.clone(), value.clone());
                overlay
            }
            AuthProfileMaterial::OAuth2 { .. } => {
                let (config, mut current) = match &profile.material {
                    AuthProfileMaterial::OAuth2 { config, tokens } => (
                        config.clone(),
                        tokens.clone().ok_or_else(|| {
                            anyhow!("OAuth2 profile has not completed connection yet")
                        })?,
                    ),
                    _ => bail!("Auth profile is no longer an OAuth2 profile"),
                };
                let access_expired = current.access_token.trim().is_empty()
                    || is_expired(current.expires_at.as_deref());
                if access_expired {
                    let refresh = current
                        .refresh_token
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| {
                            anyhow!(
                                "OAuth2 access token is expired and no refresh token is available"
                            )
                        })?;
                    let oauth = oauth_config(&config, DEFAULT_OAUTH_REDIRECT_URI);
                    let refreshed = OAuthClient::new().refresh_token(&oauth, refresh).await?;
                    current = oauth_tokens_from_tokens(&refreshed);
                    if let AuthProfileMaterial::OAuth2 { tokens, .. } = &mut profile.material {
                        *tokens = Some(current.clone());
                    } else {
                        bail!("Auth profile is no longer an OAuth2 profile");
                    }
                    profile.last_validated_at = Some(now_rfc3339());
                    profile.last_error = None;
                    profile.updated_at = now_rfc3339();
                    profiles[index] = normalize_profile(profile.clone());
                    save_profiles(storage, &profiles).await?;
                }

                let mut overlay = HttpAuthOverlay::default();
                let header_name = config
                    .auth_header
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Authorization")
                    .to_string();
                let token_type = current
                    .token_type
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Bearer");
                let header_value = if header_name.eq_ignore_ascii_case("authorization") {
                    format!("{} {}", token_type, current.access_token.trim())
                } else {
                    current.access_token.trim().to_string()
                };
                overlay.headers.insert(header_name, header_value);
                overlay
            }
            AuthProfileMaterial::CookieSession {
                cookies, headers, ..
            } => {
                let active = active_cookies(cookies);
                if active.is_empty() {
                    bail!("Cookie session does not have any active cookies");
                }
                let mut overlay = HttpAuthOverlay {
                    headers: headers.clone(),
                    ..HttpAuthOverlay::default()
                };
                let cookie_header = active
                    .iter()
                    .map(|cookie| format!("{}={}", cookie.name, cookie.value))
                    .collect::<Vec<_>>()
                    .join("; ");
                overlay.headers.insert("Cookie".to_string(), cookie_header);
                overlay
            }
            AuthProfileMaterial::BrowserSession {
                browser_profile_id,
                cookies,
                headers,
                ..
            } => {
                let active = active_cookies(cookies);
                if active.is_empty() {
                    bail!(
                        "Browser session has no captured reusable cookies yet. Capture cookies after browser sign-in before using this profile for HTTP actions."
                    );
                }
                let mut overlay = HttpAuthOverlay {
                    headers: headers.clone(),
                    browser_profile_id: Some(browser_profile_id.clone()),
                    ..HttpAuthOverlay::default()
                };
                let cookie_header = active
                    .iter()
                    .map(|cookie| format!("{}={}", cookie.name, cookie.value))
                    .collect::<Vec<_>>()
                    .join("; ");
                overlay.headers.insert("Cookie".to_string(), cookie_header);
                overlay
            }
            AuthProfileMaterial::ServiceAccount {
                access_token,
                expires_at,
                auth_header,
                ..
            } => {
                let token = access_token
                    .as_deref()
                    .filter(|value| !value.trim().is_empty() && !is_expired(expires_at.as_deref()))
                    .ok_or_else(|| {
                        anyhow!("Service account profile does not have a valid access token")
                    })?;
                let mut overlay = HttpAuthOverlay::default();
                let header_name = auth_header
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Authorization")
                    .to_string();
                let header_value = if header_name.eq_ignore_ascii_case("authorization") {
                    format!("Bearer {}", token.trim())
                } else {
                    token.trim().to_string()
                };
                overlay.headers.insert(header_name, header_value);
                overlay
            }
        };

        mutate_profile(storage, id, |profile| {
            profile.last_validated_at = Some(now_rfc3339());
            profile.last_error = None;
            profile.updated_at = now_rfc3339();
            Ok(())
        })
        .await?;

        Ok(AuthProfileResolution { overlay })
    }

    pub async fn resolve_env_exports(
        storage: &Storage,
        id: &str,
        requested: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>> {
        if requested.is_empty() {
            return Ok(BTreeMap::new());
        }

        let resolved = Self::resolve_http(storage, id).await?;
        let profiles = load_profiles(storage).await?;
        let profile = profiles
            .into_iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| anyhow!("Auth profile not found"))?;

        let mut exports = BTreeMap::new();
        for (env_name, source) in requested {
            let env_name = env_name.trim();
            if env_name.is_empty() {
                bail!("Auth export env names cannot be empty");
            }
            let value = resolve_env_export_source(&profile, &resolved.overlay, source)?;
            exports.insert(env_name.to_string(), value);
        }
        Ok(exports)
    }
}
