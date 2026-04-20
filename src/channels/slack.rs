//! Slack transport foundation.
//!
//! This module is intentionally self-contained so it can be wired into the
//! channel tree later without changing shared glue files.
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};

use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustDecision};
use crate::core::Agent;
use crate::storage::Storage;

type SharedAgent = Arc<RwLock<Agent>>;

const CONFIG_STORAGE_KEY: &str = "channels:slack:config";
const LAST_DESTINATION_STORAGE_KEY: &str = "channels:slack:last_destination";
const LAST_CHANNEL_STORAGE_KEY: &str = "channels:slack:last_channel_id";
const LAST_THREAD_STORAGE_KEY: &str = "channels:slack:last_thread_ts";
const LAST_TEAM_STORAGE_KEY: &str = "channels:slack:last_team_id";
const LAST_USER_STORAGE_KEY: &str = "channels:slack:last_user_id";
const LAST_MESSAGE_STORAGE_KEY: &str = "channels:slack:last_message_ts";
const RECENT_EVENT_IDS_STORAGE_KEY: &str = "channels:slack:recent_event_ids";
const MAX_RECENT_EVENT_IDS: usize = 64;
const RECENT_EVENT_ID_WINDOW_SECS: u64 = 60 * 60 * 24;

static SLACK_EVENT_DEDUP_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackChannelConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub signing_secret: String,
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub default_channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thread_ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
}

impl Default for SlackChannelConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            signing_secret: String::new(),
            api_base_url: default_api_base_url(),
            default_channel_id: String::new(),
            default_thread_ts: None,
            workspace_id: None,
            workspace_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackDestinationContext {
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_ts: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackWebhookEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    challenge: Option<String>,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    event: Option<SlackEvent>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    thread_ts: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SlackApiResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SlackRecentEventState {
    #[serde(default)]
    recent: Vec<SlackRecentEventEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SlackRecentEventEntry {
    event_id: String,
    seen_at: u64,
}

fn default_api_base_url() -> String {
    "https://slack.com/api".to_string()
}

fn trim_trailing_slashes(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn now_unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| anyhow!("system clock is before UNIX_EPOCH: {}", e))?
        .as_secs())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    crate::security::constant_time_eq(left, right)
}

fn slack_hmac_sha256_hex(secret: &str, data: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK_SIZE {
        let mut hasher = Sha256::new();
        hasher.update(&key);
        key = hasher.finalize().to_vec();
    }
    if key.len() < BLOCK_SIZE {
        key.resize(BLOCK_SIZE, 0);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for (idx, byte) in key.iter().enumerate() {
        ipad[idx] ^= byte;
        opad[idx] ^= byte;
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    hex::encode(outer.finalize())
}

fn slack_request_signature(timestamp: &str, raw_body: &[u8]) -> Vec<u8> {
    format!("v0:{}:{}", timestamp, String::from_utf8_lossy(raw_body)).into_bytes()
}

fn slack_conversation_id(context: &SlackDestinationContext) -> String {
    let team = context.team_id.as_deref().unwrap_or("workspace");
    let thread = context
        .thread_ts
        .as_deref()
        .or(context.message_ts.as_deref())
        .unwrap_or("root");
    format!("slack:{}:{}:{}", team, context.channel_id, thread)
}

fn slack_sender_verification_notice(user_id: &str) -> String {
    format!(
        "Sender approval required before I can respond here.\n\nUser: `{}`\nOpen `Settings -> Connected Systems -> Sender Verification` to approve this sender.",
        user_id.trim()
    )
}

fn slack_sender_verification_notification(
    user_id: &str,
    team_id: Option<&str>,
    text: &str,
) -> String {
    let scope = team_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("\nWorkspace: {}", value))
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
        "A new Slack sender needs approval before {} will act.\nSender: {}{}{}\nApprove it in Settings -> Connected Systems -> Sender Verification.",
        crate::branding::PRODUCT_NAME,
        user_id.trim(),
        scope,
        preview
    )
}

