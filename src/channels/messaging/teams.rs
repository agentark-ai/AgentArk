//! Microsoft Teams transport foundation.
//!
//! This module keeps the transport logic self-contained: serializable
//! configuration, Bot Framework / Graph-friendly outbound payload builders, and
//! inbound activity handling that persists reply destinations before handing
//! the message to the agent core.
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use once_cell::sync::Lazy;
use ring::signature::{self, RsaPublicKeyComponents};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use url::Url;

use crate::core::connectivity::sender_verification::{
    self, SenderChannel, SenderIdentity, SenderTrustDecision,
};
use crate::core::Agent;
use crate::storage::Storage;

type SharedAgent = Arc<RwLock<Agent>>;

const CONFIG_STORAGE_KEY: &str = "channels:teams:config";
const LAST_DESTINATION_STORAGE_KEY: &str = "channels:teams:last_destination";
const TEAMS_DESTINATIONS_KEY_PREFIX: &str = "teams:reply_destinations:v1:";
const TEAMS_DEFAULT_GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";
const TEAMS_DEFAULT_TIMEOUT_SECS: u64 = 15;
const TEAMS_BOT_FRAMEWORK_OPENID_CONFIGURATION_URL: &str =
    "https://login.botframework.com/v1/.well-known/openidconfiguration";
const TEAMS_BOT_FRAMEWORK_AUTH_CACHE_TTL: Duration = Duration::from_secs(60 * 60 * 24);
const TEAMS_BOT_FRAMEWORK_CLOCK_SKEW_SECS: i64 = 300;
const TEAMS_RECENT_ACTIVITY_IDS_STORAGE_KEY: &str = "channels:teams:recent_activity_ids";
const MAX_RECENT_ACTIVITY_IDS: usize = 64;
const RECENT_ACTIVITY_ID_WINDOW_SECS: u64 = 60 * 60 * 24;

static TEAMS_BOT_FRAMEWORK_AUTH_CACHE: Lazy<RwLock<Option<BotFrameworkAuthCache>>> =
    Lazy::new(|| RwLock::new(None));
