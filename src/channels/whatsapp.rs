//! WhatsApp Business Cloud API channel
//!
//! Integrates with the Meta WhatsApp Business Cloud API (v18.0) to provide
//! bidirectional messaging. Supports webhook verification, inbound message
//! handling, outbound text replies, push notifications, and slash commands.
//!
//! API reference: https://developers.facebook.com/docs/whatsapp/cloud-api

use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use url::Url;

use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustDecision};
use crate::core::{Agent, TaskStatus};
use crate::storage::Storage;

type SharedAgent = Arc<RwLock<Agent>>;

const RECENT_MESSAGE_IDS_STORAGE_KEY: &str = "channels:whatsapp:recent_message_ids";
const MAX_RECENT_MESSAGE_IDS: usize = 64;
const RECENT_MESSAGE_ID_WINDOW_SECS: u64 = 60 * 60 * 24;

static WHATSAPP_MESSAGE_DEDUP_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct WhatsAppRecentMessageState {
    #[serde(default)]
    recent: Vec<WhatsAppRecentMessageEntry>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct WhatsAppRecentMessageEntry {
    message_id: String,
    seen_at: u64,
}

#[derive(Clone, Copy, Debug)]
enum TunnelControlCommand {
    Start,
    Stop,
    Status,
}

fn parse_tunnel_command(text: &str) -> Option<TunnelControlCommand> {
    let normalized = text.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.as_str() {
        "/tunnel start" | "/start tunnel" => Some(TunnelControlCommand::Start),
        "/tunnel stop" | "/stop tunnel" => Some(TunnelControlCommand::Stop),
        "/tunnel" | "/tunnel status" => Some(TunnelControlCommand::Status),
        _ => None,
    }
}

fn internal_api_base_url() -> String {
    crate::core::net::internal_api_base_url()
}

fn now_unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock before unix epoch: {}", error))?
        .as_secs())
}

fn prune_recent_message_state(state: &mut WhatsAppRecentMessageState, now: u64) {
    let min_seen_at = now.saturating_sub(RECENT_MESSAGE_ID_WINDOW_SECS);
    state.recent.retain(|entry| entry.seen_at >= min_seen_at);
    if state.recent.len() > MAX_RECENT_MESSAGE_IDS {
        let excess = state.recent.len() - MAX_RECENT_MESSAGE_IDS;
        state.recent.drain(0..excess);
    }
}

async fn load_recent_message_state(storage: &Storage) -> Result<WhatsAppRecentMessageState> {
    if let Ok(Some(raw)) = storage.get(RECENT_MESSAGE_IDS_STORAGE_KEY).await {
        if let Ok(state) = serde_json::from_slice::<WhatsAppRecentMessageState>(&raw) {
            return Ok(state);
        }
    }
    Ok(WhatsAppRecentMessageState::default())
}

async fn persist_recent_message_state(
    storage: &Storage,
    state: &WhatsAppRecentMessageState,
) -> Result<()> {
    let raw = serde_json::to_vec(state)?;
    storage.set(RECENT_MESSAGE_IDS_STORAGE_KEY, &raw).await?;
    Ok(())
}

async fn record_whatsapp_message_id(storage: &Storage, message_id: &str) -> Result<bool> {
    let message_id = message_id.trim();
    if message_id.is_empty() {
        return Ok(false);
    }
    let _guard = WHATSAPP_MESSAGE_DEDUP_LOCK.lock().await;
    let now = now_unix_seconds()?;
    let mut state = load_recent_message_state(storage).await?;
    prune_recent_message_state(&mut state, now);
    if state
        .recent
        .iter()
        .any(|entry| entry.message_id == message_id)
    {
        return Ok(true);
    }
    state.recent.push(WhatsAppRecentMessageEntry {
        message_id: message_id.to_string(),
        seen_at: now,
    });
    prune_recent_message_state(&mut state, now);
    persist_recent_message_state(storage, &state).await?;
    Ok(false)
}

async fn execute_tunnel_command(agent: &SharedAgent, cmd: TunnelControlCommand) -> String {
    let api_key = { agent.read().await.api_key.clone() };
    let base_url = internal_api_base_url();
    let client = match crate::core::net::build_internal_control_client(10) {
        Ok(c) => c,
        Err(e) => return format!("Tunnel command failed: {}", e),
    };
    let url = match cmd {
        TunnelControlCommand::Start => format!("{}/tunnel/start", base_url),
        TunnelControlCommand::Stop => format!("{}/tunnel/stop", base_url),
        TunnelControlCommand::Status => format!("{}/tunnel/status", base_url),
    };

    let mut request = match cmd {
        TunnelControlCommand::Status => client.get(&url),
        TunnelControlCommand::Start | TunnelControlCommand::Stop => client.post(&url),
    };
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => return format!("Failed to reach tunnel controller at {}: {}", base_url, e),
    };
    let status = response.status();
    let json: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => return format!("Tunnel command failed (invalid response): {}", e),
    };

    if !status.is_success() {
        let err = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return format!("Tunnel command failed: {}", err);
    }

    match cmd {
        TunnelControlCommand::Start => {
            let url = json.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if !url.is_empty() {
                format!("Tunnel started.\nExternal URL: {}", url)
            } else {
                json.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Tunnel is starting; URL pending.")
                    .to_string()
            }
        }
        TunnelControlCommand::Stop => json
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Tunnel stopped.")
            .to_string(),
        TunnelControlCommand::Status => {
            let active = json
                .get("active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut out = format!(
                "Tunnel status: {}",
                if active { "active" } else { "inactive" }
            );
            if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                if !url.is_empty() {
                    out.push_str(&format!("\nExternal URL: {}", url));
                }
            }
            if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                if !err.is_empty() {
                    out.push_str(&format!("\nLast error: {}", err));
                }
            }
            out
        }
    }
}

async fn process_whatsapp_prompt(
    agent: &SharedAgent,
    prompt: &str,
    conversation_id: &str,
) -> String {
    let agent_snapshot = Agent::snapshot(agent).await;
    match agent_snapshot
        .process_message_with_meta(prompt, "whatsapp", Some(conversation_id), None)
        .await
    {
        Ok(processed) => Agent::render_plain_channel_response(processed),
        Err(error) => format!("Error: {}", error),
    }
}

/// Base URL for the Meta WhatsApp Business Cloud API (v18.0).
const API_BASE: &str = "https://graph.facebook.com/v18.0";

/// Maximum text message length supported by WhatsApp before truncation.
/// The actual limit is 4096 characters for text body messages.
const MAX_MESSAGE_LEN: usize = 4096;

pub const EMBEDDED_BRIDGE_URL: &str = "http://127.0.0.1:8999";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Connection mode: Baileys (QR scan) or Meta Business Cloud API.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppMode {
    /// Baileys bridge — scan QR code, no Meta account needed.
    #[default]
    Baileys,
    /// Meta Business Cloud API — production-grade, requires Business account.
    CloudApi,
}

/// Runtime ownership for the Baileys bridge path.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppBridgeRuntime {
    /// AgentArk manages a bundled localhost bridge process.
    #[default]
    Embedded,
    /// AgentArk talks to a separately managed bridge over HTTP.
    External,
}

