use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustDecision};
use crate::core::Agent;

type SharedAgent = Arc<RwLock<Agent>>;

const LAST_DESTINATION_STORAGE_KEY: &str = "channels:line:last_destination";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineChannelConfig {
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub channel_access_token: String,
    #[serde(default)]
    pub channel_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

impl Default for LineChannelConfig {
    fn default() -> Self {
        Self {
            api_base_url: default_api_base_url(),
            channel_access_token: String::new(),
            channel_secret: String::new(),
            default_target: None,
            user_agent: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LineDestinationContext {
    target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LineWebhookPayload {
    #[serde(default)]
    events: Vec<LineEvent>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LineEvent {
    #[serde(rename = "type", default)]
    event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<LineSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<LineMessage>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LineSource {
    #[serde(default, rename = "userId", skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(default, rename = "groupId", skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
    #[serde(default, rename = "roomId", skip_serializing_if = "Option::is_none")]
    room_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LineMessage {
    #[serde(rename = "type", default)]
    message_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

fn default_api_base_url() -> String {
    "https://api.line.me".to_string()
}

fn trim_trailing_slashes(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    crate::security::constant_time_eq(left, right)
}

fn hmac_sha256_base64(secret: &str, body: &[u8]) -> String {
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
    STANDARD.encode(outer.finalize())
}

fn verify_line_signature(secret: &str, raw_body: &[u8], signature: Option<&str>) -> Result<()> {
    if secret.trim().is_empty() {
        return Err(anyhow!(
            "LINE channel secret is required for inbound webhooks"
        ));
    }
    let provided = signature
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("LINE signature header is required"))?;
    let expected = hmac_sha256_base64(secret, raw_body);
    if !constant_time_eq(expected.as_bytes(), provided.as_bytes()) {
        return Err(anyhow!("LINE signature verification failed"));
    }
    Ok(())
}

#[allow(dead_code)]
async fn load_destination(agent: &Agent) -> Result<Option<LineDestinationContext>> {
    if let Ok(Some(raw)) = agent.storage.get(LAST_DESTINATION_STORAGE_KEY).await {
        if let Ok(value) = serde_json::from_slice::<LineDestinationContext>(&raw) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

async fn persist_destination(agent: &Agent, destination: &LineDestinationContext) -> Result<()> {
    let raw = serde_json::to_vec(destination)?;
    agent
        .storage
        .set(LAST_DESTINATION_STORAGE_KEY, &raw)
        .await?;
    Ok(())
}

fn resolve_destination(
    config: &LineChannelConfig,
    _last_destination: Option<LineDestinationContext>,
) -> Result<LineDestinationContext> {
    let Some(target) = config
        .default_target
        .clone()
        .filter(|value| !value.trim().is_empty())
    else {
        return Err(anyhow!("LINE target is not configured"));
    };
    Ok(LineDestinationContext {
        target,
        scope_id: None,
    })
}

async fn send_message_to_destination(
    config: &LineChannelConfig,
    destination: &LineDestinationContext,
    text: &str,
) -> Result<()> {
    if config.channel_access_token.trim().is_empty() {
        return Err(anyhow!("LINE channel access token is missing"));
    }
    let url = format!(
        "{}/v2/bot/message/push",
        trim_trailing_slashes(&config.api_base_url)
    );
    let client = http_client()?;
    for chunk in super::outbound_split::split_for_provider_safe_channel("line", text) {
        let response = super::outbound_rate_limit::send_with_bounded_retries(
            "line",
            "push_message",
            client
                .post(&url)
                .bearer_auth(&config.channel_access_token)
                .json(&json!({
                    "to": destination.target,
                    "messages": [{ "type": "text", "text": chunk }],
                })),
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("LINE delivery failed ({}): {}", status, body));
        }
    }
    Ok(())
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = agent
        .config
        .line
        .clone()
        .ok_or_else(|| anyhow!("LINE is not configured"))?;
    let destination = resolve_destination(&config, None)?;
    send_message_to_destination(&config, &destination, text).await?;
    Ok(())
}

fn line_target(source: &LineSource) -> Option<String> {
    source
        .user_id
        .clone()
        .or_else(|| source.group_id.clone())
        .or_else(|| source.room_id.clone())
}

pub async fn handle_webhook(
    agent: SharedAgent,
    raw_body: &[u8],
    signature: Option<&str>,
) -> Result<String> {
    let payload = serde_json::from_slice::<LineWebhookPayload>(raw_body)?;
    let (config, trust_settings) = {
        let guard = agent.read().await;
        (
            guard
                .config
                .line
                .clone()
                .ok_or_else(|| anyhow!("LINE is not configured"))?,
            sender_verification::load_settings(&guard.storage).await?,
        )
    };
    verify_line_signature(&config.channel_secret, raw_body, signature)?;

    for event in payload.events {
        if event.event_type != "message" {
            continue;
        }
        let Some(message) = event.message else {
            continue;
        };
        if message.message_type != "text" {
            continue;
        }
        let text = message.text.unwrap_or_default().trim().to_string();
        if text.is_empty() {
            continue;
        }
        let Some(source) = event.source else {
            continue;
        };
        let Some(target) = line_target(&source) else {
            continue;
        };
        let sender_id = source.user_id.clone().unwrap_or_else(|| target.clone());
        let scope_id = source.group_id.clone().or(source.room_id.clone());
        let destination = LineDestinationContext {
            target: target.clone(),
            scope_id: scope_id.clone(),
        };
        let conversation_id =
            if let Some(scope) = scope_id.clone().filter(|value| !value.trim().is_empty()) {
                format!("line:scope:{}", scope)
            } else {
                format!("line:dm:{}", sender_id)
            };

        let trust_decision = {
            let guard = agent.read().await;
            let identity = SenderIdentity {
                channel: SenderChannel::Line,
                sender_id: sender_id.clone(),
                sender_label: Some(sender_id.clone()),
                scope_id: scope_id.clone(),
                scope_label: scope_id.clone(),
                conversation_id: Some(conversation_id.clone()),
                message_preview: Some(text.clone()),
            };
            sender_verification::evaluate_sender_with_rules(
                &guard.storage,
                &identity,
                trust_settings.line.policy,
                &trust_settings.line.allowed_senders,
            )
            .await?
        };

        if let SenderTrustDecision::NeedsApproval { created_new, .. } = trust_decision {
            if created_new {
                let guard = agent.read().await;
                guard
                    .emit_notification_forced(
                        "Sender Approval Needed",
                        &format!(
                            "A new LINE sender needs approval before {} will act.\nSender: {}\nMessage: {}",
                            crate::branding::PRODUCT_NAME,
                            sender_id,
                            text.chars().take(180).collect::<String>()
                        ),
                        "warning",
                        "sender_verification",
                    )
                    .await;
            }
            let _ = send_message_to_destination(
                &config,
                &destination,
                &format!(
                    "Sender approval is required before {} can respond here. Approve this sender in Settings -> Messaging Channels -> Sender Trust.",
                    crate::branding::PRODUCT_NAME
                ),
            )
            .await;
            continue;
        }

        let response = {
            let agent_snapshot = Agent::snapshot(&agent).await;
            persist_destination(&agent_snapshot, &destination).await?;
            let processed = agent_snapshot
                .process_message_with_meta(text.as_str(), "line", Some(&conversation_id), None)
                .await?;
            Agent::render_plain_channel_response(processed)
        };

        if !response.trim().is_empty() {
            send_message_to_destination(&config, &destination, &response).await?;
        }
    }

    Ok("ok".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proactive_line_destination_uses_configured_target_only() {
        let config = LineChannelConfig {
            channel_access_token: "token".to_string(),
            channel_secret: "secret".to_string(),
            default_target: Some("U-config".to_string()),
            ..Default::default()
        };
        let last_destination = Some(LineDestinationContext {
            target: "U-last".to_string(),
            scope_id: Some("scope".to_string()),
        });

        let destination = resolve_destination(&config, last_destination).unwrap();
        assert_eq!(destination.target, "U-config");
        assert_eq!(destination.scope_id, None);
    }
}