static TEAMS_ACTIVITY_DEDUP_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TeamsRecentActivityState {
    #[serde(default)]
    recent: Vec<TeamsRecentActivityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TeamsRecentActivityEntry {
    activity_id: String,
    seen_at: u64,
}

fn teams_sender_verification_notice(sender_label: &str) -> String {
    format!(
        "Sender approval required before {} can respond here.\n\nSender: {}\nOpen `Settings -> Connected Systems -> Sender Verification` to approve this sender.",
        crate::branding::PRODUCT_NAME,
        sender_label.trim()
    )
}

fn teams_sender_verification_notification(
    sender_label: &str,
    tenant_id: Option<&str>,
    text: &str,
) -> String {
    let scope = tenant_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("\nTenant: {}", value))
        .unwrap_or_default();
    let preview = text.trim();
    let preview = if preview.is_empty() {
        String::new()
    } else {
        format!(
            "\nMessage: {}",
            preview.chars().take(180).collect::<String>()
        )
    };
    format!(
        "A new Teams sender needs approval before {} will act.\nSender: {}{}{}\nApprove it in Settings -> Connected Systems -> Sender Verification.",
        crate::branding::PRODUCT_NAME,
        sender_label.trim(),
        scope,
        preview
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TeamsDeliveryMode {
    #[default]
    Auto,
    BotFramework,
    Graph,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamsTransportConfig {
    pub service_url: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_base_url: Option<String>,
    #[serde(default)]
    pub delivery_mode: TeamsDeliveryMode,
    #[serde(default)]
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

impl Default for TeamsTransportConfig {
    fn default() -> Self {
        Self {
            service_url: String::new(),
            access_token: String::new(),
            bot_app_id: None,
            bot_name: None,
            tenant_id: None,
            team_id: None,
            channel_id: None,
            chat_id: None,
            graph_base_url: Some(TEAMS_DEFAULT_GRAPH_BASE_URL.to_string()),
            delivery_mode: TeamsDeliveryMode::Auto,
            timeout_secs: TEAMS_DEFAULT_TIMEOUT_SECS,
            user_agent: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsIdentity {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aad_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsConversation {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_type: Option<String>,
    #[serde(default)]
    pub is_group: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsChannelDataTeam {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsChannelDataChannel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsChannelDataTenant {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsChannelData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamsChannelDataTeam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<TeamsChannelDataChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<TeamsChannelDataTenant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsActivity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub activity_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<TeamsIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient: Option<TeamsIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<TeamsConversation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_data: Option<TeamsChannelData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsReplyDestination {
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reply_to_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsReplyDestinationState {
    #[serde(default)]
    pub conversations: BTreeMap<String, TeamsReplyDestination>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsOutboundMessage {
    pub conversation_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsOutboundResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamsInboundSummary {
    pub activity_id: Option<String>,
    pub conversation_id: String,
    pub reply_destination_key: String,
    pub processed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_preview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TeamsVerifiedInbound {
    pub service_url: String,
}

#[derive(Debug, Clone)]
struct BotFrameworkAuthCache {
    fetched_at: Instant,
    issuer: String,
    signing_algs: Vec<String>,
    keys: Vec<BotFrameworkSigningKey>,
}

#[derive(Debug, Clone)]
struct BotFrameworkSigningKey {
    kid: Option<String>,
    x5t: Option<String>,
    endorsements: Vec<String>,
    modulus: Vec<u8>,
    exponent: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
struct BotFrameworkOpenIdConfiguration {
    issuer: String,
    jwks_uri: String,
    #[serde(default)]
    id_token_signing_alg_values_supported: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct BotFrameworkJwkSet {
    #[serde(default)]
    keys: Vec<BotFrameworkJwk>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct BotFrameworkJwk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x5t: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    n: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    e: Option<String>,
    #[serde(default)]
    endorsements: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtHeader {
    #[serde(default)]
    alg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x5t: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum JwtAudience {
    One(String),
    Many(Vec<String>),
}

impl JwtAudience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value.trim() == expected,
            Self::Many(values) => values.iter().any(|value| value.trim() == expected),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct BotFrameworkClaims {
    iss: String,
    aud: JwtAudience,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nbf: Option<i64>,
    #[serde(
        default,
        rename = "serviceurl",
        alias = "serviceUrl",
        skip_serializing_if = "Option::is_none"
    )]
    service_url: Option<String>,
    #[serde(
        default,
        rename = "channelid",
        alias = "channelId",
        skip_serializing_if = "Option::is_none"
    )]
    channel_id: Option<String>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn now_unix_seconds() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock is before UNIX_EPOCH: {}", error))?
        .as_secs() as i64)
}

fn prune_recent_activity_state(state: &mut TeamsRecentActivityState, now: u64) {
    let min_seen_at = now.saturating_sub(RECENT_ACTIVITY_ID_WINDOW_SECS);
    state.recent.retain(|entry| entry.seen_at >= min_seen_at);
    if state.recent.len() > MAX_RECENT_ACTIVITY_IDS {
        let excess = state.recent.len() - MAX_RECENT_ACTIVITY_IDS;
        state.recent.drain(0..excess);
    }
}

async fn load_recent_activity_state(storage: &Storage) -> Result<TeamsRecentActivityState> {
    if let Ok(Some(raw)) = storage.get(TEAMS_RECENT_ACTIVITY_IDS_STORAGE_KEY).await {
        if let Ok(state) = serde_json::from_slice::<TeamsRecentActivityState>(&raw) {
            return Ok(state);
        }
    }
    Ok(TeamsRecentActivityState::default())
}

async fn persist_recent_activity_state(
    storage: &Storage,
    state: &TeamsRecentActivityState,
) -> Result<()> {
    let raw = serde_json::to_vec(state)?;
    storage
        .set(TEAMS_RECENT_ACTIVITY_IDS_STORAGE_KEY, &raw)
        .await?;
    Ok(())
}

async fn record_teams_activity_id(storage: &Storage, activity_id: &str) -> Result<bool> {
    let activity_id = activity_id.trim();
    if activity_id.is_empty() {
        return Ok(false);
    }
    let _guard = TEAMS_ACTIVITY_DEDUP_LOCK.lock().await;
    let now = u64::try_from(now_unix_seconds()?)
        .map_err(|_| anyhow!("system clock returned a negative timestamp"))?;
    let mut state = load_recent_activity_state(storage).await?;
    prune_recent_activity_state(&mut state, now);
    if state
        .recent
        .iter()
        .any(|entry| entry.activity_id == activity_id)
    {
        return Ok(true);
    }
    state.recent.push(TeamsRecentActivityEntry {
        activity_id: activity_id.to_string(),
        seen_at: now,
    });
    prune_recent_activity_state(&mut state, now);
    persist_recent_activity_state(storage, &state).await?;
    Ok(false)
}

fn sanitize_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_service_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("service_url is required");
    }

    let parsed = Url::parse(trimmed).context("invalid Teams service_url")?;
    if parsed.scheme() != "https" {
        bail!("Teams service_url must use https");
    }
    if parsed.host_str().is_none() {
        bail!("Teams service_url host is required");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("Teams service_url must not contain embedded credentials");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        bail!("Teams service_url must not contain query or fragment data");
    }

    let mut normalized = parsed.to_string();
    while normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}

fn activity_channel_id(activity: &TeamsActivity) -> Result<String> {
    sanitize_text(activity.channel_id.clone())
        .ok_or_else(|| anyhow!("Teams activity missing channel_id"))
}

fn parse_bearer_token(authorization: Option<&str>) -> Result<&str> {
    let header = authorization
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Teams authorization header is required"))?;
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Teams authorization header must use Bearer auth"))?;
    Ok(token)
}

fn decode_jwt_json<T>(segment: &str, label: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let decoded = URL_SAFE_NO_PAD
        .decode(segment)
        .with_context(|| format!("failed to decode Teams JWT {}", label))?;
    serde_json::from_slice::<T>(&decoded)
        .with_context(|| format!("failed to parse Teams JWT {}", label))
}

fn jwk_to_signing_key(jwk: &BotFrameworkJwk) -> Result<BotFrameworkSigningKey> {
    let n = jwk
        .n
        .as_deref()
        .ok_or_else(|| anyhow!("Bot Framework JWK missing modulus"))?;
    let e = jwk
        .e
        .as_deref()
        .ok_or_else(|| anyhow!("Bot Framework JWK missing exponent"))?;
    let n = URL_SAFE_NO_PAD
        .decode(n)
        .context("failed to decode Bot Framework JWK modulus")?;
    let e = URL_SAFE_NO_PAD
        .decode(e)
        .context("failed to decode Bot Framework JWK exponent")?;
    Ok(BotFrameworkSigningKey {
        kid: jwk.kid.clone(),
        x5t: jwk.x5t.clone(),
        endorsements: jwk.endorsements.clone(),
        modulus: n,
        exponent: e,
    })
}

fn select_signing_key<'a>(
    cache: &'a BotFrameworkAuthCache,
    header: &JwtHeader,
    channel_id: &str,
) -> Result<&'a BotFrameworkSigningKey> {
    let key = cache
        .keys
        .iter()
        .find(|key| {
            header
                .kid
                .as_deref()
                .is_some_and(|kid| key.kid.as_deref().is_some_and(|value| value == kid))
                || header
                    .x5t
                    .as_deref()
                    .is_some_and(|x5t| key.x5t.as_deref().is_some_and(|value| value == x5t))
        })
        .ok_or_else(|| anyhow!("Teams authorization key was not present in Bot Framework JWKS"))?;
    if !key
        .endorsements
        .iter()
        .any(|endorsement| endorsement.eq_ignore_ascii_case(channel_id))
    {
        bail!(
            "Teams authorization key is not endorsed for channel {}",
            channel_id
        );
    }
    Ok(key)
}

fn verify_signature(
    key: &BotFrameworkSigningKey,
    signed_input: &str,
    signature_segment: &str,
) -> Result<()> {
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(signature_segment)
        .context("failed to decode Teams authorization signature")?;
    let public_key = RsaPublicKeyComponents {
        n: key.modulus.as_slice(),
        e: key.exponent.as_slice(),
    };
    public_key
        .verify(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            signed_input.as_bytes(),
            signature_bytes.as_slice(),
        )
        .map_err(|_| anyhow!("Teams authorization signature verification failed"))
}

async fn auth_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build Teams auth HTTP client")
}

async fn fetch_bot_framework_auth_cache() -> Result<BotFrameworkAuthCache> {
    let client = auth_http_client().await?;
    let metadata = client
        .get(TEAMS_BOT_FRAMEWORK_OPENID_CONFIGURATION_URL)
        .send()
        .await
        .context("failed to fetch Bot Framework OpenID configuration")?
        .error_for_status()
        .context("Bot Framework OpenID configuration request failed")?
        .json::<BotFrameworkOpenIdConfiguration>()
        .await
        .context("failed to decode Bot Framework OpenID configuration")?;
    let jwks = client
        .get(metadata.jwks_uri.clone())
        .send()
        .await
        .context("failed to fetch Bot Framework JWKS")?
        .error_for_status()
        .context("Bot Framework JWKS request failed")?
        .json::<BotFrameworkJwkSet>()
        .await
        .context("failed to decode Bot Framework JWKS")?;
    let mut keys = Vec::new();
    for jwk in &jwks.keys {
        if let Ok(key) = jwk_to_signing_key(jwk) {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        bail!("Bot Framework JWKS did not contain any usable signing keys");
    }
    Ok(BotFrameworkAuthCache {
        fetched_at: Instant::now(),
        issuer: metadata.issuer,
        signing_algs: metadata.id_token_signing_alg_values_supported,
        keys,
    })
}

async fn load_bot_framework_auth_cache() -> Result<BotFrameworkAuthCache> {
    {
        let guard = TEAMS_BOT_FRAMEWORK_AUTH_CACHE.read().await;
        if let Some(cache) = guard.as_ref() {
            if cache.fetched_at.elapsed() < TEAMS_BOT_FRAMEWORK_AUTH_CACHE_TTL {
                return Ok(cache.clone());
            }
        }
    }

    let fetched = fetch_bot_framework_auth_cache().await;
    let mut guard = TEAMS_BOT_FRAMEWORK_AUTH_CACHE.write().await;
    match fetched {
        Ok(cache) => {
            *guard = Some(cache.clone());
            Ok(cache)
        }
        Err(error) => {
            if let Some(cache) = guard.as_ref() {
                tracing::warn!(
                    "Teams auth refresh failed; falling back to cached Bot Framework keys: {}",
                    error
                );
                Ok(cache.clone())
            } else {
                Err(error)
            }
        }
    }
}

pub async fn verify_inbound_activity_request(
    config: &TeamsTransportConfig,
    authorization: Option<&str>,
    activity: &TeamsActivity,
) -> Result<TeamsVerifiedInbound> {
    validate_config(config)?;
    let bot_app_id = config
        .bot_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("bot_app_id is required for Teams ingress auth"))?;
    let token = parse_bearer_token(authorization)?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        bail!("Teams authorization token is malformed");
    }

    let header = decode_jwt_json::<JwtHeader>(parts[0], "header")?;
    if header.alg.trim() != "RS256" {
        bail!("Teams authorization algorithm must be RS256");
    }

    let claims = decode_jwt_json::<BotFrameworkClaims>(parts[1], "claims")?;
    let cache = load_bot_framework_auth_cache().await?;
    if !cache.signing_algs.is_empty()
        && !cache
            .signing_algs
            .iter()
            .any(|alg| alg.eq_ignore_ascii_case("RS256"))
    {
        bail!("Bot Framework metadata does not advertise RS256 signing");
    }
    if claims.iss.trim() != cache.issuer.trim() {
        bail!("Teams authorization issuer mismatch");
    }
    if !claims.aud.contains(bot_app_id) {
        bail!("Teams authorization audience mismatch");
    }

    let now = now_unix_seconds()?;
    let exp = claims
        .exp
        .ok_or_else(|| anyhow!("Teams authorization token missing exp claim"))?;
    let nbf = claims
        .nbf
        .ok_or_else(|| anyhow!("Teams authorization token missing nbf claim"))?;
    if exp < now - TEAMS_BOT_FRAMEWORK_CLOCK_SKEW_SECS {
        bail!("Teams authorization token has expired");
    }
    if nbf > now + TEAMS_BOT_FRAMEWORK_CLOCK_SKEW_SECS {
        bail!("Teams authorization token is not yet valid");
    }

    let activity_service_url = normalize_service_url(
        activity
            .service_url
            .as_deref()
            .ok_or_else(|| anyhow!("Teams activity missing service_url"))?,
    )?;
    let claim_service_url = normalize_service_url(
        claims
            .service_url
            .as_deref()
            .ok_or_else(|| anyhow!("Teams authorization token missing serviceUrl claim"))?,
    )?;
    if activity_service_url != claim_service_url {
        bail!("Teams activity service_url does not match the authorization token");
    }

    let channel_id = activity_channel_id(activity)?;
    if let Some(claim_channel_id) = claims.channel_id.as_deref() {
        let claim_channel_id = claim_channel_id.trim();
        if !claim_channel_id.is_empty() && !claim_channel_id.eq_ignore_ascii_case(&channel_id) {
            bail!("Teams authorization token channelId does not match the activity");
        }
    }

    let key = select_signing_key(&cache, &header, &channel_id)?;
    verify_signature(key, &format!("{}.{}", parts[0], parts[1]), parts[2])?;

    Ok(TeamsVerifiedInbound {
        service_url: activity_service_url,
    })
}

fn conversation_id(activity: &TeamsActivity) -> Option<String> {
    activity
        .conversation
        .as_ref()
        .map(|conversation| conversation.id.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| sanitize_text(activity.reply_to_id.clone()))
}

fn destination_key(conversation_id: &str) -> String {
    format!("{}{}", TEAMS_DESTINATIONS_KEY_PREFIX, conversation_id)
}

fn graph_base_url(config: &TeamsTransportConfig) -> String {
    config
        .graph_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(TEAMS_DEFAULT_GRAPH_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

fn build_client(config: &TeamsTransportConfig) -> Result<reqwest::Client> {
    let timeout_secs = if config.timeout_secs == 0 {
        TEAMS_DEFAULT_TIMEOUT_SECS
    } else {
        config.timeout_secs
    };
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(timeout_secs));
    if let Some(user_agent) = config
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder = builder.user_agent(user_agent.to_string());
    }
    builder.build().context("failed to build Teams HTTP client")
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode Teams payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode Teams payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

async fn load_state(storage: &Storage, key: &str) -> Result<TeamsReplyDestinationState> {
    load_json(storage, key).await
}

async fn save_state(
    storage: &Storage,
    key: &str,
    state: &TeamsReplyDestinationState,
) -> Result<()> {
    save_json(storage, key, state).await
}

pub async fn load_config_from_storage(storage: &Storage) -> Result<Option<TeamsTransportConfig>> {
    if let Ok(Some(raw)) = storage.get(CONFIG_STORAGE_KEY).await {
        if let Ok(config) = serde_json::from_slice::<TeamsTransportConfig>(&raw) {
            return Ok(Some(config));
        }
    }

    let service_url = std::env::var("TEAMS_SERVICE_URL").unwrap_or_default();
    let access_token = std::env::var("TEAMS_ACCESS_TOKEN").unwrap_or_default();
    let bot_app_id = std::env::var("TEAMS_BOT_APP_ID").ok();
    let bot_name = std::env::var("TEAMS_BOT_NAME").ok();
    let tenant_id = std::env::var("TEAMS_TENANT_ID").ok();
    let team_id = std::env::var("TEAMS_TEAM_ID").ok();
    let channel_id = std::env::var("TEAMS_CHANNEL_ID").ok();
    let chat_id = std::env::var("TEAMS_CHAT_ID").ok();
    let graph_base_url = std::env::var("TEAMS_GRAPH_BASE_URL").ok();
    let delivery_mode = std::env::var("TEAMS_DELIVERY_MODE")
        .ok()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "bot_framework" | "bot" => TeamsDeliveryMode::BotFramework,
            "graph" => TeamsDeliveryMode::Graph,
            _ => TeamsDeliveryMode::Auto,
        })
        .unwrap_or_default();
    let timeout_secs = std::env::var("TEAMS_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(TEAMS_DEFAULT_TIMEOUT_SECS);
    let user_agent = std::env::var("TEAMS_USER_AGENT").ok();

    if service_url.trim().is_empty() && access_token.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(TeamsTransportConfig {
        service_url,
        access_token,
        bot_app_id,
        bot_name,
        tenant_id,
        team_id,
        channel_id,
        chat_id,
        graph_base_url,
        delivery_mode,
        timeout_secs,
        user_agent,
    }))
}

async fn load_config(agent: &Agent) -> Result<Option<TeamsTransportConfig>> {
    if let Some(config) = agent.config.teams.clone() {
        return Ok(Some(config));
    }
    load_config_from_storage(&agent.storage).await
}

fn validate_config(config: &TeamsTransportConfig) -> Result<()> {
    if config.service_url.trim().is_empty() {
        bail!("service_url is required");
    }
    if config.access_token.trim().is_empty() {
        bail!("access_token is required");
    }
    if config
        .bot_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        bail!("bot_app_id is required");
    }
    normalize_service_url(&config.service_url)?;
    Ok(())
}

fn build_bot_framework_payload(
    config: &TeamsTransportConfig,
    destination: &TeamsReplyDestination,
    message: &TeamsOutboundMessage,
) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "text": message.text,
        "from": {
            "id": config.bot_app_id.as_deref().unwrap_or("agentark"),
            "name": config
                .bot_name
                .as_deref()
                .unwrap_or(crate::branding::PRODUCT_NAME)
        },
        "conversation": {
            "id": destination.conversation_id,
            "isGroup": destination.team_id.is_some() || destination.channel_id.is_some()
        },
        "recipient": destination.user_id.as_deref().map(|id| serde_json::json!({ "id": id })).unwrap_or(serde_json::Value::Null),
        "channelData": {
            "tenant": destination.tenant_id.as_deref().map(|id| serde_json::json!({ "id": id })).unwrap_or(serde_json::Value::Null),
            "team": destination.team_id.as_deref().map(|id| serde_json::json!({ "id": id })).unwrap_or(serde_json::Value::Null),
            "channel": destination.channel_id.as_deref().map(|id| serde_json::json!({ "id": id })).unwrap_or(serde_json::Value::Null)
        }
    })
}

fn build_graph_payload(message: &TeamsOutboundMessage) -> serde_json::Value {
    serde_json::json!({
        "body": {
            "contentType": message.content_type.as_deref().unwrap_or("text"),
            "content": message.text
        },
        "replyToId": message.reply_to_id,
    })
}

fn graph_endpoint_for_destination(
    config: &TeamsTransportConfig,
    destination: &TeamsReplyDestination,
) -> Option<String> {
    let base = graph_base_url(config);
    if let (Some(team_id), Some(channel_id)) = (
        destination.team_id.as_deref(),
        destination.channel_id.as_deref(),
    ) {
        return Some(format!(
            "{}/teams/{}/channels/{}/messages",
            base,
            urlencoding::encode(team_id),
            urlencoding::encode(channel_id)
        ));
    }
    if let Some(chat_id) = destination
        .chat_id
        .as_deref()
        .or(Some(destination.conversation_id.as_str()))
    {
        return Some(format!(
            "{}/chats/{}/messages",
            base,
            urlencoding::encode(chat_id)
        ));
    }
    None
}

async fn persist_destination(
    storage: &Storage,
    config: &TeamsTransportConfig,
    destination: TeamsReplyDestination,
) -> Result<()> {
    let key = destination_key(&destination.conversation_id);
    let mut state = load_state(storage, &key).await?;
    state
        .conversations
        .insert(destination.conversation_id.clone(), destination.clone());
    if state.conversations.len() > 128 {
        let mut values: Vec<_> = state.conversations.values().cloned().collect();
        values.sort_by(|left, right| left.last_seen_at.cmp(&right.last_seen_at));
        values.reverse();
        values.truncate(128);
        state.conversations = values
            .into_iter()
            .map(|destination| (destination.conversation_id.clone(), destination))
            .collect();
    }
    save_state(storage, &key, &state).await?;
    let raw = serde_json::to_vec(&destination)?;
    storage
        .set(LAST_DESTINATION_STORAGE_KEY, &raw)
        .await
        .context("failed to store Teams last destination")?;
    let _ = config;
    Ok(())
}

fn load_default_destination(
    config: &TeamsTransportConfig,
) -> Result<Option<TeamsReplyDestination>> {
    let conversation_id = if let Some(chat_id) = config
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(chat_id.to_string())
    } else if let (Some(team_id), Some(channel_id)) = (
        config
            .team_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        config
            .channel_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        Some(format!("{}:{}", team_id, channel_id))
    } else {
        None
    };

    let Some(conversation_id) = conversation_id else {
        return Ok(None);
    };

    Ok(Some(TeamsReplyDestination {
        conversation_id: conversation_id.clone(),
        service_url: Some(normalize_service_url(&config.service_url)?),
        chat_id: config.chat_id.clone(),
        team_id: config.team_id.clone(),
        channel_id: config.channel_id.clone(),
        thread_id: None,
        tenant_id: config.tenant_id.clone(),
        user_id: None,
        user_name: None,
        last_activity_id: None,
        last_reply_to_id: None,
        last_seen_at: Some(now_rfc3339()),
    }))
}

pub fn default_destination_for_config(
    config: &TeamsTransportConfig,
) -> Result<Option<TeamsReplyDestination>> {
    load_default_destination(config)
}

pub fn validate_transport_config(config: &TeamsTransportConfig) -> Result<()> {
    validate_config(config)
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = load_config(agent)
        .await?
        .ok_or_else(|| anyhow!("Teams is not configured"))?;
    let destination = load_default_destination(&config)?
        .ok_or_else(|| anyhow!("Teams has no delivery destination yet"))?;
    for chunk in super::outbound_split::split_for_provider_safe_channel("teams", text) {
        send_message_to_destination(
            &config,
            &destination,
            &TeamsOutboundMessage {
                conversation_id: destination.conversation_id.clone(),
                text: chunk,
                reply_to_id: destination.last_reply_to_id.clone(),
                service_url: destination.service_url.clone(),
                team_id: destination.team_id.clone(),
                channel_id: destination.channel_id.clone(),
                chat_id: destination.chat_id.clone(),
                content_type: None,
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn send_message_to_destination(
    config: &TeamsTransportConfig,
    destination: &TeamsReplyDestination,
    message: &TeamsOutboundMessage,
) -> Result<TeamsOutboundResponse> {
    validate_config(config)?;
    let text = message.text.trim();
    if text.is_empty() {
        bail!("message text is required");
    }

    let client = build_client(config)?;
    let mode = match config.delivery_mode {
        TeamsDeliveryMode::Auto => {
            if graph_endpoint_for_destination(config, destination).is_some() {
                TeamsDeliveryMode::Graph
            } else {
                TeamsDeliveryMode::BotFramework
            }
        }
        other => other,
    };

    match mode {
        TeamsDeliveryMode::Graph => {
            let endpoint = graph_endpoint_for_destination(config, destination)
                .ok_or_else(|| anyhow!("Graph endpoint could not be derived from destination"))?;
            let response = super::outbound_rate_limit::send_with_bounded_retries(
                "teams",
                "graph_message",
                client
                    .post(endpoint.clone())
                    .bearer_auth(config.access_token.trim())
                    .json(&build_graph_payload(message)),
            )
            .await
            .context("failed to send Teams Graph message")?;
            let status = response.status();
            let payload: serde_json::Value =
                response.json().await.unwrap_or(serde_json::Value::Null);
            if !status.is_success() {
                let err = payload
                    .get("error")
                    .and_then(|value| value.get("message"))
                    .and_then(|value| value.as_str())
                    .or_else(|| payload.get("error").and_then(|value| value.as_str()))
                    .unwrap_or("Teams Graph send failed");
                return Err(anyhow!(err.to_string()));
            }
            Ok(TeamsOutboundResponse {
                activity_id: payload
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                conversation_id: Some(destination.conversation_id.clone()),
                service_url: Some(normalize_service_url(
                    destination
                        .service_url
                        .as_deref()
                        .unwrap_or(config.service_url.as_str()),
                )?),
                endpoint: Some(endpoint),
            })
        }
        TeamsDeliveryMode::BotFramework | TeamsDeliveryMode::Auto => {
            let service_url = normalize_service_url(
                destination
                    .service_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(config.service_url.trim()),
            )?;
            let endpoint = format!(
                "{}/v3/conversations/{}/activities",
                service_url,
                urlencoding::encode(&destination.conversation_id)
            );
            let response = super::outbound_rate_limit::send_with_bounded_retries(
                "teams",
                "botframework_message",
                client
                    .post(endpoint.clone())
                    .bearer_auth(config.access_token.trim())
                    .json(&build_bot_framework_payload(config, destination, message)),
            )
            .await
            .context("failed to send Teams Bot Framework message")?;
            let status = response.status();
            let payload: serde_json::Value =
                response.json().await.unwrap_or(serde_json::Value::Null);
            if !status.is_success() {
                let err = payload
                    .get("error")
                    .and_then(|value| value.get("message"))
                    .and_then(|value| value.as_str())
                    .or_else(|| payload.get("message").and_then(|value| value.as_str()))
                    .unwrap_or("Teams Bot Framework send failed");
                return Err(anyhow!(err.to_string()));
            }
            Ok(TeamsOutboundResponse {
                activity_id: payload
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                conversation_id: Some(destination.conversation_id.clone()),
                service_url: Some(service_url),
                endpoint: Some(endpoint),
            })
        }
    }
}

pub async fn handle_activity(
    agent: &SharedAgent,
    config: &TeamsTransportConfig,
    activity: TeamsActivity,
    verified_inbound: TeamsVerifiedInbound,
) -> Result<TeamsInboundSummary> {
    validate_config(config)?;
    let conversation_id =
        conversation_id(&activity).ok_or_else(|| anyhow!("conversation id is required"))?;
    let storage = {
        let guard = agent.read().await;
        guard.storage.clone()
    };

    let destination = TeamsReplyDestination {
        conversation_id: conversation_id.clone(),
        service_url: Some(verified_inbound.service_url.clone()),
        chat_id: config.chat_id.clone().or_else(|| {
            activity
                .conversation
                .as_ref()
                .and_then(|conversation| conversation.conversation_type.as_deref())
                .filter(|conversation_type| conversation_type.eq_ignore_ascii_case("chat"))
                .map(|_| conversation_id.clone())
        }),
        team_id: config.team_id.clone().or_else(|| {
            activity
                .channel_data
                .as_ref()
                .and_then(|channel_data| channel_data.team.as_ref())
                .map(|team| team.id.clone())
        }),
        channel_id: config.channel_id.clone().or_else(|| {
            activity
                .channel_data
                .as_ref()
                .and_then(|channel_data| channel_data.channel.as_ref())
                .map(|channel| channel.id.clone())
        }),
        thread_id: activity.reply_to_id.clone(),
        tenant_id: config.tenant_id.clone().or_else(|| {
            activity
                .channel_data
                .as_ref()
                .and_then(|channel_data| channel_data.tenant.as_ref())
                .map(|tenant| tenant.id.clone())
        }),
        user_id: activity.from.as_ref().map(|from| from.id.clone()),
        user_name: activity.from.as_ref().and_then(|from| from.name.clone()),
        last_activity_id: activity.id.clone(),
        last_reply_to_id: activity.reply_to_id.clone(),
        last_seen_at: Some(now_rfc3339()),
    };

    let reply_destination_key = destination_key(&conversation_id);

    if !matches!(activity.activity_type.as_str(), "message" | "invoke") {
        return Ok(TeamsInboundSummary {
            activity_id: activity.id,
            conversation_id,
            reply_destination_key,
            processed: false,
            response_preview: None,
        });
    }

    let Some(text) = sanitize_text(activity.text.clone()) else {
        return Ok(TeamsInboundSummary {
            activity_id: activity.id,
            conversation_id,
            reply_destination_key,
            processed: false,
            response_preview: None,
        });
    };

    if let (Some(from), Some(bot_app_id)) = (
        activity.from.as_ref().and_then(|identity| {
            let id = identity.id.trim();
            if id.is_empty() {
                None
            } else {
                Some(id)
            }
        }),
        config
            .bot_app_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        if from.eq_ignore_ascii_case(bot_app_id) {
            return Ok(TeamsInboundSummary {
                activity_id: activity.id,
                conversation_id,
                reply_destination_key,
                processed: false,
                response_preview: None,
            });
        }
    }

    let sender_identity = activity.from.clone().unwrap_or_default();
    let sender_id = sender_identity.id.trim().to_string();
    if sender_id.is_empty() {
        return Ok(TeamsInboundSummary {
            activity_id: activity.id,
            conversation_id,
            reply_destination_key,
            processed: false,
            response_preview: None,
        });
    }

    if let Some(activity_id) = activity.id.as_deref() {
        if record_teams_activity_id(&storage, activity_id).await? {
            tracing::debug!("Ignoring duplicate Teams activity {}", activity_id);
            return Ok(TeamsInboundSummary {
                activity_id: activity.id,
                conversation_id,
                reply_destination_key,
                processed: false,
                response_preview: None,
            });
        }
    }

    let trust_decision = {
        let guard = agent.read().await;
        let settings = sender_verification::load_settings(&guard.storage).await?;
        let identity = SenderIdentity {
            channel: SenderChannel::Teams,
            sender_id: sender_id.clone(),
            sender_label: sender_identity
                .name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| Some(sender_id.clone())),
            scope_id: destination.tenant_id.clone(),
            scope_label: activity
                .channel_data
                .as_ref()
                .and_then(|value| value.team.as_ref())
                .and_then(|team| team.name.clone())
                .or_else(|| destination.tenant_id.clone()),
            conversation_id: Some(conversation_id.clone()),
            message_preview: Some(text.clone()),
        };
        sender_verification::evaluate_sender_with_rules(
            &guard.storage,
            &identity,
            settings.teams.policy,
            &settings.teams.allowed_senders,
        )
        .await?
    };

    if let SenderTrustDecision::NeedsApproval {
        request: _request,
        created_new,
    } = trust_decision
    {
        let sender_label = sender_identity
            .name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| sender_id.clone());
        if created_new {
            let guard = agent.read().await;
            guard
                .emit_notification_forced(
                    "Sender Approval Needed",
                    &teams_sender_verification_notification(
                        sender_label.as_str(),
                        destination.tenant_id.as_deref(),
                        text.as_str(),
                    ),
                    "warning",
                    "sender_verification",
                )
                .await;
        }
        let _ = send_message_to_destination(
            config,
            &destination,
            &TeamsOutboundMessage {
                conversation_id: conversation_id.clone(),
                text: teams_sender_verification_notice(sender_label.as_str()),
                reply_to_id: activity
                    .id
                    .clone()
                    .or_else(|| destination.last_reply_to_id.clone()),
                service_url: destination.service_url.clone(),
                team_id: destination.team_id.clone(),
                channel_id: destination.channel_id.clone(),
                chat_id: destination.chat_id.clone(),
                content_type: None,
            },
        )
        .await;
        return Ok(TeamsInboundSummary {
            activity_id: activity.id,
            conversation_id,
            reply_destination_key,
            processed: false,
            response_preview: None,
        });
    }

    persist_destination(&storage, config, destination.clone()).await?;

    let response = {
        let agent_snapshot = Agent::snapshot(agent).await;
        agent_snapshot
            .process_message_with_meta(&text, "teams", Some(&conversation_id), None)
            .await
    };

    let response_text = match response {
        Ok(processed) => Agent::render_plain_channel_response(processed),
        Err(error) => format!("Error: {}", error),
    };

    if !response_text.trim().is_empty() {
        for chunk in super::outbound_split::split_for_provider_safe_channel("teams", &response_text)
        {
            if let Err(error) = send_message_to_destination(
                config,
                &destination,
                &TeamsOutboundMessage {
                    conversation_id: conversation_id.clone(),
                    text: chunk,
                    reply_to_id: activity
                        .id
                        .clone()
                        .or_else(|| destination.last_reply_to_id.clone()),
                    service_url: destination.service_url.clone(),
                    team_id: destination.team_id.clone(),
                    channel_id: destination.channel_id.clone(),
                    chat_id: destination.chat_id.clone(),
                    content_type: None,
                },
            )
            .await
            {
                tracing::warn!("Teams reply send failed: {}", error);
                break;
            }
        }
    }

    Ok(TeamsInboundSummary {
        activity_id: activity.id,
        conversation_id,
        reply_destination_key,
        processed: true,
        response_preview: Some(response_text.chars().take(120).collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_id_prefers_conversation() {
        let activity = TeamsActivity {
            conversation: Some(TeamsConversation {
                id: "conv-1".to_string(),
                ..Default::default()
            }),
            reply_to_id: Some("reply-1".to_string()),
            ..Default::default()
        };
        assert_eq!(conversation_id(&activity), Some("conv-1".to_string()));
    }

    #[test]
    fn graph_payload_includes_reply_to_id() {
        let message = TeamsOutboundMessage {
            conversation_id: "conv-1".to_string(),
            text: "hello".to_string(),
            reply_to_id: Some("activity-1".to_string()),
            ..Default::default()
        };
        let payload = build_graph_payload(&message);
        assert_eq!(
            payload.get("replyToId").and_then(|value| value.as_str()),
            Some("activity-1")
        );
    }

    #[test]
    fn normalize_service_url_rejects_non_https() {
        assert!(normalize_service_url("http://example.com").is_err());
    }

    #[test]
    fn normalize_service_url_strips_trailing_slash() {
        assert_eq!(
            normalize_service_url("https://smba.trafficmanager.net/teams/").unwrap(),
            "https://smba.trafficmanager.net/teams"
        );
    }

    #[test]
    fn validate_config_requires_bot_app_id() {
        let config = TeamsTransportConfig {
            service_url: "https://smba.trafficmanager.net/teams".to_string(),
            access_token: "token".to_string(),
            bot_app_id: None,
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn validate_config_rejects_untrusted_service_url() {
        let config = TeamsTransportConfig {
            service_url: "http://smba.trafficmanager.net/teams".to_string(),
            access_token: "token".to_string(),
            bot_app_id: Some("bot-app-id".to_string()),
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn proactive_destination_uses_configured_scope_without_reply_to() {
        let config = TeamsTransportConfig {
            service_url: "https://smba.trafficmanager.net/teams".to_string(),
            access_token: "token".to_string(),
            bot_app_id: Some("bot-app-id".to_string()),
            team_id: Some("team-1".to_string()),
            channel_id: Some("channel-1".to_string()),
            ..Default::default()
        };

        let destination = load_default_destination(&config).unwrap().unwrap();
        assert_eq!(destination.conversation_id, "team-1:channel-1");
        assert_eq!(destination.last_reply_to_id, None);
        assert_eq!(destination.chat_id, None);
    }

    #[test]
    fn recent_activity_state_is_pruned_to_a_bounded_window() {
        let mut state = TeamsRecentActivityState {
            recent: (0..(MAX_RECENT_ACTIVITY_IDS + 10))
                .map(|idx| TeamsRecentActivityEntry {
                    activity_id: format!("activity-{}", idx),
                    seen_at: 1,
                })
                .collect(),
        };
        prune_recent_activity_state(&mut state, RECENT_ACTIVITY_ID_WINDOW_SECS + 2);
        assert!(state.recent.len() <= MAX_RECENT_ACTIVITY_IDS);
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn record_activity_id_is_idempotent_for_retries() {
        let _dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();
        assert!(!record_teams_activity_id(&storage, "activity-1")
            .await
            .unwrap());
        assert!(record_teams_activity_id(&storage, "activity-1")
            .await
            .unwrap());
    }
}