/// Configuration for the WhatsApp channel (supports both Baileys and Cloud API).
///
/// Stored as part of `AgentConfig` and serialized to `config.toml`.
/// Sensitive tokens are managed via encrypted secrets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WhatsAppChannelConfig {
    /// Connection mode.
    #[serde(default)]
    pub mode: WhatsAppMode,

    // ---- Cloud API fields ----
    /// Permanent or temporary access token issued by Meta (Cloud API mode only).
    #[serde(default)]
    pub access_token: String,

    /// Phone Number ID registered with the WhatsApp Business account (Cloud API only).
    #[serde(default)]
    pub phone_number_id: String,

    /// App secret used to verify Cloud API webhook signatures.
    #[serde(default)]
    pub app_secret: String,

    /// Token used to verify the webhook endpoint during initial setup.
    #[serde(default)]
    pub verify_token: String,

    // ---- Baileys bridge fields ----
    /// Runtime ownership for the Baileys bridge path.
    ///
    /// Older configs may omit this and infer the runtime from `bridge_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_runtime: Option<WhatsAppBridgeRuntime>,

    /// URL of the Baileys bridge. Embedded mode always uses the fixed localhost
    /// bridge address; external mode uses this configured URL.
    #[serde(default = "default_bridge_url")]
    pub bridge_url: String,

    /// Optional access token for bridge requests (external mode, or embedded for
    /// future hardening parity with other bridge-backed channels).
    #[serde(default)]
    pub bridge_token: String,

    // ---- Shared fields ----
    /// If non-empty, only messages from these phone numbers (E.164 format,
    /// e.g. "15551234567") are accepted. An empty list allows all senders.
    #[serde(default)]
    pub allowed_numbers: Vec<String>,

    /// DM policy: "pairing" (require approval code) or "open" (accept all).
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
}

fn default_bridge_url() -> String {
    EMBEDDED_BRIDGE_URL.to_string()
}

fn default_dm_policy() -> String {
    "pairing".to_string()
}

pub fn is_loopback_bridge_url(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }

    match Url::parse(trimmed) {
        Ok(url) => match url.host_str().map(|host| host.to_ascii_lowercase()) {
            Some(host) => matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]"),
            None => false,
        },
        Err(_) => false,
    }
}

pub fn infer_bridge_runtime_from_url(value: &str) -> WhatsAppBridgeRuntime {
    if is_loopback_bridge_url(value) {
        WhatsAppBridgeRuntime::Embedded
    } else {
        WhatsAppBridgeRuntime::External
    }
}

impl WhatsAppChannelConfig {
    pub fn bridge_runtime(&self) -> WhatsAppBridgeRuntime {
        self.bridge_runtime
            .unwrap_or_else(|| infer_bridge_runtime_from_url(&self.bridge_url))
    }

    pub fn uses_embedded_bridge(&self) -> bool {
        self.mode == WhatsAppMode::Baileys
            && self.bridge_runtime() == WhatsAppBridgeRuntime::Embedded
    }

    pub fn uses_external_bridge(&self) -> bool {
        self.mode == WhatsAppMode::Baileys
            && self.bridge_runtime() == WhatsAppBridgeRuntime::External
    }

    pub fn effective_bridge_url(&self) -> Result<String> {
        match self.bridge_runtime() {
            WhatsAppBridgeRuntime::Embedded => Ok(default_bridge_url()),
            WhatsAppBridgeRuntime::External => {
                let trimmed = self.bridge_url.trim().trim_end_matches('/');
                if trimmed.is_empty() {
                    Err(anyhow!("WhatsApp external bridge URL is missing"))
                } else {
                    Ok(trimmed.to_string())
                }
            }
        }
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    crate::security::constant_time_eq(left, right)
}

fn hmac_sha256_hex(secret: &str, body: &[u8]) -> String {
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
    for (index, byte) in key.iter().enumerate() {
        ipad[index] ^= byte;
        opad[index] ^= byte;
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(body);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    hex::encode(outer.finalize())
}

pub fn verify_cloud_api_request_signature(
    config: &WhatsAppChannelConfig,
    raw_body: &[u8],
    signature: Option<&str>,
) -> Result<()> {
    let app_secret = config.app_secret.trim();
    if app_secret.is_empty() {
        return Err(anyhow!(
            "WhatsApp app secret is required for Cloud API webhooks"
        ));
    }
    let provided = signature
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("WhatsApp signature header is required"))?;
    let expected = format!("sha256={}", hmac_sha256_hex(app_secret, raw_body));
    if !constant_time_eq(expected.as_bytes(), provided.as_bytes()) {
        return Err(anyhow!("WhatsApp webhook signature verification failed"));
    }
    Ok(())
}

fn normalize_whatsapp_sender(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>()
}

pub(crate) fn configured_notification_recipient(config: &WhatsAppChannelConfig) -> Option<String> {
    let mut recipients = config
        .allowed_numbers
        .iter()
        .map(|value| normalize_whatsapp_sender(value))
        .filter(|value| !value.is_empty());
    let first = recipients.next()?;
    if recipients.next().is_some() {
        return None;
    }
    Some(first)
}

fn whatsapp_sender_identity(from: &str, preview: Option<&str>) -> SenderIdentity {
    SenderIdentity {
        channel: SenderChannel::Whatsapp,
        sender_id: from.trim().to_string(),
        sender_label: Some(from.trim().to_string()),
        scope_id: None,
        scope_label: None,
        conversation_id: Some(format!("whatsapp:{}", from.trim())),
        message_preview: preview.map(str::to_string),
    }
}

async fn whatsapp_sender_is_approved(storage: &Storage, from: &str) -> bool {
    let identity = whatsapp_sender_identity(from, None);
    if sender_verification::is_sender_approved(storage, &identity)
        .await
        .unwrap_or(false)
    {
        return true;
    }
    let legacy_key = format!("whatsapp:approved:{}", normalize_whatsapp_sender(from));
    storage.get(&legacy_key).await.ok().flatten().is_some()
}

async fn approve_whatsapp_sender(storage: &Storage, from: &str, approved_by: &str) -> Result<()> {
    let identity = whatsapp_sender_identity(from, None);
    sender_verification::approve_sender(storage, &identity, Some(approved_by)).await?;
    let normalized = normalize_whatsapp_sender(from);
    if !normalized.is_empty() {
        storage
            .set(&format!("whatsapp:approved:{}", normalized), b"1")
            .await?;
        let _ = storage
            .delete(&format!("whatsapp:pairing:{}", normalized))
            .await;
    }
    Ok(())
}

fn parse_set_secret(text: &str) -> Option<(String, String)> {
    crate::core::secrets::parse_set_secret_command(text)
}

fn parse_use_current_llm_key(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("/usecurrentkey ") {
        let key = trimmed[15..].trim();
        if key.is_empty()
            || key.chars().any(|c| c.is_whitespace())
            || key.contains('\n')
            || key.contains('\r')
        {
            return None;
        }
        return Some(key.to_string());
    }
    crate::core::secrets::parse_use_current_llm_key_command(trimmed)
}

async fn store_secret_for_chat(agent: &SharedAgent, key: &str, value: &str) -> Result<(), String> {
    let (config_dir, data_dir) = {
        let a = agent.read().await;
        (a.config_dir.clone(), a.data_dir.clone())
    };
    let k = key.to_string();
    let v = value.to_string();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        crate::core::secrets::store_user_secret(&config_dir, Some(&data_dir), &k, &v)
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

async fn link_current_llm_key_for_chat(agent: &SharedAgent, key: &str) -> Result<String, String> {
    let llm_env = {
        let a = agent.read().await;
        a.config.llm.app_env_vars()
    };
    if let Some(value) = llm_env.get(key).cloned().filter(|v| !v.trim().is_empty()) {
        store_secret_for_chat(agent, key, &value).await?;
        return Ok(format!(
            "Linked '{}' to the current model credential (stored encrypted).",
            key
        ));
    }

    let mut available_keys: Vec<String> = llm_env
        .iter()
        .filter_map(|(k, v)| {
            if v.trim().is_empty() {
                None
            } else if k.ends_with("_API_KEY")
                || k.ends_with("_BASE_URL")
                || k == "LLM_MODEL"
                || k == "LLM_PROVIDER"
            {
                Some(k.clone())
            } else {
                None
            }
        })
        .collect();
    available_keys.sort();
    let available = if available_keys.is_empty() {
        "none".to_string()
    } else {
        available_keys.join(", ")
    };
    Err(format!(
        "I can't map '{}' from current model settings. Available model-backed keys: {}. Save a credential for this key in the secure web UI.",
        key, available
    ))
}

// ---------------------------------------------------------------------------
// Message splitting
// ---------------------------------------------------------------------------

/// Split a message into chunks that fit within the WhatsApp text limit.
///
/// Tries to split at paragraph boundaries (`\n\n`) first, then falls back to
/// line boundaries (`\n`), so the recipient sees coherent chunks.
#[cfg(test)]
#[allow(dead_code)]
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for paragraph in text.split("\n\n") {
        let separator = if current.is_empty() { "" } else { "\n\n" };
        let candidate_len = current.len() + separator.len() + paragraph.len();

        if candidate_len <= max_len {
            current.push_str(separator);
            current.push_str(paragraph);
        } else if paragraph.len() > max_len {
            // Paragraph itself exceeds the limit — flush current, then split by lines.
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            for line in paragraph.lines() {
                let line_sep = if current.is_empty() { "" } else { "\n" };
                if current.len() + line_sep.len() + line.len() <= max_len {
                    current.push_str(line_sep);
                    current.push_str(line);
                } else {
                    if !current.is_empty() {
                        chunks.push(std::mem::take(&mut current));
                    }
                    // If a single line exceeds max_len, hard-cut it.
                    if line.len() > max_len {
                        let mut remaining = line;
                        while !remaining.is_empty() {
                            let end = std::cmp::min(max_len, remaining.len());
                            chunks.push(remaining[..end].to_string());
                            remaining = &remaining[end..];
                        }
                    } else {
                        current = line.to_string();
                    }
                }
            }
        } else {
            // Start a new chunk with this paragraph.
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current = paragraph.to_string();
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

// ---------------------------------------------------------------------------
// Markdown -> WhatsApp formatting
// ---------------------------------------------------------------------------

/// Convert common Markdown constructs to WhatsApp-native formatting.
///
/// WhatsApp supports a small subset of formatting:
///   *bold*  _italic_  ~strikethrough~  ```code```
///
/// This function converts:
/// - `**text**` -> `*text*`  (Markdown bold to WhatsApp bold)
/// - `# heading` -> `*heading*` (headers rendered as bold)
/// - Leaves inline code, code blocks, and other text untouched.
fn format_for_whatsapp(text: &str) -> String {
    let normalized = normalize_markdown_for_whatsapp(text);
    let mut result = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // --- Bold: **text** -> *text* ---
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second '*'
                let mut inner = String::new();
                let mut closed = false;
                while let Some(&next) = chars.peek() {
                    if next == '*' {
                        chars.next();
                        if chars.peek() == Some(&'*') {
                            chars.next();
                            closed = true;
                            break;
                        }
                        inner.push('*');
                    } else {
                        inner.push(chars.next().unwrap());
                    }
                }
                if closed {
                    result.push('*');
                    result.push_str(&inner);
                    result.push('*');
                } else {
                    // Unclosed — emit literally.
                    result.push_str("**");
                    result.push_str(&inner);
                }
            }

            // --- Headers: # text -> *text* (bold) ---
            '#' if result.is_empty() || result.ends_with('\n') => {
                // Consume extra '#' symbols (##, ###, etc.)
                while chars.peek() == Some(&'#') {
                    chars.next();
                }
                // Skip optional space after '#'
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                // Collect header text until end-of-line or end-of-input.
                let mut header = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '\n' {
                        break;
                    }
                    header.push(chars.next().unwrap());
                }
                result.push('*');
                result.push_str(&header);
                result.push('*');
            }

            // --- Code blocks: ```lang\n...\n``` — keep as-is ---
            '`' if chars.peek() == Some(&'`') => {
                let mut backticks = String::from("`");
                while chars.peek() == Some(&'`') {
                    backticks.push(chars.next().unwrap());
                }
                if backticks.len() >= 3 {
                    // Pass through entire code block unchanged.
                    result.push_str(&backticks);
                    let mut consecutive_backticks = 0;
                    for ch in chars.by_ref() {
                        result.push(ch);
                        if ch == '`' {
                            consecutive_backticks += 1;
                            if consecutive_backticks >= 3 {
                                break;
                            }
                        } else {
                            consecutive_backticks = 0;
                        }
                    }
                } else {
                    result.push_str(&backticks);
                }
            }

            // --- Everything else: pass through ---
            _ => result.push(c),
        }
    }

