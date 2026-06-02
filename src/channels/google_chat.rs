use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::core::sender_verification::{self, SenderChannel, SenderIdentity, SenderTrustDecision};
use crate::core::Agent;

type SharedAgent = Arc<RwLock<Agent>>;

const LAST_DESTINATION_STORAGE_KEY: &str = "channels:google_chat:last_destination";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleChatChannelConfig {
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub verify_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
}

impl Default for GoogleChatChannelConfig {
    fn default() -> Self {
        Self {
            api_base_url: default_api_base_url(),
            access_token: String::new(),
            verify_token: String::new(),
            space: None,
            thread_key: None,
            app_id: None,
            bot_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct GoogleChatDestinationContext {
    pub space: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GoogleChatWebhookPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<GoogleChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<GoogleChatUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space: Option<GoogleChatSpace>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GoogleChatMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sender: Option<GoogleChatUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space: Option<GoogleChatSpace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread: Option<GoogleChatThread>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GoogleChatUser {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(
        default,
        rename = "displayName",
        skip_serializing_if = "Option::is_none"
    )]
    display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GoogleChatSpace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(
        default,
        rename = "displayName",
        skip_serializing_if = "Option::is_none"
    )]
    display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GoogleChatThread {
    #[serde(default, rename = "threadKey", skip_serializing_if = "Option::is_none")]
    thread_key: Option<String>,
}

fn default_api_base_url() -> String {
    "https://chat.googleapis.com".to_string()
}

fn trim_trailing_slashes(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?)
}

#[allow(dead_code)]
async fn load_destination(agent: &Agent) -> Result<Option<GoogleChatDestinationContext>> {
    if let Ok(Some(raw)) = agent.storage.get(LAST_DESTINATION_STORAGE_KEY).await {
        if let Ok(value) = serde_json::from_slice::<GoogleChatDestinationContext>(&raw) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

async fn persist_destination(
    agent: &Agent,
    destination: &GoogleChatDestinationContext,
) -> Result<()> {
    let raw = serde_json::to_vec(destination)?;
    agent
        .storage
        .set(LAST_DESTINATION_STORAGE_KEY, &raw)
        .await?;
    Ok(())
}

fn normalize_space(space: &str) -> String {
    let trimmed = space.trim();
    if trimmed.starts_with("spaces/") {
        trimmed.to_string()
    } else {
        format!("spaces/{}", trimmed)
    }
}

fn resolve_destination(
    config: &GoogleChatChannelConfig,
    _last_destination: Option<GoogleChatDestinationContext>,
) -> Result<GoogleChatDestinationContext> {
    let Some(space) = config
        .space
        .clone()
        .filter(|value| !value.trim().is_empty())
    else {
        return Err(anyhow!("Google Chat notification space is not configured"));
    };
    Ok(GoogleChatDestinationContext {
        space: normalize_space(&space),
        thread_key: config
            .thread_key
            .clone()
            .filter(|value| !value.trim().is_empty()),
    })
}

async fn send_message_to_destination(
    config: &GoogleChatChannelConfig,
    destination: &GoogleChatDestinationContext,
    text: &str,
) -> Result<()> {
    if config.access_token.trim().is_empty() {
        return Err(anyhow!("Google Chat access token is missing"));
    }
    let url = format!(
        "{}/v1/{}/messages",
        trim_trailing_slashes(&config.api_base_url),
        normalize_space(&destination.space)
    );
    for text in super::outbound_split::split_for_provider_safe_channel("google_chat", text) {
        let mut body = serde_json::json!({ "text": text });
        if let Some(thread_key) = destination.thread_key.as_deref() {
            body["thread"] = serde_json::json!({ "threadKey": thread_key });
        }
        let client = http_client()?;
        let response = super::outbound_rate_limit::send_with_bounded_retries(
            "google_chat",
            "post_message",
            client
                .post(&url)
                .bearer_auth(&config.access_token)
                .json(&body),
        )
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Google Chat delivery failed ({}): {}",
                status,
                body
            ));
        }
    }
    Ok(())
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let config = agent
        .config
        .google_chat
        .clone()
        .ok_or_else(|| anyhow!("Google Chat is not configured"))?;
    let destination = resolve_destination(&config, None)?;
    send_message_to_destination(&config, &destination, text).await?;
    Ok(())
}

pub async fn handle_webhook(agent: SharedAgent, payload: &serde_json::Value) -> Result<String> {
    let payload = serde_json::from_value::<GoogleChatWebhookPayload>(payload.clone())?;
    let (config, trust_settings) = {
        let guard = agent.read().await;
        (
            guard
                .config
                .google_chat
                .clone()
                .ok_or_else(|| anyhow!("Google Chat is not configured"))?,
            sender_verification::load_settings(&guard.storage).await?,
        )
    };
    if config.verify_token.trim().is_empty() {
        return Err(anyhow!(
            "Google Chat verification token is required for inbound webhooks"
        ));
    }
    let provided = payload.token.as_deref().unwrap_or("").trim();
    if !crate::security::constant_time_eq(
        provided.as_bytes(),
        config.verify_token.trim().as_bytes(),
    ) {
        return Err(anyhow!("Google Chat verification token mismatch"));
    }

    let Some(message) = payload.message else {
        return Ok("ignored".to_string());
    };
    let text = message.text.unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Ok("ignored".to_string());
    }

