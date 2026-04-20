//! Matrix transport foundation.
//!
//! Self-contained transport helpers for outbound homeserver sends and inbound
//! sync polling. The module stores sync state and room context in the existing
//! encrypted KV store so it can be wired into the broader channel system later
//! without schema churn.
use anyhow::{anyhow, bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::Agent;
use crate::storage::Storage;

type SharedAgent = Arc<RwLock<Agent>>;

const CONFIG_STORAGE_KEY: &str = "channels:matrix:config";
const MATRIX_STATE_KEY_PREFIX: &str = "matrix:sync_state:v1:";
const MATRIX_DEFAULT_TIMEOUT_MS: u64 = 0;
const MATRIX_DEFAULT_LIMIT: usize = 100;
const MATRIX_MAX_ROOMS_TRACKED: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixTransportConfig {
    pub homeserver_url: String,
    pub access_token: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_room_id: Option<String>,
    #[serde(default)]
    pub sync_timeout_ms: u64,
    #[serde(default)]
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

impl Default for MatrixTransportConfig {
    fn default() -> Self {
        Self {
            homeserver_url: String::new(),
            access_token: String::new(),
            user_id: String::new(),
            device_id: None,
            account_id: None,
            default_room_id: None,
            sync_timeout_ms: MATRIX_DEFAULT_TIMEOUT_MS,
            limit: MATRIX_DEFAULT_LIMIT,
            user_agent: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixRoomContext {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixSyncState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(default)]
    pub rooms: BTreeMap<String, MatrixRoomContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixOutboundMessage {
    pub room_id: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msgtype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixOutboundResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixSyncSummary {
    pub rooms_seen: usize,
    pub messages_seen: usize,
    pub messages_forwarded: usize,
    pub conversations_updated: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixSyncEnvelope {
    #[serde(default)]
    pub next_batch: Option<String>,
    #[serde(default)]
    pub rooms: MatrixRoomsEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixRoomsEnvelope {
    #[serde(default)]
    pub join: BTreeMap<String, MatrixRoomSync>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixRoomSync {
    #[serde(default)]
    pub timeline: MatrixTimeline,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatrixTimeline {
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn account_key(config: &MatrixTransportConfig) -> String {
    let homeserver = config.homeserver_url.trim().trim_end_matches('/');
    let user_id = config.user_id.trim();
    let account = config
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");
    format!(
        "{}|{}|{}",
        account,
        homeserver.to_ascii_lowercase(),
        user_id.to_ascii_lowercase()
    )
}

fn state_key(config: &MatrixTransportConfig) -> String {
    format!("{}{}", MATRIX_STATE_KEY_PREFIX, account_key(config))
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode matrix payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode matrix payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

async fn load_state(storage: &Storage, config: &MatrixTransportConfig) -> Result<MatrixSyncState> {
    load_json(storage, &state_key(config)).await
}

async fn save_state(
    storage: &Storage,
    config: &MatrixTransportConfig,
    state: &MatrixSyncState,
) -> Result<()> {
    save_json(storage, &state_key(config), state).await
}

fn build_client(config: &MatrixTransportConfig) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(user_agent) = config
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder = builder.user_agent(user_agent.to_string());
    }
    builder
        .build()
        .context("failed to build Matrix HTTP client")
}

fn matrix_url(config: &MatrixTransportConfig, path: &str) -> String {
    let base = config.homeserver_url.trim().trim_end_matches('/');
    format!("{}/{}", base, path.trim_start_matches('/'))
}

fn room_conversation_id(room_id: &str, thread_root_event_id: Option<&str>) -> String {
    match thread_root_event_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(thread_id) => format!("{}#{}", room_id, thread_id),
        None => room_id.to_string(),
    }
}

fn extract_message_body(event: &serde_json::Value) -> Option<String> {
    let msg_type = event
        .get("content")
        .and_then(|value| value.get("msgtype"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if !matches!(msg_type, "m.text" | "m.notice" | "m.emote") {
        return None;
    }
    let body = event
        .get("content")
        .and_then(|value| value.get("body"))
        .and_then(|value| value.as_str())?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_thread_root_event_id(event: &serde_json::Value) -> Option<String> {
    let relates_to = event.get("content")?.get("m.relates_to")?;
    relates_to
        .get("event_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn update_room_context(
    state: &mut MatrixSyncState,
    room_id: &str,
    event: &serde_json::Value,
    conversation_id: &str,
) {
    let context = state
        .rooms
        .entry(room_id.to_string())
        .or_insert_with(|| MatrixRoomContext {
            room_id: room_id.to_string(),
            conversation_id: Some(conversation_id.to_string()),
            ..Default::default()
        });

    context.conversation_id = Some(conversation_id.to_string());
    context.last_sync_at = Some(now_rfc3339());
    context.last_event_id = event
        .get("event_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    context.last_sender = event
        .get("sender")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    context.thread_root_event_id = extract_thread_root_event_id(event);
    context.destination_hint = event
        .get("content")
        .and_then(|value| value.get("body"))
        .and_then(|value| value.as_str())
        .map(|body| body.chars().take(48).collect::<String>());
}

fn prune_state(state: &mut MatrixSyncState) {
    if state.rooms.len() <= MATRIX_MAX_ROOMS_TRACKED {
        return;
    }

    let mut rooms: Vec<_> = state.rooms.values().cloned().collect();
    rooms.sort_by(|left, right| left.last_sync_at.cmp(&right.last_sync_at));
    rooms.reverse();
    rooms.truncate(MATRIX_MAX_ROOMS_TRACKED);
    state.rooms = rooms
        .into_iter()
        .map(|room| (room.room_id.clone(), room))
        .collect();
}

pub async fn load_config_from_storage(storage: &Storage) -> Result<Option<MatrixTransportConfig>> {
    if let Ok(Some(raw)) = storage.get(CONFIG_STORAGE_KEY).await {
        if let Ok(config) = serde_json::from_slice::<MatrixTransportConfig>(&raw) {
            return Ok(Some(config));
        }
    }

    let homeserver_url = std::env::var("MATRIX_HOMESERVER_URL").unwrap_or_default();
    let access_token = std::env::var("MATRIX_ACCESS_TOKEN").unwrap_or_default();
    let user_id = std::env::var("MATRIX_USER_ID").unwrap_or_default();
    let device_id = std::env::var("MATRIX_DEVICE_ID").ok();
    let account_id = std::env::var("MATRIX_ACCOUNT_ID").ok();
    let default_room_id = std::env::var("MATRIX_DEFAULT_ROOM_ID").ok();
    let sync_timeout_ms = std::env::var("MATRIX_SYNC_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(MATRIX_DEFAULT_TIMEOUT_MS);
    let limit = std::env::var("MATRIX_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(MATRIX_DEFAULT_LIMIT);
    let user_agent = std::env::var("MATRIX_USER_AGENT").ok();

    if homeserver_url.trim().is_empty()
        && access_token.trim().is_empty()
        && user_id.trim().is_empty()
    {
        return Ok(None);
    }

    Ok(Some(MatrixTransportConfig {
        homeserver_url,
        access_token,
        user_id,
        device_id,
        account_id,
        default_room_id,
        sync_timeout_ms,
        limit,
        user_agent,
    }))
}

async fn load_config(agent: &Agent) -> Result<Option<MatrixTransportConfig>> {
    if let Some(config) = agent.config.matrix.clone() {
        return Ok(Some(config));
    }
    load_config_from_storage(&agent.storage).await
}

fn choose_outbound_destination(
    _state: &MatrixSyncState,
    config: &MatrixTransportConfig,
) -> Option<(String, Option<String>)> {
    if let Some(default_room_id) = config
        .default_room_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some((default_room_id.to_string(), None));
    }
    None
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = load_config(agent)
        .await?
        .ok_or_else(|| anyhow!("Matrix is not configured"))?;
    let state = load_state(&agent.storage, &config).await?;
    let (room_id, thread_root_event_id) = choose_outbound_destination(&state, &config)
        .ok_or_else(|| anyhow!("Matrix has no destination room yet"))?;
    let message = MatrixOutboundMessage {
        room_id,
        body: text.to_string(),
        formatted_body: None,
        thread_root_event_id,
        msgtype: None,
    };
    send_message_to_room(&config, &message).await?;
    Ok(())
}

pub async fn send_message_to_room(
    config: &MatrixTransportConfig,
    message: &MatrixOutboundMessage,
) -> Result<MatrixOutboundResponse> {
    if message.room_id.trim().is_empty() {
        bail!("room_id is required");
    }
    let body = message.body.trim();
    if body.is_empty() {
        bail!("message body is required");
    }

    let client = build_client(config)?;
    let txn_id = uuid::Uuid::new_v4().to_string();
    let url = matrix_url(
        config,
        &format!(
            "_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            urlencoding::encode(message.room_id.trim()),
            txn_id
        ),
    );

    let mut content = serde_json::json!({
        "msgtype": message.msgtype.as_deref().unwrap_or("m.text"),
        "body": body,
    });
    if let Some(formatted_body) = message
        .formatted_body
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        content["format"] = serde_json::Value::String("org.matrix.custom.html".to_string());
        content["formatted_body"] = serde_json::Value::String(formatted_body.to_string());
    }
    if let Some(thread_root_event_id) = message
        .thread_root_event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        content["m.relates_to"] = serde_json::json!({
            "rel_type": "m.thread",
            "event_id": thread_root_event_id,
            "is_falling_back": true
        });
    }

    let response = super::outbound_rate_limit::send_with_bounded_retries(
        "matrix",
        "send_message",
        client
            .put(url)
            .bearer_auth(config.access_token.trim())
            .json(&content),
    )
    .await
    .context("failed to send Matrix message")?;

    let status = response.status();
    let payload: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
    if !status.is_success() {
        let message = payload
            .get("error")
            .and_then(|value| value.as_str())
            .or_else(|| payload.get("errcode").and_then(|value| value.as_str()))
            .unwrap_or("Matrix send failed");
        return Err(anyhow!(message.to_string()));
    }

    Ok(MatrixOutboundResponse {
        event_id: payload
            .get("event_id")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        transaction_id: Some(txn_id),
    })
}

pub async fn sync_once(
    agent: &SharedAgent,
    config: &MatrixTransportConfig,
) -> Result<MatrixSyncSummary> {
    let storage = {
        let guard = agent.read().await;
        guard.storage.clone()
    };
    let mut state = load_state(&storage, config).await?;

    let url = matrix_url(config, "_matrix/client/v3/sync");
    let mut query = vec![
        ("timeout", config.sync_timeout_ms.to_string()),
        ("limit", config.limit.max(1).to_string()),
    ];
    if let Some(next_batch) = state
        .next_batch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        query.push(("since", next_batch.to_string()));
    }

    let client = build_client(config)?;
    let response = client
        .get(&url)
        .bearer_auth(config.access_token.trim())
        .query(&query)
        .send()
        .await
        .context("failed to sync Matrix rooms")?;

    let status = response.status();
    let payload: MatrixSyncEnvelope = response
        .json()
        .await
        .context("failed to decode Matrix sync response")?;
    if !status.is_success() {
        bail!("Matrix sync failed");
    }

    let mut summary = MatrixSyncSummary {
        next_batch: payload.next_batch.clone(),
        ..Default::default()
    };

    for (room_id, room_sync) in payload.rooms.join.iter() {
        summary.rooms_seen += 1;
        for event in &room_sync.timeline.events {
            summary.messages_seen += 1;
            let Some(body) = extract_message_body(event) else {
                continue;
            };
            let thread_root_event_id = extract_thread_root_event_id(event);
            let conversation_id = room_conversation_id(room_id, thread_root_event_id.as_deref());
            let sender = event
                .get("sender")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or("");
            if !sender.is_empty() && sender.eq_ignore_ascii_case(config.user_id.trim()) {
                continue;
            }

            update_room_context(&mut state, room_id, event, &conversation_id);
            let result = {
                let agent_snapshot = Agent::snapshot(agent).await;
                agent_snapshot
                    .process_message_with_meta(&body, "matrix", Some(&conversation_id), None)
                    .await
            };
            match result {
                Ok(processed) => {
                    let response = Agent::render_plain_channel_response(processed);
                    summary.messages_forwarded += 1;
                    if !response.trim().is_empty() {
                        if let Err(error) = send_message_to_room(
                            config,
                            &MatrixOutboundMessage {
                                room_id: room_id.clone(),
                                body: response,
                                formatted_body: None,
                                thread_root_event_id: thread_root_event_id.clone(),
                                msgtype: None,
                            },
                        )
                        .await
                        {
                            tracing::warn!("Matrix reply send failed: {}", error);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("Matrix message handling failed: {}", error);
                }
            }
            summary.conversations_updated += 1;
        }
    }

    state.next_batch = payload.next_batch;
    state.last_sync_at = Some(now_rfc3339());
    prune_state(&mut state);
    save_state(&storage, config, &state).await?;
    Ok(summary)
}

pub async fn serve(
    agent: SharedAgent,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                let storage = {
                    let guard = agent.read().await;
                    guard.storage.clone()
                };
                let config = load_config_from_storage(&storage).await?;
                if let Some(config) = config {
                    let _ = sync_once(&agent, &config).await;
                }
            }
            changed = shutdown.changed() => {
                match changed {
                    Ok(_) => break,
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_id_includes_thread_root_when_present() {
        assert_eq!(
            room_conversation_id("!room:example.org", Some("$event")),
            "!room:example.org#$event"
        );
    }

    #[test]
    fn conversation_id_defaults_to_room_id() {
        assert_eq!(
            room_conversation_id("!room:example.org", None),
            "!room:example.org"
        );
    }

    #[test]
    fn extracts_text_messages_only() {
        let event = serde_json::json!({
            "content": { "msgtype": "m.text", "body": " hello " }
        });
        assert_eq!(extract_message_body(&event), Some("hello".to_string()));
    }

    #[test]
    fn proactive_destination_requires_default_room_and_drops_threads() {
        let mut state = MatrixSyncState::default();
        state.rooms.insert(
            "!room:example.org".to_string(),
            MatrixRoomContext {
                room_id: "!room:example.org".to_string(),
                thread_root_event_id: Some("$thread".to_string()),
                ..Default::default()
            },
        );
        let config = MatrixTransportConfig {
            homeserver_url: "https://matrix.example.org".to_string(),
            access_token: "token".to_string(),
            user_id: "@bot:example.org".to_string(),
            default_room_id: Some("!room:example.org".to_string()),
            ..Default::default()
        };

        assert_eq!(
            choose_outbound_destination(&state, &config),
            Some(("!room:example.org".to_string(), None))
        );

        let no_default = MatrixTransportConfig {
            default_room_id: None,
            ..config
        };
        assert_eq!(choose_outbound_destination(&state, &no_default), None);
    }
}