    result
}

fn normalize_markdown_for_whatsapp(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(normalized.len());
    let mut in_code_block = false;

    for raw_line in normalized.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if is_whatsapp_markdown_rule(trimmed) {
            if !out.ends_with("\n\n") && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        let line = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .and_then(markdown_wrapped_bold_for_whatsapp)
            .map(|heading| format!("**{}**", heading))
            .unwrap_or_else(|| line.to_string());
        out.push_str(&line);
        out.push('\n');
    }

    out.trim_end().to_string()
}

fn markdown_wrapped_bold_for_whatsapp(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("**")
        .and_then(|value| value.strip_suffix("**"))
        .or_else(|| {
            trimmed
                .strip_prefix("__")
                .and_then(|value| value.strip_suffix("__"))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_whatsapp_markdown_rule(line: &str) -> bool {
    line.len() >= 3
        && line
            .chars()
            .all(|c| c == '-' || c == '*' || c == '_' || c.is_whitespace())
        && line
            .chars()
            .filter(|c| matches!(c, '-' | '*' | '_'))
            .count()
            >= 3
}

// ---------------------------------------------------------------------------
// Low-level HTTP helpers
// ---------------------------------------------------------------------------

/// Build an authenticated `reqwest::Client` with a JSON content-type default.
fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Send a text message to a WhatsApp recipient.
///
/// Calls `POST /{phone_number_id}/messages` with the standard text payload.
/// Long messages are automatically split into multiple chunks.
async fn send_whatsapp_text(config: &WhatsAppChannelConfig, to: &str, text: &str) -> Result<()> {
    let formatted = format_for_whatsapp(text);
    let chunks = super::outbound_split::split_outbound_message(
        &formatted,
        super::outbound_split::SplitProfile::provider_safe(MAX_MESSAGE_LEN),
    );

    let client = http_client();
    let url = format!("{}/{}/messages", API_BASE, config.phone_number_id);

    for chunk in &chunks {
        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": {
                "body": chunk
            }
        });

        let resp = super::outbound_rate_limit::send_with_bounded_retries(
            "whatsapp",
            "cloud_message",
            client
                .post(&url)
                .header("Authorization", format!("Bearer {}", config.access_token))
                .header("Content-Type", "application/json")
                .json(&body),
        )
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_body = resp.text().await.unwrap_or_default();
            tracing::error!("WhatsApp API error ({}): {}", status, error_body);
            return Err(anyhow!("WhatsApp API returned {} — {}", status, error_body));
        }
    }

    if chunks.len() > 1 {
        tracing::debug!(
            "WhatsApp: sent message in {} chunks to {}",
            chunks.len(),
            to
        );
    }

    Ok(())
}

/// Mark an inbound message as "read" (blue ticks) so the sender knows we
/// have processed it.
///
/// Calls `POST /{phone_number_id}/messages` with a status payload.
async fn mark_as_read(config: &WhatsAppChannelConfig, message_id: &str) -> Result<()> {
    let client = http_client();
    let url = format!("{}/{}/messages", API_BASE, config.phone_number_id);

    let body = serde_json::json!({
        "messaging_product": "whatsapp",
        "status": "read",
        "message_id": message_id
    });

    let resp = super::outbound_rate_limit::send_with_bounded_retries(
        "whatsapp",
        "mark_read",
        client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.access_token))
            .header("Content-Type", "application/json")
            .json(&body),
    )
    .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_body = resp.text().await.unwrap_or_default();
        tracing::warn!("WhatsApp mark-as-read failed ({}): {}", status, error_body);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public: push notification (send_message)
