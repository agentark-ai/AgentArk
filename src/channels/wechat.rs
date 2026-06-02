use anyhow::{anyhow, Result};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustDecision};
use crate::core::Agent;

type SharedAgent = Arc<RwLock<Agent>>;

const LAST_DESTINATION_STORAGE_KEY: &str = "channels:wechat:last_destination";
const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:9140";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeChatChannelConfig {
    #[serde(default = "default_bridge_url")]
    pub bridge_url: String,
    #[serde(default)]
    pub bridge_token: String,
    #[serde(default)]
    pub default_target_id: String,
}

impl Default for WeChatChannelConfig {
    fn default() -> Self {
        Self {
            bridge_url: default_bridge_url(),
            bridge_token: String::new(),
            default_target_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WeChatDestinationContext {
    target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct WeChatBridgeEvent {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    sender_label: Option<String>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
}

fn default_bridge_url() -> String {
    DEFAULT_BRIDGE_URL.to_string()
}

fn trim_trailing_slashes(value: &str) -> &str {
    value.trim_end_matches('/')
}

async fn load_config(agent: &Agent) -> Result<Option<WeChatChannelConfig>> {
    Ok(agent.config.wechat.clone())
}

#[allow(dead_code)]
async fn load_destination(agent: &Agent) -> Result<Option<WeChatDestinationContext>> {
    if let Ok(Some(raw)) = agent.storage.get(LAST_DESTINATION_STORAGE_KEY).await {
        if let Ok(context) = serde_json::from_slice::<WeChatDestinationContext>(&raw) {
            if !context.target_id.trim().is_empty() {
                return Ok(Some(context));
            }
        }
    }
    Ok(None)
}

async fn persist_destination(agent: &Agent, context: &WeChatDestinationContext) -> Result<()> {
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

fn resolve_destination(config: &WeChatChannelConfig) -> Result<WeChatDestinationContext> {
    if !config.default_target_id.trim().is_empty() {
        return Ok(WeChatDestinationContext {
            target_id: config.default_target_id.clone(),
            sender_id: None,
            sender_label: None,
        });
    }
    Err(anyhow!("WeChat has no configured notification destination"))
}

async fn send_to_destination(
    config: &WeChatChannelConfig,
    destination: &WeChatDestinationContext,
    text: &str,
) -> Result<()> {
    if config.bridge_url.trim().is_empty() {
        return Err(anyhow!("WeChat bridge URL is missing"));
    }
    if destination.target_id.trim().is_empty() {
        return Err(anyhow!("WeChat target is missing"));
    }
    for chunk in super::outbound_split::split_for_provider_safe_channel("wechat", text) {
        let mut request = http_client()?.post(format!(
            "{}/send",
            trim_trailing_slashes(&config.bridge_url)
        ));
        if !config.bridge_token.trim().is_empty() {
            request = request.header("x-agentark-bridge-token", config.bridge_token.trim());
        }
        let response = super::outbound_rate_limit::send_with_bounded_retries(
            "wechat",
            "bridge_message",
            request.json(&serde_json::json!({
                "channel": "wechat",
                "target_id": destination.target_id,
                "text": chunk
            })),
        )
        .await?;
        if !response.status().is_success() {
            let payload = response.text().await.unwrap_or_default();
            return Err(anyhow!("WeChat bridge error: {}", payload));
        }
    }
    Ok(())
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = load_config(agent)
        .await?
        .ok_or_else(|| anyhow!("WeChat is not configured"))?;
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
    .ok_or_else(|| anyhow!("WeChat is not configured"))?;
    if config.bridge_token.trim().is_empty() {
        return Err(anyhow!(
            "WeChat bridge token is required for inbound webhooks"
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
        return Err(anyhow!("WeChat bridge token mismatch"));
    }
    let event = serde_json::from_slice::<WeChatBridgeEvent>(raw_body)?;
    let text = event.text.as_deref().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Ok("ignored".to_string());
    }
    let destination = WeChatDestinationContext {
        target_id: event
            .target_id
            .clone()
            .or_else(|| event.sender_id.clone())
            .unwrap_or_default(),
        sender_id: event.sender_id.clone(),
        sender_label: event.sender_label.clone(),
    };
    let conversation_id = event
        .conversation_id
        .clone()
        .unwrap_or_else(|| format!("wechat:{}", destination.target_id));
    let reply = {
        let agent_snapshot = Agent::snapshot(&agent).await;
        if let Some(sender_id) = event
            .sender_id
            .clone()
            .filter(|value| !value.trim().is_empty())
        {
            let verification = sender_verification::load_settings(&agent_snapshot.storage).await?;
            let identity = SenderIdentity {
                channel: SenderChannel::WeChat,
                sender_id: sender_id.clone(),
                sender_label: event.sender_label.clone().or(Some(sender_id)),
                scope_id: None,
                scope_label: None,
                conversation_id: Some(conversation_id.clone()),
                message_preview: Some(text.clone()),
            };
            match sender_verification::evaluate_sender_with_rules(
                &agent_snapshot.storage,
                &identity,
                verification.wechat.policy,
                &verification.wechat.allowed_senders,
            )
            .await?
            {
                SenderTrustDecision::Allowed => {}
                SenderTrustDecision::NeedsApproval { created_new, .. } => {
                    if created_new {
                        agent_snapshot
                            .notify_preferred_channel(
                                "A new WeChat sender needs approval before AgentArk will reply. Open Settings -> Messaging Channels -> Sender Trust to review it.",
                            )
                            .await;
                    }
                    return Ok("approval_pending".to_string());
                }
            }
        }
        persist_destination(&agent_snapshot, &destination).await?;
        agent_snapshot
            .process_message_with_meta(&text, "wechat", Some(&conversation_id), None)
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
    fn proactive_wechat_destination_uses_configured_target_only() {
        let config = WeChatChannelConfig {
            bridge_url: "https://bridge.example.com".to_string(),
            bridge_token: "token".to_string(),
            default_target_id: "wechat-target".to_string(),
        };

        let destination = resolve_destination(&config).unwrap();
        assert_eq!(destination.target_id, "wechat-target");
        assert_eq!(destination.sender_id, None);
    }
}
