//! Discord transport foundation.
//!
//! This module provides self-contained config, delivery, and ingestion
//! helpers for later channel-tree wiring.
use anyhow::{anyhow, Result};
use futures::{Sink, SinkExt, StreamExt};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;

use crate::core::Agent;
use crate::storage::Storage;

type SharedAgent = Arc<RwLock<Agent>>;

const CONFIG_STORAGE_KEY: &str = "channels:discord:config";
const LAST_DESTINATION_STORAGE_KEY: &str = "channels:discord:last_destination";
const LAST_CHANNEL_STORAGE_KEY: &str = "channels:discord:last_channel_id";
const LAST_GUILD_STORAGE_KEY: &str = "channels:discord:last_guild_id";
const LAST_THREAD_STORAGE_KEY: &str = "channels:discord:last_thread_id";
const LAST_MESSAGE_STORAGE_KEY: &str = "channels:discord:last_message_id";
const GATEWAY_STATE_STORAGE_KEY: &str = "channels:discord:gateway_state";
const SELF_USER_STORAGE_KEY: &str = "channels:discord:self_user_id";
const RECENT_EVENT_IDS_STORAGE_KEY: &str = "channels:discord:recent_event_ids";
const MAX_RECENT_EVENT_IDS: usize = 64;
const RECENT_EVENT_ID_WINDOW_SECS: u64 = 60 * 60 * 24;