// ---------------------------------------------------------------------------

/// Send a push notification to the last known WhatsApp sender.
///
/// This mirrors the Telegram `send_message` helper — it looks up the most
/// Sends a proactive text message to the configured WhatsApp notification target.
///
/// Routes through the Baileys bridge or Meta Cloud API depending on config mode.
pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let Some(config) = &agent.config.whatsapp else {
        let message = "WhatsApp is not configured";
        tracing::warn!("WhatsApp send_message: {}", message);
        return Err(anyhow!(message));
    };

    let Some(phone_number) = configured_notification_recipient(config) else {
        let message = "WhatsApp proactive delivery is fail-closed until exactly one allowed number is configured.";
        tracing::warn!("WhatsApp send_message: {}", message);
        return Err(anyhow!(message));
    };

    send_message_to_recipient(config, &phone_number, &agent.config.name, text).await
}

pub async fn send_message_to_recipient(
    config: &WhatsAppChannelConfig,
    phone_number: &str,
    agent_name: &str,
    text: &str,
) -> Result<()> {
    if phone_number.trim().is_empty() || text.trim().is_empty() {
        return Ok(());
    }

    // Prefix with agent name so WhatsApp recipients know who's messaging
    let prefix = format!("[{}] ", agent_name);
    let prefixed_text =
        if text.starts_with(&prefix) || text.starts_with(&format!("[{}]", agent_name)) {
            text.to_string()
        } else {
            format!("{}{}", prefix, text)
        };

    match config.mode {
        WhatsAppMode::Baileys => send_via_bridge(config, phone_number, &prefixed_text).await,
        WhatsAppMode::CloudApi => send_whatsapp_text(config, phone_number, &prefixed_text).await,
    }
}

/// Send a message through the Baileys bridge sidecar.
async fn send_via_bridge(config: &WhatsAppChannelConfig, to: &str, text: &str) -> Result<()> {
    let formatted = format_for_whatsapp(text);
    let client = http_client();
    let url = format!("{}/send", config.effective_bridge_url()?);

    for formatted in super::outbound_split::split_for_provider_safe_channel("whatsapp", &formatted)
    {
        let body = serde_json::json!({
            "to": to,
            "text": formatted
        });

        let mut request = client.post(&url).header("Content-Type", "application/json");
        if !config.bridge_token.trim().is_empty() {
            request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
        }
        let resp = super::outbound_rate_limit::send_with_bounded_retries(
            "whatsapp",
            "bridge_message",
            request.json(&body),
        )
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_body = resp.text().await.unwrap_or_default();
            tracing::error!("WhatsApp bridge send error ({}): {}", status, error_body);
            return Err(anyhow!(
                "WhatsApp bridge returned {} — {}",
                status,
                error_body
            ));
        }
    }

    Ok(())
}