async fn load_config(agent: &Agent) -> Result<Option<SlackChannelConfig>> {
    if let Some(config) = agent.config.slack.clone() {
        return Ok(Some(config));
    }
    load_config_from_storage(&agent.storage).await
}

pub async fn load_config_from_storage(storage: &Storage) -> Result<Option<SlackChannelConfig>> {
    if let Ok(Some(raw)) = storage.get(CONFIG_STORAGE_KEY).await {
        if let Ok(config) = serde_json::from_slice::<SlackChannelConfig>(&raw) {
            return Ok(Some(config));
        }
    }

    let bot_token = std::env::var("SLACK_BOT_TOKEN").unwrap_or_default();
    let signing_secret = std::env::var("SLACK_SIGNING_SECRET").unwrap_or_default();
    let default_channel_id = std::env::var("SLACK_DEFAULT_CHANNEL_ID").unwrap_or_default();
    let api_base_url =
        std::env::var("SLACK_API_BASE_URL").unwrap_or_else(|_| default_api_base_url());
    let default_thread_ts = std::env::var("SLACK_DEFAULT_THREAD_TS").ok();
    let workspace_id = std::env::var("SLACK_WORKSPACE_ID").ok();
    let workspace_name = std::env::var("SLACK_WORKSPACE_NAME").ok();

    if bot_token.is_empty()
        && signing_secret.is_empty()
        && default_channel_id.is_empty()
        && default_thread_ts.is_none()
        && workspace_id.is_none()
        && workspace_name.is_none()
    {
        return Ok(None);
    }

    Ok(Some(SlackChannelConfig {
        bot_token,
        signing_secret,
        api_base_url,
        default_channel_id,
        default_thread_ts,
        workspace_id,
        workspace_name,
    }))
}

