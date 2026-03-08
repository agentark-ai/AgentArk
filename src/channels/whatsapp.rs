//! WhatsApp Business Cloud API channel
//!
//! Integrates with the Meta WhatsApp Business Cloud API (v18.0) to provide
//! bidirectional messaging. Supports webhook verification, inbound message
//! handling, outbound text replies, push notifications, and slash commands.
//!
//! API reference: https://developers.facebook.com/docs/whatsapp/cloud-api

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::{Agent, TaskStatus};

type SharedAgent = Arc<RwLock<Agent>>;

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
        "start tunnel" | "/tunnel start" | "/start_tunnel" => Some(TunnelControlCommand::Start),
        "stop tunnel" | "/tunnel stop" | "/stop_tunnel" => Some(TunnelControlCommand::Stop),
        "tunnel status" | "status tunnel" | "/tunnel" | "/tunnel status" | "/tunnel_status" => {
            Some(TunnelControlCommand::Status)
        }
        _ => None,
    }
}

fn internal_api_base_url() -> String {
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    let tls_enabled = std::env::var("AGENTARK_TLS_CERT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
        && std::env::var("AGENTARK_TLS_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
    let scheme = if tls_enabled { "https" } else { "http" };
    format!("{}://{}", scheme, bind_addr)
}

async fn execute_tunnel_command(agent: &SharedAgent, cmd: TunnelControlCommand) -> String {
    let api_key = { agent.read().await.api_key.clone() };
    let base_url = internal_api_base_url();
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
    {
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

/// Base URL for the Meta WhatsApp Business Cloud API (v18.0).
const API_BASE: &str = "https://graph.facebook.com/v18.0";

/// Maximum text message length supported by WhatsApp before truncation.
/// The actual limit is 4096 characters for text body messages.
const MAX_MESSAGE_LEN: usize = 4096;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Connection mode: Baileys (QR scan) or Meta Business Cloud API.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppMode {
    /// Baileys bridge — scan QR code, no Meta account needed.
    #[default]
    Baileys,
    /// Meta Business Cloud API — production-grade, requires Business account.
    CloudApi,
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

    /// Token used to verify the webhook endpoint during initial setup.
    #[serde(default)]
    pub verify_token: String,

    // ---- Baileys bridge fields ----
    /// URL of the embedded Baileys bridge (default: http://127.0.0.1:8999).
    #[serde(default = "default_bridge_url")]
    pub bridge_url: String,

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
    "http://127.0.0.1:8999".to_string()
}

fn default_dm_policy() -> String {
    "pairing".to_string()
}

fn parse_set_secret(text: &str) -> Option<(String, String)> {
    // Accept both:
    // - "/setsecret KEY=VALUE" (WhatsApp slash command)
    // - "set secret KEY=VALUE" (plain text; mainly for parity)
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = if lower.starts_with("/setsecret ") || lower.starts_with("set secret ") {
        trimmed[10..].trim() // len("set secret ") == 10
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }

    let (key, value) = if let Some(eq) = rest.find('=') {
        let (k, v) = rest.split_at(eq);
        (k.trim(), v[1..].trim())
    } else {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let k = parts.next().unwrap_or("").trim();
        let v = parts.next().unwrap_or("").trim();
        (k, v)
    };

    if key.is_empty() || value.is_empty() {
        return None;
    }
    if key.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    if key.contains('\n') || key.contains('\r') {
        return None;
    }

    Some((key.to_string(), value.to_string()))
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
        "I can't map '{}' from current model settings. Available model-backed keys: {}. You can set it manually with: set secret {}=VALUE",
        key, available, key
    ))
}

// ---------------------------------------------------------------------------
// Message splitting
// ---------------------------------------------------------------------------

/// Split a message into chunks that fit within the WhatsApp text limit.
///
/// Tries to split at paragraph boundaries (`\n\n`) first, then falls back to
/// line boundaries (`\n`), so the recipient sees coherent chunks.
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
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

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
    let chunks = split_message(&formatted, MAX_MESSAGE_LEN);

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

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
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

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.access_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
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
/// recent chat partner from storage and delivers a text message. Useful for
/// scheduled task results, alerts, reminders, etc.
///
/// Routes through the Baileys bridge or Meta Cloud API depending on config mode.
pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let Some(config) = &agent.config.whatsapp else {
        return Ok(());
    };

    let phone_bytes = agent.storage.get("whatsapp:last_sender").await?;
    let Some(bytes) = phone_bytes else {
        tracing::debug!("WhatsApp send_message: no last_sender stored, skipping");
        return Ok(());
    };

    let phone_number = String::from_utf8_lossy(&bytes).to_string();
    if phone_number.is_empty() {
        return Ok(());
    }

    // Prefix with agent name so WhatsApp recipients know who's messaging
    let prefix = format!("[{}] ", agent.config.name);
    let prefixed_text =
        if text.starts_with(&prefix) || text.starts_with(&format!("[{}]", agent.config.name)) {
            text.to_string()
        } else {
            format!("{}{}", prefix, text)
        };

    match config.mode {
        WhatsAppMode::Baileys => send_via_bridge(config, &phone_number, &prefixed_text).await,
        WhatsAppMode::CloudApi => send_whatsapp_text(config, &phone_number, &prefixed_text).await,
    }
}

/// Send a message through the Baileys bridge sidecar.
async fn send_via_bridge(config: &WhatsAppChannelConfig, to: &str, text: &str) -> Result<()> {
    let formatted = format_for_whatsapp(text);
    let client = http_client();
    let url = format!("{}/send", config.bridge_url);

    let body = serde_json::json!({
        "to": to,
        "text": formatted
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
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

    Ok(())
}

/// Send a presence update (e.g. "composing" or "paused") via the Baileys bridge.
async fn send_presence(config: &WhatsAppChannelConfig, to: &str, presence_type: &str) -> Result<()> {
    let client = http_client();
    let url = format!("{}/presence", config.bridge_url);

    let body = serde_json::json!({
        "to": to,
        "type": presence_type
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_body = resp.text().await.unwrap_or_default();
        tracing::warn!("WhatsApp bridge presence error ({}): {}", status, error_body);
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

    if token != verify_token {
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

    // ---- Authorization check ----
    let is_baileys = body.get("_source").and_then(|v| v.as_str()) == Some("baileys");

    let config = {
        let agent_read = agent.read().await;
        agent_read.config.whatsapp.clone()
    };

    let config = match config {
        Some(c) => c,
        None if is_baileys => {
            // Baileys bridge runs inside the container (trusted localhost) —
            // allow messages even without explicit WhatsApp config in settings.
            tracing::info!(
                "WhatsApp: accepting Baileys message (bridge is trusted, no config needed)"
            );
            WhatsAppChannelConfig {
                mode: WhatsAppMode::Baileys,
                access_token: String::new(),
                phone_number_id: String::new(),
                verify_token: String::new(),
                bridge_url: "http://127.0.0.1:8999".to_string(),
                dm_policy: "open".to_string(),
                allowed_numbers: vec![],
            }
        }
        None => {
            tracing::warn!("WhatsApp webhook received but channel not configured");
            return Ok("ok".to_string());
        }
    };

    if !config.allowed_numbers.is_empty() && !config.allowed_numbers.iter().any(|n| n == from) {
        tracing::warn!(
            "WhatsApp: rejected message from unauthorized number {}",
            from
        );
        return Ok("ok".to_string());
    }

    // ---- DM pairing: require explicit approval for unknown senders ----
    if config.dm_policy == "pairing" && !config.allowed_numbers.is_empty() {
        // Already checked above — only allowed numbers get through.
        // For "pairing" with empty allowed_numbers, check storage for approved senders.
    }
    if config.dm_policy == "pairing" && config.allowed_numbers.is_empty() {
        let approved_key = format!("whatsapp:approved:{}", from);
        let is_approved = {
            let agent_read = agent.read().await;
            agent_read
                .storage
                .get(&approved_key)
                .await
                .ok()
                .flatten()
                .is_some()
        };
        if !is_approved {
            // Generate a pairing code and ask sender to approve
            let code = format!(
                "{:06}",
                from.as_bytes().iter().map(|b| *b as u64).sum::<u64>() % 1000000
            );
            let pairing_msg = format!(
                "Hello! I'm a AgentArk AI agent.\n\n\
                 For security, new contacts must be approved.\n\
                 Please ask the agent owner to run:\n\n\
                 _/approve {}_{}\n\n\
                 Your pairing code: *{}*",
                from, "", code
            );
            let _ = send_reply(&config, from, &pairing_msg).await;
            // Store the pairing code for verification
            {
                let agent_read = agent.read().await;
                let _ = agent_read
                    .storage
                    .set(&format!("whatsapp:pairing:{}", from), code.as_bytes())
                    .await;
            }
            tracing::info!(
                "WhatsApp: sent pairing request to {} (code: {})",
                from,
                code
            );
            return Ok("ok".to_string());
        }
    }

    // ---- Mark as read (fire and forget, Cloud API only — Baileys does it on bridge side) ----
    if !message_id.is_empty() && config.mode == WhatsAppMode::CloudApi {
        let config_clone = config.clone();
        let mid = message_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = mark_as_read(&config_clone, &mid).await {
                tracing::warn!("Failed to mark WhatsApp message as read: {}", e);
            }
        });
    }

    // ---- Persist last sender for push notifications ----
    {
        let agent_read = agent.read().await;
        let _ = agent_read
            .storage
            .set("whatsapp:last_sender", from.as_bytes())
            .await;
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

    // Also support plain-language tunnel commands for non-technical users.
    if let Some(cmd) = parse_tunnel_command(&text) {
        let response = execute_tunnel_command(&agent, cmd).await;
        send_reply(&config, from, &response).await?;
        return Ok("ok".to_string());
    }

    // Secret UX parity for chat channels: handle sensitive commands before LLM processing.
    let can_store_secret = if !config.allowed_numbers.is_empty() {
        config.allowed_numbers.iter().any(|n| n == from)
    } else if config.dm_policy == "pairing" {
        let approved_key = format!("whatsapp:approved:{}", from);
        let agent_read = agent.read().await;
        agent_read
            .storage
            .get(&approved_key)
            .await
            .ok()
            .flatten()
            .is_some()
    } else {
        false
    };

    if let Some((key, value)) = parse_set_secret(&text) {
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
    tokio::spawn(async move {
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
        let agent_read = agent.read().await;
        match agent_read
            .process_message(&text, "whatsapp", Some(&conversation_id), None)
            .await
        {
            Ok(r) => r,
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

/// Send a video to the last known WhatsApp sender.
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

    let phone_bytes = agent.storage.get("whatsapp:last_sender").await?;
    let Some(bytes) = phone_bytes else {
        return Ok(());
    };
    let phone_number = String::from_utf8_lossy(&bytes).to_string();
    if phone_number.is_empty() {
        return Ok(());
    }

    match config.mode {
        WhatsAppMode::Baileys => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(video_bytes);
            let client = http_client();
            let url = format!("{}/send-video", config.bridge_url);
            let body = serde_json::json!({
                "to": phone_number,
                "video": b64,
                "caption": caption,
            });
            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
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

/// Send an image preview to the last known WhatsApp sender.
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

    let phone_bytes = agent.storage.get("whatsapp:last_sender").await?;
    let Some(bytes) = phone_bytes else {
        return Ok(());
    };
    let phone_number = String::from_utf8_lossy(&bytes).to_string();
    if phone_number.is_empty() {
        return Ok(());
    }

    match config.mode {
        WhatsAppMode::Baileys => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
            let client = http_client();
            let url = format!("{}/send-image", config.bridge_url);
            let body = serde_json::json!({
                "to": phone_number,
                "image": b64,
                "caption": caption,
            });
            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
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
                 /run <skill> [query] - Run a custom/bundled skill\n\
                 /tasks - View pending tasks\n\
                 /search <query> - Web search\n\
                 /image <prompt> - Generate an image\n\
                 /tunnel [start|stop|status] - Manage public UI tunnel\n\
                 /setsecret KEY=VALUE - Store a secret encrypted (paired/allowlisted only)\n\
                 /approve <number> - Approve a contact\n\
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
                let agent = agent.read().await;
                match agent
                    .process_message(&prompt, "whatsapp", Some(&conversation_id), None)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => format!("Error: {}", e),
                }
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
                let agent = agent.read().await;
                match agent
                    .process_message(&prompt, "whatsapp", Some(&conversation_id), None)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => format!("Error: {}", e),
                }
            }
        }

        "/setsecret" => {
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
                let key = format!("whatsapp:approved:{}", from);
                storage.get(&key).await.ok().flatten().is_some()
            } else {
                false
            };

            if !allowlisted && !approved {
                return "Refusing to store secrets from this number. Pair/approve first (dm_policy=pairing) or add it to allowed_numbers in Settings.".to_string();
            }

            if args.is_empty() {
                return "Usage: /setsecret KEY=VALUE\nExample: /setsecret OPENAI_API_KEY=sk-..."
                    .to_string();
            }
            let input = format!("/setsecret {}", args);
            let Some((key, value)) = parse_set_secret(&input) else {
                return "Invalid syntax. Use: /setsecret KEY=VALUE".to_string();
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
                let key = format!("whatsapp:approved:{}", from);
                storage.get(&key).await.ok().flatten().is_some()
            } else {
                false
            };

            if !allowlisted && !approved {
                return "Refusing to store secrets from this number. Pair/approve first (dm_policy=pairing) or add it to allowed_numbers in Settings.".to_string();
            }

            if args.is_empty() {
                return "Usage: /usecurrentkey KEY\nExample: /usecurrentkey OPENAI_API_KEY"
                    .to_string();
            }
            let input = format!("/usecurrentkey {}", args);
            let Some(key) = parse_use_current_llm_key(&input) else {
                return "Invalid syntax. Use: /usecurrentkey KEY".to_string();
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
                        format!("{} {}", marker, t.description)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("*Pending Tasks*\n\n{}", list)
            }
        }

        "/search" => {
            if args.is_empty() {
                "Usage: /search <query>\n\nExample: /search latest news about AI".to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Search the web for: {}", args);
                    match agent
                        .process_message(&prompt, "whatsapp", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                };
                response
            }
        }

        "/image" => {
            if args.is_empty() {
                "Usage: /image <prompt>\n\nExample: /image a cute robot playing guitar".to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Generate an image of: {}", args);
                    match agent
                        .process_message(&prompt, "whatsapp", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                };
                response
            }
        }

        "/clear" => {
            let agent = agent.read().await;
            agent
                .clear_conversation_by_id("whatsapp", &conversation_id, None)
                .await;
            "Conversation cleared. Starting fresh!".to_string()
        }

        "/approve" => {
            if args.is_empty() {
                "Usage: /approve <phone_number>\n\nExample: /approve 15551234567".to_string()
            } else {
                let number = args.trim();
                let agent = agent.read().await;
                let approved_key = format!("whatsapp:approved:{}", number);
                match agent.storage.set(&approved_key, b"1").await {
                    Ok(_) => {
                        // Clear pending pairing code
                        let _ = agent
                            .storage
                            .delete(&format!("whatsapp:pairing:{}", number))
                            .await;
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

    #[test]
    fn test_split_message_short() {
        let text = "Hello, world!";
        let chunks = split_message(text, MAX_MESSAGE_LEN);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_split_message_at_paragraph() {
        let a = "A".repeat(2000);
        let b = "B".repeat(2000);
        let text = format!("{}\n\n{}", a, b);
        let chunks = split_message(&text, 2500);
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
}