/// Send a presence update (e.g. "composing" or "paused") via the Baileys bridge.
async fn send_presence(
    config: &WhatsAppChannelConfig,
    to: &str,
    presence_type: &str,
) -> Result<()> {
    let client = http_client();
    let url = format!("{}/presence", config.effective_bridge_url()?);

    let body = serde_json::json!({
        "to": to,
        "type": presence_type
    });

    let mut request = client.post(&url).header("Content-Type", "application/json");
    if !config.bridge_token.trim().is_empty() {
        request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
    }
    let resp = super::outbound_rate_limit::send_with_bounded_retries(
        "whatsapp",
        "bridge_presence",
        request.json(&body),
    )
    .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_body = resp.text().await.unwrap_or_default();
        tracing::warn!(
            "WhatsApp bridge presence error ({}): {}",
            status,
            error_body
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Webhook verification (HTTP GET)
// ---------------------------------------------------------------------------

/// Verify the webhook endpoint during Meta's initial handshake.
///
/// Meta sends an HTTP GET with query parameters:
///   - `hub.mode` — must be `"subscribe"`
///   - `hub.verify_token` — must match the configured `verify_token`
///   - `hub.challenge` — an opaque string that must be echoed back
///
/// Returns the challenge string on success, or an error on mismatch.
pub async fn verify_webhook(
    query_params: &HashMap<String, String>,
    verify_token: &str,
) -> Result<String> {
    let mode = query_params
        .get("hub.mode")
        .ok_or_else(|| anyhow!("Missing hub.mode parameter"))?;

    if mode != "subscribe" {
        return Err(anyhow!(
            "Invalid hub.mode: expected 'subscribe', got '{}'",
            mode
        ));
    }

    let token = query_params
        .get("hub.verify_token")
        .ok_or_else(|| anyhow!("Missing hub.verify_token parameter"))?;

    if !crate::security::constant_time_eq(token.as_bytes(), verify_token.as_bytes()) {
        return Err(anyhow!("Verify token mismatch"));
    }

    let challenge = query_params
        .get("hub.challenge")
        .ok_or_else(|| anyhow!("Missing hub.challenge parameter"))?;

    tracing::info!("WhatsApp webhook verified successfully");
    Ok(challenge.clone())
}

// ---------------------------------------------------------------------------
// Webhook message handler (HTTP POST)
// ---------------------------------------------------------------------------

/// Handle an inbound webhook POST from the WhatsApp Business Cloud API.
///
/// The payload structure (simplified):
/// ```json
/// {
///   "entry": [{
///     "changes": [{
///       "value": {
///         "messages": [{
///           "from": "15551234567",
///           "id": "wamid.xxx",
///           "type": "text",
///           "text": { "body": "Hello!" }
///         }]
///       }
///     }]
///   }]
/// }
/// ```
///
/// Returns `"ok"` on success (Meta expects a 200 response quickly).
pub async fn handle_webhook(agent: SharedAgent, body: &serde_json::Value) -> Result<String> {
    let config = {
        let agent_read = agent.read().await;
        agent_read
            .config
            .whatsapp
            .clone()
            .ok_or_else(|| anyhow!("WhatsApp is not configured"))?
    };
    handle_webhook_with_config(agent, &config, body).await
}

pub async fn handle_webhook_with_config(
    agent: SharedAgent,
    config: &WhatsAppChannelConfig,
    body: &serde_json::Value,
) -> Result<String> {
    let is_baileys = body.get("_source").and_then(|value| value.as_str()) == Some("baileys");
    let config = config.clone();
    match (is_baileys, config.mode) {
        (true, WhatsAppMode::Baileys) | (false, WhatsAppMode::CloudApi) => {}
        (true, _) => {
            return Err(anyhow!(
                "WhatsApp bridge payload rejected because the channel is not in Baileys mode"
            ));
        }
        (false, _) => {
            return Err(anyhow!(
                "WhatsApp Cloud API payload rejected because the channel is not in Cloud API mode"
            ));
        }
    }

    // Navigate to the first message in the payload.
    let message = body
        .get("entry")
        .and_then(|e| e.get(0))
        .and_then(|e| e.get("changes"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("value"))
        .and_then(|v| v.get("messages"))
        .and_then(|m| m.get(0));

    let Some(message) = message else {
        // Not every webhook event contains a message (e.g. status updates).
        tracing::debug!("WhatsApp webhook: no message in payload, ignoring");
        return Ok("ok".to_string());
    };

    // Extract core fields.
    let from = message
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let message_id = message
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let msg_type = message
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if from.is_empty() {
        tracing::warn!("WhatsApp webhook: message has no sender, ignoring");
        return Ok("ok".to_string());
    }

    if !message_id.is_empty() {
        let storage = {
            let agent_read = agent.read().await;
            agent_read.storage.clone()
        };
        if record_whatsapp_message_id(&storage, message_id).await? {
            tracing::debug!("WhatsApp: ignoring duplicate message {}", message_id);
            return Ok("ok".to_string());
        }
    }

    if !config.allowed_numbers.is_empty() && !config.allowed_numbers.iter().any(|n| n == from) {
        tracing::warn!(
            "WhatsApp: rejected message from unauthorized number {}",
            from
        );
        {
            let agent_read = agent.read().await;
            agent_read
                .security_events
                .record_unauthorized_channel_attempt();
        }
        return Ok("ok".to_string());
    }

    // ---- DM pairing: require explicit approval for unknown senders ----
    if config.dm_policy == "pairing" && !config.allowed_numbers.is_empty() {
        // Already checked above — only allowed numbers get through.
        // For "pairing" with empty allowed_numbers, check storage for approved senders.
    }
    if config.dm_policy == "pairing" && config.allowed_numbers.is_empty() {
        // Unknown senders are gated after text extraction so the approval request can
        // carry a useful preview for the operator.
    }

    // ---- Mark as read (fire and forget, Cloud API only — Baileys does it on bridge side) ----
    if !message_id.is_empty() && config.mode == WhatsAppMode::CloudApi {
        let config_clone = config.clone();
        let mid = message_id.to_string();
        crate::spawn_logged!("src/channels/whatsapp.rs:1116", async move {
            if let Err(e) = mark_as_read(&config_clone, &mid).await {
                tracing::warn!("Failed to mark WhatsApp message as read: {}", e);
            }
        });
    }

    // ---- Extract text content ----
    let text = match msg_type {
        "text" => message
            .get("text")
            .and_then(|t| t.get("body"))
            .and_then(|b| b.as_str())
            .unwrap_or_default()
            .to_string(),
        "image" | "video" | "audio" | "document" => {
            // Media messages include an optional caption.
            let caption = message
                .get(msg_type)
                .and_then(|m| m.get("caption"))
                .and_then(|c| c.as_str())
                .unwrap_or_default();
            if caption.is_empty() {
                format!("[Received {} message]", msg_type)
            } else {
                caption.to_string()
            }
        }
        "location" => {
            let lat = message
                .get("location")
                .and_then(|l| l.get("latitude"))
                .and_then(|v| v.as_f64())
                .unwrap_or_default();
            let lon = message
                .get("location")
                .and_then(|l| l.get("longitude"))
                .and_then(|v| v.as_f64())
                .unwrap_or_default();
            format!("[Location: {:.6}, {:.6}]", lat, lon)
        }
        "reaction" => {
            // Reactions don't need a reply.
            tracing::debug!("WhatsApp: received reaction from {}", from);
            return Ok("ok".to_string());
        }
        _ => {
            tracing::debug!(
                "WhatsApp: unsupported message type '{}' from {}",
                msg_type,
                from
            );
            let _ = send_reply(
                &config,
                from,
                "Sorry, I can only process text messages at the moment.",
            )
            .await;
            return Ok("ok".to_string());
        }
    };

    if text.is_empty() {
        return Ok("ok".to_string());
    }

    if config.dm_policy == "pairing" && config.allowed_numbers.is_empty() {
        let trust_decision = {
            let agent_read = agent.read().await;
            sender_verification::evaluate_sender_with_rules(
                &agent_read.storage,
                &whatsapp_sender_identity(from, Some(&text)),
                sender_verification::SenderTrustPolicy::Pairing,
                &[],
            )
            .await?
        };
        if let SenderTrustDecision::NeedsApproval { created_new, .. } = trust_decision {
            let code = format!(
                "{:06}",
                from.as_bytes().iter().map(|b| *b as u64).sum::<u64>() % 1000000
            );
            let pairing_msg = format!(
                "Hello! I'm {}.\n\nFor security, new contacts must be approved before I can act here.\nAsk the owner to approve this sender in Settings -> Connected Systems -> Sender Verification, or from a trusted WhatsApp chat run:\n\n_/approve {}_\n\nYour pairing code: *{}*",
                crate::branding::PRODUCT_NAME,
                from,
                code
            );
            let _ = send_reply(&config, from, &pairing_msg).await;
            let normalized_from = normalize_whatsapp_sender(from);
            {
                let agent_read = agent.read().await;
                let _ = agent_read
                    .storage
                    .set(
                        &format!("whatsapp:pairing:{}", normalized_from),
                        code.as_bytes(),
                    )
                    .await;
                if created_new {
                    let body = format!(
                        "A new WhatsApp sender needs approval before {} will act.\nSender: {}\nMessage: {}\nApprove it in Settings -> Connected Systems -> Sender Verification, or from a trusted WhatsApp chat run /approve {}.",
                        crate::branding::PRODUCT_NAME,
                        from,
                        text.chars().take(180).collect::<String>(),
                        from
                    );
                    agent_read
                        .emit_notification_forced(
                            "Sender Approval Needed",
                            &body,
                            "warning",
                            "sender_verification",
                        )
                        .await;
                }
            }
            tracing::info!(
                "WhatsApp: sent pairing request to {} (code: {})",
                from,
                code
            );
            return Ok("ok".to_string());
        }
    }

    // ---- Persist last sender for push notifications ----
    {
        let agent_read = agent.read().await;
        let normalized_sender = normalize_whatsapp_sender(from);
        let _ = agent_read
            .storage
            .set("whatsapp:last_sender", normalized_sender.as_bytes())
            .await;
    }

    let conversation_id = format!("whatsapp:{}", from);

    tracing::info!(
        "WhatsApp message from {} ({} type, {} chars)",
        from,
        msg_type,
        text.len()
    );

    // ---- Handle slash commands ----
    if text.starts_with('/') {
        let response = handle_command(&text, &agent, from).await;
        send_reply(&config, from, &response).await?;
        return Ok("ok".to_string());
    }

    // Explicit tunnel control commands are handled before LLM processing.
    if let Some(cmd) = parse_tunnel_command(&text) {
        let response = execute_tunnel_command(&agent, cmd).await;
        send_reply(&config, from, &response).await?;
        return Ok("ok".to_string());
    }

    // Internal escape hatch only. The product UX is the secure credential form.
    let can_store_secret = if !config.allowed_numbers.is_empty() {
        config.allowed_numbers.iter().any(|n| n == from)
    } else if config.dm_policy == "pairing" {
        let agent_read = agent.read().await;
        whatsapp_sender_is_approved(&agent_read.storage, from).await
    } else {
        false
    };

    if let Some((key, value)) = parse_set_secret(&text) {
        if !crate::core::secrets::setsecret_command_escape_hatch_enabled() {
            send_reply(
                &config,
                from,
                crate::core::secrets::setsecret_command_disabled_response(),
            )
            .await?;
            return Ok("ok".to_string());
        }
        if !can_store_secret {
            send_reply(
                &config,
                from,
                "Refusing to store secrets from this number. Pair/approve first (dm_policy=pairing) or add it to allowed_numbers in Settings.",
            )
            .await?;
            return Ok("ok".to_string());
        }
        let reply = match store_secret_for_chat(&agent, &key, &value).await {
            Ok(()) => {
                let conversation_id = format!("whatsapp:{}", from);
                let followup = {
                    let a = agent.read().await;
                    a.on_secret_saved_followup(&conversation_id).await
                };
                let mut response = format!(
                    "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
                    key
                );
                if let Some(f) = followup {
                    response.push_str("\n\n");
                    response.push_str(&f);
                }
                response
            }
            Err(e) => format!("Failed to store secret: {}", e),
        };
        send_reply(&config, from, &reply).await?;
        return Ok("ok".to_string());
    }

    if let Some(key) = parse_use_current_llm_key(&text) {
        if !crate::core::secrets::secret_command_escape_hatch_enabled() {
            send_reply(
                &config,
                from,
                crate::core::secrets::setsecret_command_disabled_response(),
            )
            .await?;
            return Ok("ok".to_string());
        }
        if !can_store_secret {
            send_reply(
                &config,
                from,
                "Refusing to store secrets from this number. Pair/approve first (dm_policy=pairing) or add it to allowed_numbers in Settings.",
            )
            .await?;
            return Ok("ok".to_string());
        }
        let reply = match link_current_llm_key_for_chat(&agent, &key).await {
            Ok(prefix) => {
                let conversation_id = format!("whatsapp:{}", from);
                let followup = {
                    let a = agent.read().await;
                    a.on_secret_saved_followup(&conversation_id).await
                };
                let mut response = prefix;
                if let Some(f) = followup {
                    response.push_str("\n\n");
                    response.push_str(&f);
                }
                response
            }
            Err(e) => e,
        };
        send_reply(&config, from, &reply).await?;
        return Ok("ok".to_string());
    }

    // ---- Show "composing..." typing indicator ----
    let _ = send_presence(&config, from, "composing").await;

    // Keep typing indicator alive in background (WhatsApp composing expires after ~10s)
    let typing_config = config.clone();
    let typing_from = from.to_string();
    let typing_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let typing_flag = typing_done.clone();
    crate::spawn_logged!("src/channels/whatsapp.rs:1356", async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            if typing_flag.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let _ = send_presence(&typing_config, &typing_from, "composing").await;
        }
    });

    // ---- Process via agent ----
    let response = {
        let agent_snapshot = Agent::snapshot(&agent).await;
        match agent_snapshot
            .process_message_with_meta(&text, "whatsapp", Some(&conversation_id), None)
            .await
        {
            Ok(processed) => Agent::render_plain_channel_response(processed),
            Err(e) => {
                tracing::error!("WhatsApp agent processing error: {}", e);
                format!("I encountered an error processing your message: {}", e)
            }
        }
    };
    typing_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = send_presence(&config, from, "paused").await;

    // ---- Send reply ----
    send_reply(&config, from, &response).await?;

    Ok("ok".to_string())
}

/// Send a video to the configured WhatsApp notification target.
///
/// For Baileys mode: sends base64-encoded video to the bridge /send-video endpoint.
/// For Cloud API mode: sends a download link (WhatsApp Cloud API requires media upload).
pub async fn send_video(
    agent: &Agent,
    video_bytes: &[u8],
    caption: &str,
    download_url: Option<&str>,
) -> Result<()> {
    let Some(config) = &agent.config.whatsapp else {
        return Ok(());
    };

    let Some(phone_number) = configured_notification_recipient(config) else {
        return Ok(());
    };

    match config.mode {
        WhatsAppMode::Baileys => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(video_bytes);
            let client = http_client();
            let url = format!("{}/send-video", config.effective_bridge_url()?);
            let body = serde_json::json!({
                "to": phone_number,
                "video": b64,
                "caption": caption,
            });
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !config.bridge_token.trim().is_empty() {
                request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
            }
            let resp = super::outbound_rate_limit::send_with_bounded_retries(
                "whatsapp",
                "bridge_video",
                request,
            )
            .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let error_body = resp.text().await.unwrap_or_default();
                tracing::error!(
                    "WhatsApp bridge send-video error ({}): {}",
                    status,
                    error_body
                );
                return Err(anyhow!(
                    "WhatsApp bridge returned {} — {}",
                    status,
                    error_body
                ));
            }
            Ok(())
        }
        WhatsAppMode::CloudApi => {
            // Cloud API requires media upload — fall back to sending a download link
            let link = download_url.unwrap_or("Video available, but no public download URL.");
            let link_msg = format!("{}\n\nDownload: {}", caption, link);
            send_whatsapp_text(config, &phone_number, &link_msg).await
        }
    }
}