    let space = message
        .space
        .as_ref()
        .and_then(|value| value.name.clone())
        .or_else(|| payload.space.as_ref().and_then(|value| value.name.clone()))
        .unwrap_or_default();
    if space.trim().is_empty() {
        return Err(anyhow!("Google Chat message did not include a space id"));
    }
    let destination = GoogleChatDestinationContext {
        space: normalize_space(&space),
        thread_key: message.thread.and_then(|value| value.thread_key),
    };
    let sender = message.sender.or(payload.user).unwrap_or_default();
    let sender_id = sender.name.clone().unwrap_or_default();
    if sender_id.trim().is_empty() {
        return Ok("ignored".to_string());
    }
    let sender_label = sender
        .display_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| sender.name.clone());
    let space_label = message
        .space
        .as_ref()
        .and_then(|value| value.display_name.clone())
        .or_else(|| {
            payload
                .space
                .as_ref()
                .and_then(|value| value.display_name.clone())
        });
    let conversation_id = format!(
        "google_chat:{}:{}",
        destination.space,
        destination
            .thread_key
            .clone()
            .unwrap_or_else(|| "root".to_string())
    );

    let trust_decision = {
        let guard = agent.read().await;
        let identity = SenderIdentity {
            channel: SenderChannel::GoogleChat,
            sender_id: sender_id.clone(),
            sender_label: sender_label.clone(),
            scope_id: Some(destination.space.clone()),
            scope_label: space_label.clone(),
            conversation_id: Some(conversation_id.clone()),
            message_preview: Some(text.clone()),
        };
        sender_verification::evaluate_sender_with_rules(
            &guard.storage,
            &identity,
            trust_settings.google_chat.policy,
            &trust_settings.google_chat.allowed_senders,
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
                        "A new Google Chat sender needs approval before {} will act.\nSender: {}\nSpace: {}\nMessage: {}",
                        crate::branding::PRODUCT_NAME,
                        sender_label.as_deref().unwrap_or(sender_id.as_str()),
                        space_label.as_deref().unwrap_or(destination.space.as_str()),
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
        return Ok("approval_pending".to_string());
    }

    let response = {
        let agent_snapshot = Agent::snapshot(&agent).await;
        persist_destination(&agent_snapshot, &destination).await?;
        let processed = agent_snapshot
            .process_message_with_meta(text.as_str(), "google_chat", Some(&conversation_id), None)
            .await?;
        Agent::render_plain_channel_response(processed)
    };

    if response.trim().is_empty() {
        return Ok("ok".to_string());
    }

    send_message_to_destination(&config, &destination, &response).await?;
    Ok("ok".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proactive_destination_uses_configured_space_and_thread_only() {
        let config = GoogleChatChannelConfig {
            access_token: "token".to_string(),
            verify_token: "verify".to_string(),
            space: Some("spaces/AAA".to_string()),
            thread_key: Some("thread-config".to_string()),
            ..Default::default()
        };
        let last_destination = Some(GoogleChatDestinationContext {
            space: "spaces/OLD".to_string(),
            thread_key: Some("thread-old".to_string()),
        });

        let destination = resolve_destination(&config, last_destination).unwrap();
        assert_eq!(destination.space, "spaces/AAA");
        assert_eq!(destination.thread_key.as_deref(), Some("thread-config"));
    }
}