#[allow(dead_code)]
async fn load_destination(agent: &Agent) -> Result<Option<SlackDestinationContext>> {
    if let Ok(Some(raw)) = agent.storage.get(LAST_DESTINATION_STORAGE_KEY).await {
        if let Ok(context) = serde_json::from_slice::<SlackDestinationContext>(&raw) {
            return Ok(Some(context));
        }
    }

    let channel_id = agent
        .storage
        .get(LAST_CHANNEL_STORAGE_KEY)
        .await
        .ok()
        .flatten();
    if let Some(channel_id) = channel_id {
        let context = SlackDestinationContext {
            channel_id: String::from_utf8_lossy(&channel_id).to_string(),
            thread_ts: agent
                .storage
                .get(LAST_THREAD_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
            team_id: agent
                .storage
                .get(LAST_TEAM_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
            user_id: agent
                .storage
                .get(LAST_USER_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
            message_ts: agent
                .storage
                .get(LAST_MESSAGE_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
        };
        return Ok(Some(context));
    }

    Ok(None)
}

async fn persist_destination(agent: &Agent, context: &SlackDestinationContext) -> Result<()> {
    let raw = serde_json::to_vec(context)?;
    agent
        .storage
        .set(LAST_DESTINATION_STORAGE_KEY, &raw)
        .await?;
    agent
        .storage
        .set(LAST_CHANNEL_STORAGE_KEY, context.channel_id.as_bytes())
        .await?;
    if let Some(thread_ts) = &context.thread_ts {
        agent
            .storage
            .set(LAST_THREAD_STORAGE_KEY, thread_ts.as_bytes())
            .await?;
    }
    if let Some(team_id) = &context.team_id {
        agent
            .storage
            .set(LAST_TEAM_STORAGE_KEY, team_id.as_bytes())
            .await?;
    }
    if let Some(user_id) = &context.user_id {
        agent
            .storage
            .set(LAST_USER_STORAGE_KEY, user_id.as_bytes())
            .await?;
    }
    if let Some(message_ts) = &context.message_ts {
        agent
            .storage
            .set(LAST_MESSAGE_STORAGE_KEY, message_ts.as_bytes())
            .await?;
    }
    Ok(())
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?)
}

async fn slack_api_post_message(
    config: &SlackChannelConfig,
    destination: &SlackDestinationContext,
    text: &str,
) -> Result<()> {
    if config.bot_token.trim().is_empty() {
        return Err(anyhow!("Slack bot token is missing"));
    }
    if destination.channel_id.trim().is_empty() {
        return Err(anyhow!("Slack channel destination is missing"));
    }

    let client = http_client()?;
    let url = format!(
        "{}/chat.postMessage",
        trim_trailing_slashes(&config.api_base_url)
    );
    let mut body = serde_json::json!({
        "channel": destination.channel_id,
        "text": text,
        "mrkdwn": true,
        "unfurl_links": true,
        "unfurl_media": true
    });
    let thread_ts = destination
        .thread_ts
        .as_deref()
        .or(config.default_thread_ts.as_deref());
    if let Some(thread_ts) = thread_ts {
        body["thread_ts"] = Value::String(thread_ts.to_string());
    }

    let response = super::outbound_rate_limit::send_with_bounded_retries(
        "slack",
        "chat.postMessage",
        client.post(&url).bearer_auth(&config.bot_token).json(&body),
    )
    .await?;

    let status = response.status();
    let payload = response
        .json::<SlackApiResponse>()
        .await
        .unwrap_or_default();
    if !status.is_success() || !payload.ok {
        let error = payload.error.unwrap_or(status.to_string());
        return Err(anyhow!("Slack API error: {}", error));
    }

    Ok(())
}

fn resolve_destination(config: &SlackChannelConfig) -> Result<SlackDestinationContext> {
    if !config.default_channel_id.trim().is_empty() {
        return Ok(SlackDestinationContext {
            channel_id: config.default_channel_id.clone(),
            thread_ts: config.default_thread_ts.clone(),
            team_id: config.workspace_id.clone(),
            user_id: None,
            message_ts: None,
        });
    }

    Err(anyhow!("Slack has no configured notification destination"))
}

pub fn default_destination(config: &SlackChannelConfig) -> Result<SlackDestinationContext> {
    resolve_destination(config)
}

pub async fn send_message_with_config(config: &SlackChannelConfig, text: &str) -> Result<()> {
    let destination = resolve_destination(config)?;
    slack_api_post_message(config, &destination, text).await
}

fn verify_slack_signature(
    signing_secret: &str,
    timestamp: &str,
    signature: &str,
    raw_body: &[u8],
) -> Result<()> {
    if signing_secret.trim().is_empty() {
        return Ok(());
    }

    let ts: i64 = timestamp
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid Slack timestamp header"))?;
    let now = now_unix_seconds()? as i64;
    if (now - ts).abs() > 60 * 5 {
        return Err(anyhow!(
            "Slack request timestamp is outside the allowed window"
        ));
    }

    let expected = slack_hmac_sha256_hex(
        signing_secret,
        &slack_request_signature(timestamp.trim(), raw_body),
    );
    let expected_sig = format!("v0={}", expected);
    if !constant_time_eq(expected_sig.as_bytes(), signature.trim().as_bytes()) {
        return Err(anyhow!("Slack request signature verification failed"));
    }
    Ok(())
}

fn slack_event_payload(body: &[u8]) -> Result<SlackWebhookEnvelope> {
    Ok(serde_json::from_slice(body)?)
}

fn slack_event_message_text(event: &SlackEvent) -> Option<&str> {
    event.text.as_deref().filter(|text| !text.trim().is_empty())
}

fn slack_event_should_process(event: &SlackEvent) -> bool {
    if event.event_type != "message" {
        return false;
    }
    if event.bot_id.as_deref().is_some() {
        return false;
    }
    match event.subtype.as_deref() {
        None => true,
        Some("file_share") => true,
        Some("bot_message") => false,
        Some("message_changed") => false,
        Some("message_deleted") => false,
        Some(_) => false,
    }
}

fn prune_recent_event_state(state: &mut SlackRecentEventState, now: u64) {
    state
        .recent
        .retain(|entry| now.saturating_sub(entry.seen_at) <= RECENT_EVENT_ID_WINDOW_SECS);
    if state.recent.len() > MAX_RECENT_EVENT_IDS {
        let excess = state.recent.len() - MAX_RECENT_EVENT_IDS;
        state.recent.drain(0..excess);
    }
}

async fn load_recent_event_state(storage: &Storage) -> Result<SlackRecentEventState> {
    if let Ok(Some(raw)) = storage.get(RECENT_EVENT_IDS_STORAGE_KEY).await {
        if let Ok(state) = serde_json::from_slice::<SlackRecentEventState>(&raw) {
            return Ok(state);
        }
    }
    Ok(SlackRecentEventState::default())
}

async fn persist_recent_event_state(
    storage: &Storage,
    state: &SlackRecentEventState,
) -> Result<()> {
    let raw = serde_json::to_vec(state)?;
    storage.set(RECENT_EVENT_IDS_STORAGE_KEY, &raw).await?;
    Ok(())
}

async fn record_slack_event_id(storage: &Storage, event_id: &str) -> Result<bool> {
    let event_id = event_id.trim();
    if event_id.is_empty() {
        return Ok(false);
    }

    let _guard = SLACK_EVENT_DEDUP_LOCK.lock().await;
    let now = now_unix_seconds()?;
    let mut state = load_recent_event_state(storage).await?;
    prune_recent_event_state(&mut state, now);
    if state.recent.iter().any(|entry| entry.event_id == event_id) {
        return Ok(true);
    }

    state.recent.push(SlackRecentEventEntry {
        event_id: event_id.to_string(),
        seen_at: now,
    });
    prune_recent_event_state(&mut state, now);
    persist_recent_event_state(storage, &state).await?;
    Ok(false)
}

fn require_slack_signature_headers(
    signing_secret: &str,
    timestamp: Option<&str>,
    signature: Option<&str>,
) -> Result<()> {
    if signing_secret.trim().is_empty() {
        return Ok(());
    }

    timestamp.ok_or_else(|| {
        anyhow!("Slack signature timestamp header is required when a signing secret is configured")
    })?;
    signature.ok_or_else(|| {
        anyhow!("Slack signature header is required when a signing secret is configured")
    })?;
    Ok(())
}

fn verify_webhook_request_with_config(
    config: Option<&SlackChannelConfig>,
    raw_body: &[u8],
    timestamp: Option<&str>,
    signature: Option<&str>,
) -> Result<()> {
    let Some(config) = config else {
        return Err(anyhow!("Slack is not configured for inbound webhooks"));
    };

    if config.signing_secret.trim().is_empty() {
        return Err(anyhow!(
            "Slack signing secret is required for inbound webhooks"
        ));
    }

    require_slack_signature_headers(&config.signing_secret, timestamp, signature)?;
    let (Some(ts), Some(sig)) = (timestamp, signature) else {
        return Err(anyhow!(
            "Slack signature headers are required when a signing secret is configured"
        ));
    };
    verify_slack_signature(&config.signing_secret, ts, sig, raw_body)
}

pub fn verify_webhook_request_for_config(
    config: &SlackChannelConfig,
    raw_body: &[u8],
    timestamp: Option<&str>,
    signature: Option<&str>,
) -> Result<()> {
    verify_webhook_request_with_config(Some(config), raw_body, timestamp, signature)
}

pub async fn verify_webhook_request(
    agent: SharedAgent,
    raw_body: &[u8],
    timestamp: Option<&str>,
    signature: Option<&str>,
) -> Result<()> {
    let config = {
        let agent = agent.read().await;
        load_config(&agent).await?
    };
    verify_webhook_request_with_config(config.as_ref(), raw_body, timestamp, signature)
}

/// Send a Slack message to the last seen destination or default channel.
pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = load_config(agent)
        .await?
        .ok_or_else(|| anyhow!("Slack is not configured"))?;
    let destination = resolve_destination(&config)?;
    slack_api_post_message(&config, &destination, text).await?;
    Ok(())
}

/// Handle an inbound Slack webhook/event payload.
///
/// Returns either the URL verification challenge or a lightweight acknowledgement.
pub async fn handle_webhook(
    agent: SharedAgent,
    raw_body: &[u8],
    timestamp: Option<&str>,
    signature: Option<&str>,
) -> Result<String> {
    let config = {
        let agent = agent.read().await;
        load_config(&agent).await?
    };
    handle_webhook_with_config(agent, config.as_ref(), raw_body, timestamp, signature).await
}

pub async fn handle_webhook_with_config(
    agent: SharedAgent,
    config: Option<&SlackChannelConfig>,
    raw_body: &[u8],
    timestamp: Option<&str>,
    signature: Option<&str>,
) -> Result<String> {
    let payload = slack_event_payload(raw_body)?;
    let config = config.cloned();
    verify_webhook_request_with_config(config.as_ref(), raw_body, timestamp, signature)?;

    if payload.event_type == "url_verification" {
        let challenge = payload
            .challenge
            .ok_or_else(|| anyhow!("Slack url_verification payload missing challenge"))?;
        return Ok(challenge);
    }

    if let Some(event_id) = payload.event_id.as_deref() {
        let is_duplicate = {
            let agent = agent.read().await;
            record_slack_event_id(&agent.storage, event_id).await?
        };
        if is_duplicate {
            return Ok("duplicate".to_string());
        }
    }

    let Some(event) = payload.event else {
        return Ok("ignored".to_string());
    };
    if !slack_event_should_process(&event) {
        return Ok("ignored".to_string());
    }

    let Some(text) = slack_event_message_text(&event) else {
        return Ok("ignored".to_string());
    };
    let channel_id = event
        .channel
        .clone()
        .ok_or_else(|| anyhow!("Slack message missing channel id"))?;
    let team_id = payload
        .team_id
        .clone()
        .or_else(|| config.as_ref().and_then(|c| c.workspace_id.clone()));
    let thread_ts = event.thread_ts.clone().or_else(|| event.ts.clone());
    let conversation_context = SlackDestinationContext {
        channel_id: channel_id.clone(),
        thread_ts: thread_ts.clone(),
        team_id,
        user_id: event.user.clone(),
        message_ts: event.ts.clone(),
    };
    let conversation_id = slack_conversation_id(&conversation_context);
    let Some(sender_id) = event.user.clone().filter(|value| !value.trim().is_empty()) else {
        return Ok("ignored".to_string());
    };

    let trust_decision = {
        let agent = agent.read().await;
        let settings = sender_verification::load_settings(&agent.storage).await?;
        let identity = SenderIdentity {
            channel: SenderChannel::Slack,
            sender_id: sender_id.clone(),
            sender_label: Some(sender_id.clone()),
            scope_id: conversation_context.team_id.clone(),
            scope_label: config
                .as_ref()
                .and_then(|value| value.workspace_name.clone())
                .or_else(|| conversation_context.team_id.clone()),
            conversation_id: Some(conversation_id.clone()),
            message_preview: Some(text.to_string()),
        };
        sender_verification::evaluate_sender_with_rules(
            &agent.storage,
            &identity,
            settings.slack.policy,
            &settings.slack.allowed_senders,
        )
        .await?
    };

    if let SenderTrustDecision::NeedsApproval {
        request: _request,
        created_new,
    } = trust_decision
    {
        if created_new {
            let agent = agent.read().await;
            agent
                .emit_notification_forced(
                    "Sender Approval Needed",
                    &slack_sender_verification_notification(
                        sender_id.as_str(),
                        conversation_context.team_id.as_deref(),
                        text,
                    ),
                    "warning",
                    "sender_verification",
                )
                .await;
        }
        if let Some(config) = config.as_ref() {
            let _ = slack_api_post_message(
                config,
                &conversation_context,
                &slack_sender_verification_notice(sender_id.as_str()),
            )
            .await;
        }
        return Ok("approval_pending".to_string());
    }

    let response = {
        let agent_snapshot = Agent::snapshot(&agent).await;
        persist_destination(&agent_snapshot, &conversation_context).await?;
        let processed = agent_snapshot
            .process_message_with_meta(text, "slack", Some(&conversation_id), None)
            .await?;
        Agent::render_plain_channel_response(processed)
    };

    if response.trim().is_empty() {
        return Ok("ok".to_string());
    }

    if let Some(config) = config.as_ref() {
        slack_api_post_message(config, &conversation_context, &response).await?;
    }

    Ok("ok".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_signature_contains_prefix_and_timestamp() {
        let payload = slack_request_signature("1700000000", br#"{"hello":"world"}"#);
        let text = String::from_utf8(payload).unwrap();
        assert!(text.starts_with("v0:1700000000:"));
    }

    #[test]
    fn signature_verification_round_trip() {
        let raw_body = br#"{"type":"url_verification","challenge":"abc"}"#;
        let timestamp = now_unix_seconds().unwrap().to_string();
        let secret = "topsecret";
        let expected = slack_hmac_sha256_hex(
            secret,
            &slack_request_signature(timestamp.as_str(), raw_body),
        );
        let signature = format!("v0={}", expected);
        assert!(verify_slack_signature(secret, timestamp.as_str(), &signature, raw_body).is_ok());
    }

    #[test]
    fn signature_verification_rejects_tampered_body() {
        let raw_body = br#"{"type":"url_verification","challenge":"abc"}"#;
        let tampered_body = br#"{"type":"url_verification","challenge":"xyz"}"#;
        let timestamp = "1700000000";
        let secret = "topsecret";
        let expected = slack_hmac_sha256_hex(secret, &slack_request_signature(timestamp, raw_body));
        let signature = format!("v0={}", expected);
        assert!(verify_slack_signature(secret, timestamp, &signature, tampered_body).is_err());
    }

    #[test]
    fn signature_headers_are_required_when_secret_is_configured() {
        assert!(require_slack_signature_headers("topsecret", None, None).is_err());
        assert!(require_slack_signature_headers("", None, None).is_ok());
    }

    #[test]
    fn webhook_verification_fails_closed_when_config_is_missing() {
        let error = verify_webhook_request_with_config(None, br#"{}"#, Some("1"), Some("v0=test"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not configured"));
    }

    #[test]
    fn webhook_verification_fails_closed_when_secret_is_blank() {
        let config = SlackChannelConfig {
            bot_token: "xoxb-test".to_string(),
            signing_secret: String::new(),
            ..Default::default()
        };
        let error =
            verify_webhook_request_with_config(Some(&config), br#"{}"#, Some("1"), Some("v0=test"))
                .unwrap_err()
                .to_string();
        assert!(error.contains("signing secret"));
    }

    #[test]
    fn recent_event_state_is_pruned_to_a_bounded_window() {
        let mut state = SlackRecentEventState {
            recent: (0..(MAX_RECENT_EVENT_IDS + 10))
                .map(|idx| SlackRecentEventEntry {
                    event_id: format!("evt-{}", idx),
                    seen_at: 1,
                })
                .collect(),
        };
        prune_recent_event_state(&mut state, RECENT_EVENT_ID_WINDOW_SECS + 2);
        assert!(state.recent.len() <= MAX_RECENT_EVENT_IDS);
    }

    #[tokio::test]
    async fn record_event_id_is_idempotent_for_retries() {
        let _dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();
        assert!(!record_slack_event_id(&storage, "evt-1").await.unwrap());
        assert!(record_slack_event_id(&storage, "evt-1").await.unwrap());
    }

    #[test]
    fn slack_conversation_id_uses_thread_when_present() {
        let context = SlackDestinationContext {
            channel_id: "C123".to_string(),
            thread_ts: Some("1.2".to_string()),
            team_id: Some("T999".to_string()),
            user_id: None,
            message_ts: None,
        };
        assert_eq!(slack_conversation_id(&context), "slack:T999:C123:1.2");
    }

    #[test]
    fn resolve_destination_prefers_configured_channel_over_last_thread() {
        let config = SlackChannelConfig {
            bot_token: "token".to_string(),
            default_channel_id: "C-config".to_string(),
            default_thread_ts: None,
            workspace_id: Some("T-config".to_string()),
            ..Default::default()
        };

        let destination = resolve_destination(&config).unwrap();
        assert_eq!(destination.channel_id, "C-config");
        assert_eq!(destination.thread_ts, None);
        assert_eq!(destination.team_id.as_deref(), Some("T-config"));
    }
}