/// Send an image preview to the configured WhatsApp notification target.
///
/// For Baileys mode: sends base64-encoded image to the bridge /send-image endpoint.
/// For Cloud API mode: sends caption + preview link text (no media upload path here yet).
pub async fn send_image(
    agent: &Agent,
    image_bytes: &[u8],
    caption: &str,
    image_url: Option<&str>,
) -> Result<()> {
    let Some(config) = &agent.config.whatsapp else {
        return Ok(());
    };

    let Some(phone_number) = configured_notification_recipient(config) else {
        return Ok(());
    };

    match config.mode {
        WhatsAppMode::Baileys => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
            let client = http_client();
            let url = format!("{}/send-image", config.effective_bridge_url()?);
            let body = serde_json::json!({
                "to": phone_number,
                "image": b64,
                "caption": caption,
            });
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !config.bridge_token.trim().is_empty() {
                request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
            }
            let resp = super::outbound_rate_limit::send_with_bounded_retries(
                "whatsapp",
                "bridge_image",
                request,
            )
            .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let error_body = resp.text().await.unwrap_or_default();
                tracing::error!(
                    "WhatsApp bridge send-image error ({}): {}",
                    status,
                    error_body
                );
                return Err(anyhow!(
                    "WhatsApp bridge returned {} - {}",
                    status,
                    error_body
                ));
            }
            Ok(())
        }
        WhatsAppMode::CloudApi => {
            let link = image_url.unwrap_or("Preview image generated.");
            let link_msg = format!("{}\n\nPreview: {}", caption, link);
            send_whatsapp_text(config, &phone_number, &link_msg).await
        }
    }
}

/// Route a reply through the correct backend (Baileys bridge or Cloud API).
async fn send_reply(config: &WhatsAppChannelConfig, to: &str, text: &str) -> Result<()> {
    match config.mode {
        WhatsAppMode::Baileys => send_via_bridge(config, to, text).await,
        WhatsAppMode::CloudApi => send_whatsapp_text(config, to, text).await,
    }
}

// ---------------------------------------------------------------------------
// Slash command handler
// ---------------------------------------------------------------------------

