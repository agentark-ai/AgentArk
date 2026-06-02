use anyhow::{anyhow, Result};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustDecision};
use crate::core::Agent;

type SharedAgent = Arc<RwLock<Agent>>;

const LAST_DESTINATION_STORAGE_KEY: &str = "channels:imessage:last_destination";
const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:9130";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IMessageChannelConfig {
    #[serde(default = "default_bridge_url")]
    pub bridge_url: String,
    #[serde(default)]
    pub bridge_token: String,
    #[serde(default)]
    pub default_chat_id: String,
    #[serde(default)]
    pub default_handle: String,
}

impl Default for IMessageChannelConfig {
    fn default() -> Self {
        Self {
            bridge_url: default_bridge_url(),
            bridge_token: String::new(),
            default_chat_id: String::new(),
            default_handle: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct IMessageDestinationContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct IMessageBridgeEvent {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    sender_label: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
}

fn default_bridge_url() -> String {
    DEFAULT_BRIDGE_URL.to_string()
}

fn trim_trailing_slashes(value: &str) -> &str {
    value.trim_end_matches('/')
}

async fn load_config(agent: &Agent) -> Result<Option<IMessageChannelConfig>> {
    Ok(agent.config.imessage.clone())
}

#[allow(dead_code)]
async fn load_destination(agent: &Agent) -> Result<Option<IMessageDestinationContext>> {
    if let Ok(Some(raw)) = agent.storage.get(LAST_DESTINATION_STORAGE_KEY).await {
        if let Ok(context) = serde_json::from_slice::<IMessageDestinationContext>(&raw) {
            if context
                .chat_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
                || context
                    .handle
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some()
            {
                return Ok(Some(context));
            }
        }
    }
    Ok(None)
}

async fn persist_destination(agent: &Agent, context: &IMessageDestinationContext) -> Result<()> {
    let raw = serde_json::to_vec(context)?;
    agent
        .storage
        .set(LAST_DESTINATION_STORAGE_KEY, &raw)
        .await?;
    Ok(())
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn resolve_destination(config: &IMessageChannelConfig) -> Result<IMessageDestinationContext> {
    if !config.default_chat_id.trim().is_empty() || !config.default_handle.trim().is_empty() {
        return Ok(IMessageDestinationContext {
            chat_id: if config.default_chat_id.trim().is_empty() {
                None
            } else {
                Some(config.default_chat_id.clone())
            },
            handle: if config.default_handle.trim().is_empty() {
                None
            } else {
                Some(config.default_handle.clone())
            },
            sender_id: None,
            sender_label: None,
        });
    }
    Err(anyhow!(
        "iMessage has no configured notification destination"
    ))
}

async fn send_to_destination(
    config: &IMessageChannelConfig,
    destination: &IMessageDestinationContext,
    text: &str,
) -> Result<()> {
    if config.bridge_url.trim().is_empty() {
        return Err(anyhow!("iMessage bridge URL is missing"));
    }
    if destination
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
        && destination
            .handle
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err(anyhow!("iMessage destination is missing"));
    }
    for chunk in super::outbound_split::split_for_provider_safe_channel("imessage", text) {
        let mut request = http_client()?.post(format!(
            "{}/send",
            trim_trailing_slashes(&config.bridge_url)
        ));
        if !config.bridge_token.trim().is_empty() {
            request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
        }
        let response = super::outbound_rate_limit::send_with_bounded_retries(
            "imessage",
            "bridge_message",
            request.json(&serde_json::json!({
                "channel": "imessage",
                "text": chunk,
                "chat_id": destination.chat_id,
                "handle": destination.handle
            })),
        )
        .await?;
        if !response.status().is_success() {
            let payload = response.text().await.unwrap_or_default();
            return Err(anyhow!("iMessage bridge error: {}", payload));
        }
    }
    Ok(())
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = load_config(agent)
        .await?
        .ok_or_else(|| anyhow!("iMessage is not configured"))?;
    let destination = resolve_destination(&config)?;
    send_to_destination(&config, &destination, text).await?;
    Ok(())
}

pub async fn handle_webhook(
    agent: SharedAgent,
    headers: &HeaderMap,
    raw_body: &[u8],
) -> Result<String> {
    let config = {
        let agent = agent.read().await;
        load_config(&agent).await?
    }
    .ok_or_else(|| anyhow!("iMessage is not configured"))?;
    if config.bridge_token.trim().is_empty() {
        return Err(anyhow!(
            "iMessage bridge token is required for inbound webhooks"
        ));
    }
    let token = headers
        .get("x-agentark-bridge-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !crate::security::constant_time_eq(
        token.trim().as_bytes(),
        config.bridge_token.trim().as_bytes(),
    ) {
        return Err(anyhow!("iMessage bridge token mismatch"));
    }
    let event = serde_json::from_slice::<IMessageBridgeEvent>(raw_body)?;
    let text = event.text.as_deref().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Ok("ignored".to_string());
    }
    let destination = IMessageDestinationContext {
        chat_id: event.chat_id.clone(),
        handle: event.handle.clone().or_else(|| event.sender_id.clone()),
        sender_id: event.sender_id.clone(),
        sender_label: event.sender_label.clone(),
    };
    let conversation_id = event.conversation_id.clone().unwrap_or_else(|| {
        format!(
            "imessage:{}",
            event
                .chat_id
                .clone()
                .unwrap_or_else(|| event.sender_id.clone().unwrap_or_default())
        )
    });

    let reply = {
        let agent_snapshot = Agent::snapshot(&agent).await;
        if let Some(sender_id) = event
            .sender_id
            .clone()
            .filter(|value| !value.trim().is_empty())
        {
            let verification = sender_verification::load_settings(&agent_snapshot.storage).await?;
            let identity = SenderIdentity {
                channel: SenderChannel::IMessage,
                sender_id: sender_id.clone(),
                sender_label: event.sender_label.clone().or(Some(sender_id)),
                scope_id: event.chat_id.clone(),
                scope_label: event.chat_id.clone(),
                conversation_id: Some(conversation_id.clone()),
                message_preview: Some(text.clone()),
            };
            match sender_verification::evaluate_sender_with_rules(
                &agent_snapshot.storage,
                &identity,
                verification.imessage.policy,
                &verification.imessage.allowed_senders,
            )
            .await?
            {
                SenderTrustDecision::Allowed => {}
                SenderTrustDecision::NeedsApproval { created_new, .. } => {
                    if created_new {
                        agent_snapshot
                            .notify_preferred_channel(
                                "A new iMessage sender needs approval before AgentArk will reply. Open Settings -> Messaging Channels -> Sender Trust to review it.",
                            )
                            .await;
                    }
                    return Ok("approval_pending".to_string());
                }
            }
        }
        persist_destination(&agent_snapshot, &destination).await?;
        agent_snapshot
            .process_message_with_meta(&text, "imessage", Some(&conversation_id), None)
            .await
            .map(Agent::render_plain_channel_response)?
    };
    if reply.trim().is_empty() {
        return Ok("ignored".to_string());
    }
    send_to_destination(&config, &destination, &reply).await?;
    let agent_snapshot = Agent::snapshot(&agent).await;
    persist_destination(&agent_snapshot, &destination).await?;
    Ok("ok".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proactive_imessage_destination_uses_configured_target_only() {
        let config = IMessageChannelConfig {
            bridge_url: "https://bridge.example.com".to_string(),
            bridge_token: "token".to_string(),
            default_chat_id: "chat-123".to_string(),
            default_handle: String::new(),
        };

        let destination = resolve_destination(&config).unwrap();
        assert_eq!(destination.chat_id.as_deref(), Some("chat-123"));
        assert_eq!(destination.handle, None);
    }
}