static DISCORD_EVENT_DEDUP_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DiscordRecentEventState {
    #[serde(default)]
    recent: Vec<DiscordRecentEventEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscordRecentEventEntry {
    event_id: String,
    seen_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordChannelConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub default_channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
}

impl Default for DiscordChannelConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            webhook_url: String::new(),
            api_base_url: default_api_base_url(),
            default_channel_id: String::new(),
            default_thread_id: None,
            guild_id: None,
            application_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordDestinationContext {
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordAuthor {
    id: String,
    #[serde(default)]
    bot: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordMessageCreate {
    id: String,
    channel_id: String,
    #[serde(default)]
    guild_id: Option<String>,
    #[serde(default)]
    content: String,
    #[serde(default)]
    author: Option<DiscordAuthor>,
    #[serde(default)]
    webhook_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DiscordApiError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordGatewayRuntimeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_gateway_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DiscordGatewayEnvelope {
    op: u64,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    s: Option<u64>,
    #[serde(default)]
    t: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordGatewayHello {
    heartbeat_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordGatewayBotResponse {
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordGatewayReadyUser {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordGatewayReadyPayload {
    session_id: String,
    #[serde(default)]
    resume_gateway_url: Option<String>,
    user: DiscordGatewayReadyUser,
}

fn default_api_base_url() -> String {
    "https://discord.com/api/v10".to_string()
}

fn trim_trailing_slashes(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn append_thread_query(url: &str, thread_id: &str) -> String {
    if thread_id.trim().is_empty() {
        return url.to_string();
    }
    let separator = if url.contains('?') { "&" } else { "?" };
    format!(
        "{}{}thread_id={}",
        url,
        separator,
        urlencoding::encode(thread_id)
    )
}

async fn load_config(agent: &Agent) -> Result<Option<DiscordChannelConfig>> {
    if let Some(config) = agent.config.discord.clone() {
        return Ok(Some(config));
    }
    load_config_from_storage(&agent.storage).await
}

pub async fn load_config_from_storage(storage: &Storage) -> Result<Option<DiscordChannelConfig>> {
    if let Ok(Some(raw)) = storage.get(CONFIG_STORAGE_KEY).await {
        if let Ok(config) = serde_json::from_slice::<DiscordChannelConfig>(&raw) {
            return Ok(Some(config));
        }
    }

    let bot_token = std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default();
    let webhook_url = std::env::var("DISCORD_WEBHOOK_URL").unwrap_or_default();
    let api_base_url =
        std::env::var("DISCORD_API_BASE_URL").unwrap_or_else(|_| default_api_base_url());
    let default_channel_id = std::env::var("DISCORD_DEFAULT_CHANNEL_ID").unwrap_or_default();
    let default_thread_id = std::env::var("DISCORD_DEFAULT_THREAD_ID").ok();
    let guild_id = std::env::var("DISCORD_GUILD_ID").ok();
    let application_id = std::env::var("DISCORD_APPLICATION_ID").ok();
    if bot_token.is_empty()
        && webhook_url.is_empty()
        && default_channel_id.is_empty()
        && default_thread_id.is_none()
        && guild_id.is_none()
        && application_id.is_none()
    {
        return Ok(None);
    }

    Ok(Some(DiscordChannelConfig {
        bot_token,
        webhook_url,
        api_base_url,
        default_channel_id,
        default_thread_id,
        guild_id,
        application_id,
    }))
}

#[allow(dead_code)]
async fn load_destination(agent: &Agent) -> Result<Option<DiscordDestinationContext>> {
    if let Ok(Some(raw)) = agent.storage.get(LAST_DESTINATION_STORAGE_KEY).await {
        if let Ok(context) = serde_json::from_slice::<DiscordDestinationContext>(&raw) {
            return Ok(Some(context));
        }
    }

    let channel = agent
        .storage
        .get(LAST_CHANNEL_STORAGE_KEY)
        .await
        .ok()
        .flatten();
    if let Some(channel_id) = channel {
        let context = DiscordDestinationContext {
            channel_id: String::from_utf8_lossy(&channel_id).to_string(),
            guild_id: agent
                .storage
                .get(LAST_GUILD_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
            thread_id: agent
                .storage
                .get(LAST_THREAD_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
            message_id: agent
                .storage
                .get(LAST_MESSAGE_STORAGE_KEY)
                .await
                .ok()
                .flatten()
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
            webhook_url: None,
        };
        return Ok(Some(context));
    }

    Ok(None)
}

async fn persist_destination(agent: &Agent, context: &DiscordDestinationContext) -> Result<()> {
    let raw = serde_json::to_vec(context)?;
    agent
        .storage
        .set(LAST_DESTINATION_STORAGE_KEY, &raw)
        .await?;
    agent
        .storage
        .set(LAST_CHANNEL_STORAGE_KEY, context.channel_id.as_bytes())
        .await?;
    if let Some(guild_id) = &context.guild_id {
        agent
            .storage
            .set(LAST_GUILD_STORAGE_KEY, guild_id.as_bytes())
            .await?;
    }
    if let Some(thread_id) = &context.thread_id {
        agent
            .storage
            .set(LAST_THREAD_STORAGE_KEY, thread_id.as_bytes())
            .await?;
    }
    if let Some(message_id) = &context.message_id {
        agent
            .storage
            .set(LAST_MESSAGE_STORAGE_KEY, message_id.as_bytes())
            .await?;
    }
    Ok(())
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn discord_conversation_id(context: &DiscordDestinationContext) -> String {
    let scope = context.guild_id.as_deref().unwrap_or("dm");
    let thread = context
        .thread_id
        .as_deref()
        .unwrap_or(context.channel_id.as_str());
    format!("discord:{}:{}", scope, thread)
}

async fn send_via_webhook(
    config: &DiscordChannelConfig,
    destination: &DiscordDestinationContext,
    text: &str,
) -> Result<()> {
    let webhook_url = destination
        .webhook_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            if config.webhook_url.trim().is_empty() {
                None
            } else {
                Some(config.webhook_url.as_str())
            }
        })
        .ok_or_else(|| anyhow!("Discord webhook URL is missing"))?;

    let url = if let Some(thread_id) = destination.thread_id.as_deref() {
        append_thread_query(webhook_url, thread_id)
    } else if let Some(thread_id) = config.default_thread_id.as_deref() {
        append_thread_query(webhook_url, thread_id)
    } else {
        webhook_url.to_string()
    };

    let client = http_client()?;
    let response = super::outbound_rate_limit::send_with_bounded_retries(
        "discord",
        "webhook_message",
        client.post(url).json(&serde_json::json!({
            "content": text,
            "allowed_mentions": { "parse": [] }
        })),
    )
    .await?;

    if !(response.status().is_success() || response.status().as_u16() == 204) {
        let payload = response.text().await.unwrap_or_default();
        return Err(anyhow!("Discord webhook error: {}", payload));
    }

    Ok(())
}

async fn send_via_bot(
    config: &DiscordChannelConfig,
    destination: &DiscordDestinationContext,
    text: &str,
) -> Result<()> {
    if config.bot_token.trim().is_empty() {
        return Err(anyhow!("Discord bot token is missing"));
    }

    let channel_id = destination.channel_id.trim();
    if channel_id.is_empty() {
        return Err(anyhow!("Discord channel destination is missing"));
    }

    let url = format!(
        "{}/channels/{}/messages",
        trim_trailing_slashes(&config.api_base_url),
        channel_id
    );

    let client = http_client()?;
    let response = super::outbound_rate_limit::send_with_bounded_retries(
        "discord",
        "bot_message",
        client
            .post(&url)
            .header("Authorization", format!("Bot {}", config.bot_token))
            .json(&serde_json::json!({
                "content": text,
                "allowed_mentions": { "parse": [] }
            })),
    )
    .await?;

    if !response.status().is_success() {
        let payload = response.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<DiscordApiError>(&payload).ok();
        let message = parsed
            .and_then(|error| error.message)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(payload);
        return Err(anyhow!("Discord API error: {}", message));
    }

    Ok(())
}

fn resolve_destination(config: &DiscordChannelConfig) -> Result<DiscordDestinationContext> {
    if !config.default_channel_id.trim().is_empty() {
        return Ok(DiscordDestinationContext {
            channel_id: config.default_channel_id.clone(),
            guild_id: config.guild_id.clone(),
            thread_id: config.default_thread_id.clone(),
            message_id: None,
            webhook_url: if config.webhook_url.trim().is_empty() {
                None
            } else {
                Some(config.webhook_url.clone())
            },
        });
    }

    Err(anyhow!(
        "Discord has no configured notification destination"
    ))
}

fn should_process_message(message: &DiscordMessageCreate) -> bool {
    if message.content.trim().is_empty() {
        return false;
    }
    if message.webhook_id.is_some() {
        return false;
    }
    if let Some(author) = &message.author {
        if author.bot.unwrap_or(false) {
            return false;
        }
    }
    true
}

fn matches_configured_scope(message: &DiscordMessageCreate, config: &DiscordChannelConfig) -> bool {
    if let Some(guild_id) = config
        .guild_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if message.guild_id.as_deref().map(str::trim) != Some(guild_id) {
            return false;
        }
    }

    if let Some(thread_id) = config
        .default_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return message.channel_id.trim() == thread_id;
    }

    let channel_id = config.default_channel_id.trim();
    if !channel_id.is_empty() {
        return message.channel_id.trim() == channel_id;
    }

    config
        .guild_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn gateway_ws_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.contains("encoding=json") {
        trimmed.to_string()
    } else if trimmed.contains('?') {
        format!("{}&v=10&encoding=json", trimmed)
    } else {
        format!("{}?v=10&encoding=json", trimmed)
    }
}

async fn load_runtime_state(agent: &Agent) -> Result<DiscordGatewayRuntimeState> {
    if let Ok(Some(raw)) = agent.storage.get(GATEWAY_STATE_STORAGE_KEY).await {
        if let Ok(state) = serde_json::from_slice::<DiscordGatewayRuntimeState>(&raw) {
            return Ok(state);
        }
    }
    Ok(DiscordGatewayRuntimeState::default())
}

async fn save_runtime_state(agent: &Agent, state: &DiscordGatewayRuntimeState) -> Result<()> {
    let raw = serde_json::to_vec(state)?;
    agent.storage.set(GATEWAY_STATE_STORAGE_KEY, &raw).await?;
    if let Some(bot_user_id) = &state.bot_user_id {
        agent
            .storage
            .set(SELF_USER_STORAGE_KEY, bot_user_id.as_bytes())
            .await?;
    }
    Ok(())
}

fn now_unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock before unix epoch: {}", error))?
        .as_secs())
}

fn prune_recent_event_state(state: &mut DiscordRecentEventState, now: u64) {
    let min_seen_at = now.saturating_sub(RECENT_EVENT_ID_WINDOW_SECS);
    state.recent.retain(|entry| entry.seen_at >= min_seen_at);
    if state.recent.len() > MAX_RECENT_EVENT_IDS {
        let excess = state.recent.len() - MAX_RECENT_EVENT_IDS;
        state.recent.drain(0..excess);
    }
}

async fn load_recent_event_state(storage: &Storage) -> Result<DiscordRecentEventState> {
    if let Ok(Some(raw)) = storage.get(RECENT_EVENT_IDS_STORAGE_KEY).await {
        if let Ok(state) = serde_json::from_slice::<DiscordRecentEventState>(&raw) {
            return Ok(state);
        }
    }
    Ok(DiscordRecentEventState::default())
}

async fn persist_recent_event_state(
    storage: &Storage,
    state: &DiscordRecentEventState,
) -> Result<()> {
    let raw = serde_json::to_vec(state)?;
    storage.set(RECENT_EVENT_IDS_STORAGE_KEY, &raw).await?;
    Ok(())
}

async fn record_discord_event_id(storage: &Storage, event_id: &str) -> Result<bool> {
    let event_id = event_id.trim();
    if event_id.is_empty() {
        return Ok(false);
    }
    let _guard = DISCORD_EVENT_DEDUP_LOCK.lock().await;
    let now = now_unix_seconds()?;
    let mut state = load_recent_event_state(storage).await?;
    prune_recent_event_state(&mut state, now);
    if state.recent.iter().any(|entry| entry.event_id == event_id) {
        return Ok(true);
    }
    state.recent.push(DiscordRecentEventEntry {
        event_id: event_id.to_string(),
        seen_at: now,
    });
    prune_recent_event_state(&mut state, now);
    persist_recent_event_state(storage, &state).await?;
    Ok(false)
}

async fn load_self_user_id(agent: &Agent) -> Option<String> {
    if let Ok(Some(raw)) = agent.storage.get(SELF_USER_STORAGE_KEY).await {
        let value = String::from_utf8_lossy(&raw).trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

async fn resolve_gateway_endpoint(config: &DiscordChannelConfig) -> Result<String> {
    let client = http_client()?;
    let url = format!(
        "{}/gateway/bot",
        trim_trailing_slashes(&config.api_base_url)
    );
    let response = client
        .get(&url)
        .header("Authorization", format!("Bot {}", config.bot_token))
        .send()
        .await;

    if let Ok(response) = response {
        if response.status().is_success() {
            if let Ok(payload) = response.json::<DiscordGatewayBotResponse>().await {
                if !payload.url.trim().is_empty() {
                    return Ok(gateway_ws_url(&payload.url));
                }
            }
        }
    }

    Ok(gateway_ws_url("wss://gateway.discord.gg"))
}

async fn send_gateway_json(
    sender: &mut (impl Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    payload: &Value,
) -> Result<()> {
    let text = serde_json::to_string(payload)?;
    sender.send(Message::Text(text.into())).await?;
    Ok(())
}

async fn send_gateway_identify(
    sender: &mut (impl Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    token: &str,
) -> Result<()> {
    let payload = serde_json::json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": (1u64 << 0) | (1u64 << 9) | (1u64 << 12) | (1u64 << 15),
            "properties": {
                "$os": std::env::consts::OS,
                "$browser": crate::branding::PRODUCT_NAME,
                "$device": crate::branding::PRODUCT_NAME
            },
            "large_threshold": 50,
            "compress": false,
            "presence": {
                "status": "online",
                "since": Value::Null,
                "activities": [],
                "afk": false
            }
        }
    });
    send_gateway_json(sender, &payload).await
}

async fn send_gateway_resume(
    sender: &mut (impl Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    token: &str,
    session_id: &str,
    seq: u64,
) -> Result<()> {
    let payload = serde_json::json!({
        "op": 6,
        "d": {
            "token": token,
            "session_id": session_id,
            "seq": seq
        }
    });
    send_gateway_json(sender, &payload).await
}

async fn send_gateway_heartbeat(
    sender: &mut (impl Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    seq: Option<u64>,
) -> Result<()> {
    let payload = serde_json::json!({ "op": 1, "d": seq });
    send_gateway_json(sender, &payload).await
}

async fn handle_ready_event(
    agent: &SharedAgent,
    state: &mut DiscordGatewayRuntimeState,
    raw: &Value,
) -> Result<()> {
    let ready = serde_json::from_value::<DiscordGatewayReadyPayload>(raw.clone())?;
    state.session_id = Some(ready.session_id);
    state.resume_gateway_url = ready.resume_gateway_url;
    state.bot_user_id = Some(ready.user.id);
    let agent = agent.read().await;
    save_runtime_state(&agent, state).await?;
    Ok(())
}

async fn handle_message_create_event(
    agent: SharedAgent,
    state: DiscordGatewayRuntimeState,
    message: DiscordMessageCreate,
) -> Result<()> {
    if !should_process_message(&message) {
        return Ok(());
    }

    if let Some(author) = &message.author {
        if author.bot.unwrap_or(false) {
            return Ok(());
        }
        if let Some(bot_user_id) = state.bot_user_id.as_deref() {
            if author.id == bot_user_id {
                return Ok(());
            }
        }
    }

    let config = {
        let agent = agent.read().await;
        load_config(&agent).await?
    }
    .ok_or_else(|| anyhow!("Discord is not configured"))?;
    if config.bot_token.trim().is_empty() {
        return Err(anyhow!(
            "Discord bot token is required for inbound gateway messages"
        ));
    }
    if !matches_configured_scope(&message, &config) {
        tracing::debug!(
            "Discord message ignored because it is outside the configured guild/channel scope"
        );
        return Ok(());
    }
    let is_duplicate = {
        let agent = agent.read().await;
        record_discord_event_id(&agent.storage, &message.id).await?
    };
    if is_duplicate {
        tracing::debug!("Ignoring duplicate Discord message event {}", message.id);
        return Ok(());
    }

    let context = DiscordDestinationContext {
        channel_id: message.channel_id.clone(),
        guild_id: message.guild_id.clone(),
        thread_id: config
            .default_thread_id
            .clone()
            .filter(|thread_id| thread_id.trim() == message.channel_id.trim()),
        message_id: Some(message.id.clone()),
        webhook_url: None,
    };
    let conversation_id = discord_conversation_id(&context);
    let content = message.content.clone();

    let reply = {
        let agent_snapshot = Agent::snapshot(&agent).await;
        persist_destination(&agent_snapshot, &context).await?;
        let self_user_id = load_self_user_id(&agent_snapshot).await;
        if let (Some(author), Some(self_user_id)) =
            (message.author.as_ref(), self_user_id.as_deref())
        {
            if author.id == self_user_id || author.bot.unwrap_or(false) {
                return Ok(());
            }
        }
        agent_snapshot
            .process_message_with_meta(&content, "discord", Some(&conversation_id), None)
            .await
            .map(Agent::render_plain_channel_response)?
    };

    if reply.trim().is_empty() {
        return Ok(());
    }

    let agent_snapshot = Agent::snapshot(&agent).await;
    send_message_to_destination(&agent_snapshot, &context, &reply).await?;
    save_runtime_state(&agent_snapshot, &state).await?;
    Ok(())
}

async fn send_message_to_destination(
    agent: &Agent,
    destination: &DiscordDestinationContext,
    text: &str,
) -> Result<()> {
    let config = load_config(agent)
        .await?
        .unwrap_or_else(|| DiscordChannelConfig {
            bot_token: String::new(),
            webhook_url: destination.webhook_url.clone().unwrap_or_default(),
            api_base_url: default_api_base_url(),
            default_channel_id: destination.channel_id.clone(),
            default_thread_id: destination.thread_id.clone(),
            guild_id: destination.guild_id.clone(),
            application_id: None,
        });
    let used_webhook = destination
        .webhook_url
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || !config.webhook_url.trim().is_empty();

    if used_webhook {
        send_via_webhook(&config, destination, text).await
    } else {
        send_via_bot(&config, destination, text).await
    }
}

/// Send a Discord message using the configured proactive notification destination.
pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let maybe_config = load_config(agent).await?;
    let config = maybe_config
        .as_ref()
        .ok_or_else(|| anyhow!("Discord is not configured"))?;
    let destination = resolve_destination(config)?;
    send_message_to_destination(agent, &destination, text).await?;
    Ok(())
}

pub async fn run_gateway(agent: SharedAgent) -> Result<()> {
    let mut backoff = Duration::from_secs(2);
    let mut state = {
        let agent = agent.read().await;
        load_runtime_state(&agent).await?
    };

    loop {
        let maybe_config = {
            let agent = agent.read().await;
            load_config(&agent).await?
        };
        let Some(config) = maybe_config else {
            tracing::debug!("Discord gateway runtime: no config available yet");
            tokio::time::sleep(Duration::from_secs(60)).await;
            continue;
        };
        if config.bot_token.trim().is_empty() {
            tracing::debug!("Discord gateway runtime: bot token missing");
            tokio::time::sleep(Duration::from_secs(60)).await;
            continue;
        }

        let gateway_url = if let Some(resume_url) = state.resume_gateway_url.clone() {
            resume_url
        } else {
            match resolve_gateway_endpoint(&config).await {
                Ok(url) => url,
                Err(error) => {
                    tracing::warn!("Discord gateway endpoint lookup failed: {}", error);
                    gateway_ws_url("wss://gateway.discord.gg")
                }
            }
        };
        let url = gateway_ws_url(&gateway_url);
        let outcome = run_gateway_once(agent.clone(), &config, &mut state, &url).await;
        match outcome {
            Ok(()) => {
                backoff = Duration::from_secs(2);
            }
            Err(error) => {
                tracing::warn!("Discord gateway runtime error: {}", error);
                let delay = backoff;
                backoff = std::cmp::min(backoff.saturating_mul(2), Duration::from_secs(60));
                tokio::time::sleep(delay).await;
            }
        }
        let agent = agent.read().await;
        let _ = save_runtime_state(&agent, &state).await;
    }
}

async fn run_gateway_once(
    agent: SharedAgent,
    config: &DiscordChannelConfig,
    state: &mut DiscordGatewayRuntimeState,
    gateway_url: &str,
) -> Result<()> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(gateway_url).await?;
    let (mut sender, mut receiver) = ws_stream.split();

    let first = match receiver.next().await {
        Some(Ok(Message::Text(text))) => serde_json::from_str::<DiscordGatewayEnvelope>(&text)?,
        Some(Ok(Message::Binary(bytes))) => {
            serde_json::from_slice::<DiscordGatewayEnvelope>(&bytes)?
        }
        Some(Ok(Message::Ping(data))) => {
            sender.send(Message::Pong(data)).await?;
            return Ok(());
        }
        Some(Ok(Message::Close(_))) | None => return Ok(()),
        Some(Ok(_)) => return Ok(()),
        Some(Err(error)) => return Err(anyhow!(error)),
    };

    if first.op != 10 {
        return Err(anyhow!("Discord gateway did not send a hello frame"));
    }
    let hello = serde_json::from_value::<DiscordGatewayHello>(first.d)?;
    state.heartbeat_interval_ms = Some(hello.heartbeat_interval);
    let mut heartbeat_timer =
        tokio::time::interval(Duration::from_millis(hello.heartbeat_interval));
    heartbeat_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    if let Some(session_id) = state.session_id.as_deref() {
        if let Some(seq) = state.seq {
            send_gateway_resume(&mut sender, &config.bot_token, session_id, seq).await?;
        } else {
            send_gateway_identify(&mut sender, &config.bot_token).await?;
        }
    } else {
        send_gateway_identify(&mut sender, &config.bot_token).await?;
    }

    let mut awaiting_ack = false;
    loop {
        tokio::select! {
            _ = heartbeat_timer.tick() => {
                if awaiting_ack {
                    tracing::warn!("Discord gateway heartbeat ack missed; reconnecting");
                    return Ok(());
                }
                send_gateway_heartbeat(&mut sender, state.seq).await?;
                awaiting_ack = true;
            }
            message = receiver.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let envelope = serde_json::from_str::<DiscordGatewayEnvelope>(&text)?;
                        if let Some(seq) = envelope.s {
                            state.seq = Some(seq);
                        }
                        match envelope.op {
                            11 => {
                                awaiting_ack = false;
                            }
                            7 => {
                                return Ok(());
                            }
                            9 => {
                                let can_resume = envelope.d.as_bool().unwrap_or(false);
                                if !can_resume {
                                    state.session_id = None;
                                    state.seq = None;
                                }
                                return Ok(());
                            }
                            0 => {
                                if let Some(event_type) = envelope.t.as_deref() {
                                    match event_type {
                                        "READY" => {
                                            handle_ready_event(&agent, state, &envelope.d).await?;
                                        }
                                        "MESSAGE_CREATE" => {
                                            let message = serde_json::from_value::<DiscordMessageCreate>(envelope.d.clone())?;
                                            let agent_clone = agent.clone();
                                            let state_clone = state.clone();
                                            crate::spawn_logged!("src/channels/discord.rs:935", async move {
                                                if let Err(error) = handle_message_create_event(agent_clone, state_clone, message).await {
                                                    tracing::warn!("Discord message handling failed: {}", error);
                                                }
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let envelope = serde_json::from_slice::<DiscordGatewayEnvelope>(&bytes)?;
                        if let Some(seq) = envelope.s {
                            state.seq = Some(seq);
                        }
                        match envelope.op {
                            11 => {
                                awaiting_ack = false;
                            }
                            7 => {
                                return Ok(());
                            }
                            9 => {
                                let can_resume = envelope.d.as_bool().unwrap_or(false);
                                if !can_resume {
                                    state.session_id = None;
                                    state.seq = None;
                                }
                                return Ok(());
                            }
                            0 => {
                                if let Some(event_type) = envelope.t.as_deref() {
                                    match event_type {
                                        "READY" => {
                                            handle_ready_event(&agent, state, &envelope.d).await?;
                                        }
                                        "MESSAGE_CREATE" => {
                                            let message = serde_json::from_value::<DiscordMessageCreate>(envelope.d.clone())?;
                                            let agent_clone = agent.clone();
                                            let state_clone = state.clone();
                                            crate::spawn_logged!("src/channels/discord.rs:978", async move {
                                                if let Err(error) = handle_message_create_event(agent_clone, state_clone, message).await {
                                                    tracing::warn!("Discord message handling failed: {}", error);
                                                }
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        sender.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(anyhow!(error)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_conversation_id_uses_guild_or_dm() {
        let context = DiscordDestinationContext {
            channel_id: "123".to_string(),
            guild_id: Some("456".to_string()),
            thread_id: None,
            message_id: None,
            webhook_url: None,
        };
        assert_eq!(discord_conversation_id(&context), "discord:456:123");
    }

    #[test]
    fn webhook_url_append_thread_id() {
        assert_eq!(
            append_thread_query("https://discord.com/api/webhooks/abc", "thread-1"),
            "https://discord.com/api/webhooks/abc?thread_id=thread-1"
        );
    }

    #[test]
    fn ignores_bot_messages() {
        let message = DiscordMessageCreate {
            id: "1".to_string(),
            channel_id: "2".to_string(),
            guild_id: None,
            content: "hello".to_string(),
            author: Some(DiscordAuthor {
                id: "3".to_string(),
                bot: Some(true),
            }),
            webhook_id: None,
        };
        assert!(!should_process_message(&message));
    }

    #[test]
    fn matches_configured_scope_for_exact_channel() {
        let config = DiscordChannelConfig {
            bot_token: "token".to_string(),
            default_channel_id: "chan-1".to_string(),
            ..Default::default()
        };
        let message = DiscordMessageCreate {
            id: "1".to_string(),
            channel_id: "chan-1".to_string(),
            guild_id: Some("guild-1".to_string()),
            content: "hello".to_string(),
            author: None,
            webhook_id: None,
        };
        assert!(matches_configured_scope(&message, &config));
    }

    #[test]
    fn rejects_messages_outside_configured_scope() {
        let config = DiscordChannelConfig {
            bot_token: "token".to_string(),
            guild_id: Some("guild-1".to_string()),
            default_channel_id: "chan-1".to_string(),
            ..Default::default()
        };
        let message = DiscordMessageCreate {
            id: "1".to_string(),
            channel_id: "chan-2".to_string(),
            guild_id: Some("guild-2".to_string()),
            content: "hello".to_string(),
            author: None,
            webhook_id: None,
        };
        assert!(!matches_configured_scope(&message, &config));
    }

    #[test]
    fn proactive_destination_uses_configured_channel_and_thread_only() {
        let config = DiscordChannelConfig {
            bot_token: "token".to_string(),
            guild_id: Some("guild-1".to_string()),
            default_channel_id: "chan-config".to_string(),
            default_thread_id: None,
            ..Default::default()
        };

        let destination = resolve_destination(&config).unwrap();
        assert_eq!(destination.channel_id, "chan-config");
        assert_eq!(destination.thread_id, None);
        assert_eq!(destination.guild_id.as_deref(), Some("guild-1"));
    }
}