/// Handle `/` commands sent via WhatsApp.
///
/// Supports a subset of the Telegram commands adapted for WhatsApp's context.
async fn handle_command(text: &str, agent: &SharedAgent, from: &str) -> String {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let command = parts.first().copied().unwrap_or("");
    let args = parts.get(1).map(|s| s.trim()).unwrap_or("");
    let conversation_id = format!("whatsapp:{}", from);

    match command {
        "/start" | "/help" => {
            let agent = agent.read().await;
            format!(
                "*Welcome to {}!*\n\n\
                 Available commands:\n\n\
                 /help - Show this help message\n\
                 /status - Agent status\n\
                 /skills - List available skills\n\
                 /install <url> - Install a skill from URL\n\
                 /run <skill> [query] - Run a custom skill\n\
                 /tasks - View pending tasks\n\
                 /approve-task <task_id> - Approve a waiting task\n\
                 /reject-task <task_id> - Reject a waiting task\n\
                 /search <query> - Web search\n\
                 /image <prompt> - Generate an image\n\
                 /tunnel [start|stop|status] - Manage remote UI access\n\
                 /approve <number> - Approve a contact\n\
                 /new - Start a new conversation\n\
                 /clear - Clear conversation history\n\n\
                 Or just send me a message!",
                agent.config.name
            )
        }

        "/status" => {
            let agent = agent.read().await;
            let status = agent.status().await;
            format!(
                "*Agent Status*\n\n\
                 DID: {}\n\
                 Memory: {} entries\n\
                 Skills: {} loaded\n\
                 Tasks: {} pending",
                status.did, status.memory_entries, status.actions_loaded, status.tasks_pending
            )
        }

        "/tunnel" => {
            let cmd = if args.is_empty() {
                TunnelControlCommand::Status
            } else if args.eq_ignore_ascii_case("start") {
                TunnelControlCommand::Start
            } else if args.eq_ignore_ascii_case("stop") {
                TunnelControlCommand::Stop
            } else if args.eq_ignore_ascii_case("status") {
                TunnelControlCommand::Status
            } else {
                return "Usage: /tunnel [start|stop|status]\nExample: /tunnel start".to_string();
            };
            execute_tunnel_command(agent, cmd).await
        }

        "/skills" | "/skill" => {
            let agent = agent.read().await;
            let actions = agent.runtime.list_actions().await.unwrap_or_default();
            if actions.is_empty() {
                "No skills loaded.".to_string()
            } else {
                let list = actions
                    .iter()
                    .take(15)
                    .map(|s| format!("- {} - {}", s.name, s.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                let more = if actions.len() > 15 {
                    format!("\n\n...and {} more", actions.len() - 15)
                } else {
                    String::new()
                };
                format!("*Available Skills*\n\n{}{}", list, more)
            }
        }

        "/install" => {
            if args.is_empty() {
                "Usage: /install <skill_url>".to_string()
            } else {
                let prompt = format!("install this skill {}", args.trim());
                process_whatsapp_prompt(agent, &prompt, &conversation_id).await
            }
        }

        "/run" => {
            let rest = args.trim();
            if rest.is_empty() {
                "Usage: /run <skill_name> [query]".to_string()
            } else {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let skill_name = parts.next().unwrap_or("").trim();
                let query = parts.next().unwrap_or("").trim();
                let prompt = if query.is_empty() {
                    format!("run {}", skill_name)
                } else {
                    format!("run {} {}", skill_name, query)
                };
                process_whatsapp_prompt(agent, &prompt, &conversation_id).await
            }
        }

        "/setsecret" => {
            if !crate::core::secrets::setsecret_command_escape_hatch_enabled() {
                return crate::core::secrets::setsecret_command_disabled_response().to_string();
            }
            // Security gate: only allow in "pairing" mode with approval or explicit allowlist.
            // Do not accept secrets in open mode.
            let (cfg_opt, storage) = {
                let a = agent.read().await;
                (a.config.whatsapp.clone(), a.storage.clone())
            };
            let Some(cfg) = cfg_opt else {
                return "WhatsApp channel not configured. Use the web UI instead.".to_string();
            };

            let allowlisted =
                !cfg.allowed_numbers.is_empty() && cfg.allowed_numbers.iter().any(|n| n == from);
            let approved = if cfg.dm_policy == "pairing" {
                whatsapp_sender_is_approved(&storage, from).await
            } else {
                false
            };

            if !allowlisted && !approved {
                return "Refusing to store secrets from this number. Pair/approve first (dm_policy=pairing) or add it to allowed_numbers in Settings.".to_string();
            }

            if args.is_empty() {
                return "Internal credential command requires KEY=VALUE.".to_string();
            }
            let input = format!("/setsecret {}", args);
            let Some((key, value)) = parse_set_secret(&input) else {
                return "Internal credential command requires KEY=VALUE.".to_string();
            };

            match store_secret_for_chat(agent, &key, &value).await {
                Ok(()) => {
                    let conversation_id = format!("whatsapp:{}", from);
                    let followup = {
                        let a = agent.read().await;
                        a.on_secret_saved_followup(&conversation_id).await
                    };
                    let mut response = format!(
                        "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
                        key
                    );
                    if let Some(f) = followup {
                        response.push_str("\n\n");
                        response.push_str(&f);
                    }
                    response
                }
                Err(e) => format!("Failed to store secret: {}", e),
            }
        }
        "/usecurrentkey" => {
            if !crate::core::secrets::secret_command_escape_hatch_enabled() {
                return crate::core::secrets::setsecret_command_disabled_response().to_string();
            }
            let (cfg_opt, storage) = {
                let a = agent.read().await;
                (a.config.whatsapp.clone(), a.storage.clone())
            };
            let Some(cfg) = cfg_opt else {
                return "WhatsApp channel not configured. Use the web UI instead.".to_string();
            };

            let allowlisted =
                !cfg.allowed_numbers.is_empty() && cfg.allowed_numbers.iter().any(|n| n == from);
            let approved = if cfg.dm_policy == "pairing" {
                whatsapp_sender_is_approved(&storage, from).await
            } else {
                false
            };

            if !allowlisted && !approved {
                return "Refusing to store secrets from this number. Pair/approve first (dm_policy=pairing) or add it to allowed_numbers in Settings.".to_string();
            }

            if args.is_empty() {
                return "Internal credential command requires KEY.".to_string();
            }
            let input = format!("/usecurrentkey {}", args);
            let Some(key) = parse_use_current_llm_key(&input) else {
                return "Internal credential command requires KEY.".to_string();
            };

            match link_current_llm_key_for_chat(agent, &key).await {
                Ok(prefix) => {
                    let conversation_id = format!("whatsapp:{}", from);
                    let followup = {
                        let a = agent.read().await;
                        a.on_secret_saved_followup(&conversation_id).await
                    };
                    let mut response = prefix;
                    if let Some(f) = followup {
                        response.push_str("\n\n");
                        response.push_str(&f);
                    }
                    response
                }
                Err(e) => e,
            }
        }

        "/tasks" => {
            let agent = agent.read().await;
            let tasks = agent.tasks.read().await;
            let pending: Vec<_> = tasks
                .all()
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::AwaitingApproval))
                .take(10)
                .collect();

            if pending.is_empty() {
                "No pending tasks.".to_string()
            } else {
                let list = pending
                    .iter()
                    .map(|t| {
                        let marker = match t.status {
                            TaskStatus::AwaitingApproval => "[awaiting]",
                            TaskStatus::Pending => "[pending]",
                            _ => "[-]",
                        };
                        if matches!(t.status, TaskStatus::AwaitingApproval) {
                            format!("{} {} [{}]", marker, t.description, t.id)
                        } else {
                            format!("{} {}", marker, t.description)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("*Pending Tasks*\n\n{}", list)
            }
        }

        "/approve-task" => {
            if args.is_empty() {
                "Usage: /approve-task <task_id>".to_string()
            } else {
                let Ok(task_id) = uuid::Uuid::parse_str(args) else {
                    return "Invalid task id. Use the full task UUID shown in /tasks or the approval notification.".to_string();
                };
                let agent = agent.read().await;
                match agent.approve_task_request(task_id, "whatsapp").await {
                    Ok(Some(task)) => format!("Approved: {}", task.description),
                    Ok(None) => "Task not found or is not awaiting approval.".to_string(),
                    Err(e) => format!("Failed to approve task: {}", e),
                }
            }
        }

        "/reject-task" => {
            if args.is_empty() {
                "Usage: /reject-task <task_id>".to_string()
            } else {
                let Ok(task_id) = uuid::Uuid::parse_str(args) else {
                    return "Invalid task id. Use the full task UUID shown in /tasks or the approval notification.".to_string();
                };
                let agent = agent.read().await;
                match agent
                    .reject_task_request(
                        task_id,
                        "whatsapp",
                        "Task was rejected from WhatsApp and will not be executed.",
                    )
                    .await
                {
                    Ok(Some(task)) => format!("Rejected: {}", task.description),
                    Ok(None) => "Task not found or is not awaiting approval.".to_string(),
                    Err(e) => format!("Failed to reject task: {}", e),
                }
            }
        }

        "/search" => {
            if args.is_empty() {
                "Usage: /search <query>\n\nExample: /search latest news about AI".to_string()
            } else {
                let prompt = format!("Search the web for: {}", args);
                let response = process_whatsapp_prompt(agent, &prompt, &conversation_id).await;
                response
            }
        }

        "/image" => {
            if args.is_empty() {
                "Usage: /image <prompt>\n\nExample: /image a cute robot playing guitar".to_string()
            } else {
                let prompt = format!("Generate an image of: {}", args);
                let response = process_whatsapp_prompt(agent, &prompt, &conversation_id).await;
                response
            }
        }

        "/new" => {
            let agent = agent.read().await;
            match agent
                .start_new_channel_conversation("whatsapp", &conversation_id, None, "New Chat")
                .await
            {
                Ok(_) => "Started a new conversation. Previous history is kept.".to_string(),
                Err(e) => format!("Failed to start a new conversation: {}", e),
            }
        }

        "/clear" => {
            let agent = agent.read().await;
            match agent
                .clear_current_channel_conversation("whatsapp", &conversation_id, None)
                .await
            {
                Ok(_) => "Conversation cleared. Starting fresh.".to_string(),
                Err(e) => format!("Failed to clear conversation: {}", e),
            }
        }

        "/approve" => {
            if args.is_empty() {
                "Usage: /approve <phone_number>\n\nExample: /approve 15551234567".to_string()
            } else {
                let number = args.trim();
                let agent = agent.read().await;
                match approve_whatsapp_sender(&agent.storage, number, "whatsapp_command").await {
                    Ok(_) => {
                        format!("Approved {}. They can now chat with the agent.", number)
                    }
                    Err(e) => format!("Failed to approve: {}", e),
                }
            }
        }

        _ => format!(
            "Unknown command: {}\n\nType /help for available commands.",
            command
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::outbound_split::{SplitProfile, split_outbound_message};

    #[test]
    fn test_split_message_short() {
        let text = "Hello, world!";
        let chunks = split_outbound_message(text, SplitProfile::provider_safe(MAX_MESSAGE_LEN));
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_split_message_at_paragraph() {
        let a = "A".repeat(2000);
        let b = "B".repeat(2000);
        let text = format!("{}\n\n{}", a, b);
        let chunks = split_outbound_message(&text, SplitProfile::provider_safe(2500));
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], a);
        assert_eq!(chunks[1], b);
    }

    #[test]
    fn test_format_bold() {
        assert_eq!(format_for_whatsapp("**hello**"), "*hello*");
    }

    #[test]
    fn test_format_header() {
        assert_eq!(format_for_whatsapp("# Title"), "*Title*");
        assert_eq!(format_for_whatsapp("## Subtitle"), "*Subtitle*");
    }

    #[test]
    fn test_format_code_block_preserved() {
        let input = "```rust\nfn main() {}\n```";
        let output = format_for_whatsapp(input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_format_plain_text_unchanged() {
        let input = "Just some plain text with no markdown.";
        assert_eq!(format_for_whatsapp(input), input);
    }

    #[tokio::test]
    async fn test_verify_webhook_success() {
        let mut params = HashMap::new();
        params.insert("hub.mode".to_string(), "subscribe".to_string());
        params.insert("hub.verify_token".to_string(), "my_token".to_string());
        params.insert("hub.challenge".to_string(), "challenge_123".to_string());

        let result = verify_webhook(&params, "my_token").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "challenge_123");
    }

    #[tokio::test]
    async fn test_verify_webhook_bad_token() {
        let mut params = HashMap::new();
        params.insert("hub.mode".to_string(), "subscribe".to_string());
        params.insert("hub.verify_token".to_string(), "wrong_token".to_string());
        params.insert("hub.challenge".to_string(), "challenge_123".to_string());

        let result = verify_webhook(&params, "my_token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_webhook_missing_mode() {
        let mut params = HashMap::new();
        params.insert("hub.verify_token".to_string(), "my_token".to_string());
        params.insert("hub.challenge".to_string(), "challenge_123".to_string());

        let result = verify_webhook(&params, "my_token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_webhook_wrong_mode() {
        let mut params = HashMap::new();
        params.insert("hub.mode".to_string(), "unsubscribe".to_string());
        params.insert("hub.verify_token".to_string(), "my_token".to_string());
        params.insert("hub.challenge".to_string(), "challenge_123".to_string());

        let result = verify_webhook(&params, "my_token").await;
        assert!(result.is_err());
    }

    fn test_cloud_config() -> WhatsAppChannelConfig {
        WhatsAppChannelConfig {
            mode: WhatsAppMode::CloudApi,
            access_token: "token".to_string(),
            phone_number_id: "phone-id".to_string(),
            app_secret: "topsecret".to_string(),
            verify_token: "verify".to_string(),
            bridge_runtime: Some(WhatsAppBridgeRuntime::Embedded),
            bridge_url: default_bridge_url(),
            bridge_token: String::new(),
            allowed_numbers: vec![],
            dm_policy: default_dm_policy(),
        }
    }

    #[test]
    fn infer_bridge_runtime_defaults_to_embedded_for_loopback() {
        assert_eq!(
            infer_bridge_runtime_from_url("http://127.0.0.1:8999"),
            WhatsAppBridgeRuntime::Embedded
        );
        assert_eq!(
            infer_bridge_runtime_from_url("http://localhost:8999"),
            WhatsAppBridgeRuntime::Embedded
        );
    }

    #[test]
    fn infer_bridge_runtime_uses_external_for_non_loopback_urls() {
        assert_eq!(
            infer_bridge_runtime_from_url("https://bridge.example.com"),
            WhatsAppBridgeRuntime::External
        );
    }

    #[test]
    fn configured_notification_recipient_requires_exactly_one_allowed_number() {
        let mut config = test_cloud_config();
        config.allowed_numbers = vec!["+1 (555) 123-4567".to_string()];
        assert_eq!(
            configured_notification_recipient(&config),
            Some("15551234567".to_string())
        );

        config.allowed_numbers.clear();
        assert_eq!(configured_notification_recipient(&config), None);

        config.allowed_numbers = vec!["15551234567".to_string(), "15557654321".to_string()];
        assert_eq!(configured_notification_recipient(&config), None);
    }

    #[test]
    fn test_verify_cloud_api_request_signature_success() {
        let raw_body = br#"{"entry":[{"changes":[{"value":{"messages":[]}}]}]}"#;
        let config = test_cloud_config();
        let signature = format!("sha256={}", hmac_sha256_hex(&config.app_secret, raw_body));
        assert!(verify_cloud_api_request_signature(&config, raw_body, Some(&signature)).is_ok());
    }

    #[test]
    fn test_verify_cloud_api_request_signature_rejects_missing_secret() {
        let raw_body = br#"{}"#;
        let mut config = test_cloud_config();
        config.app_secret.clear();
        let error = verify_cloud_api_request_signature(&config, raw_body, Some("sha256=abc"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("app secret"));
    }

    #[test]
    fn test_verify_cloud_api_request_signature_rejects_tampered_payload() {
        let raw_body = br#"{"entry":[{"changes":[{"value":{"messages":[]}}]}]}"#;
        let tampered_body = br#"{"entry":[{"changes":[{"value":{"messages":[{"id":"1"}]}}]}]}"#;
        let config = test_cloud_config();
        let signature = format!("sha256={}", hmac_sha256_hex(&config.app_secret, raw_body));
        assert!(
            verify_cloud_api_request_signature(&config, tampered_body, Some(&signature)).is_err()
        );
    }

    #[test]
    fn recent_message_state_is_pruned_to_a_bounded_window() {
        let mut state = WhatsAppRecentMessageState {
            recent: (0..(MAX_RECENT_MESSAGE_IDS + 10))
                .map(|idx| WhatsAppRecentMessageEntry {
                    message_id: format!("wamid.{}", idx),
                    seen_at: 1,
                })
                .collect(),
        };
        prune_recent_message_state(&mut state, RECENT_MESSAGE_ID_WINDOW_SECS + 2);
        assert!(state.recent.len() <= MAX_RECENT_MESSAGE_IDS);
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn record_message_id_is_idempotent_for_retries() {
        let _dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();
        assert!(
            !record_whatsapp_message_id(&storage, "wamid.1")
                .await
                .unwrap()
        );
        assert!(
            record_whatsapp_message_id(&storage, "wamid.1")
                .await
                .unwrap()
        );
    }
}
