//! Communication channels - HTTP, Telegram, WhatsApp, etc.

#[path = "messaging/discord.rs"]
pub mod discord;
#[path = "gateway/gateway.rs"]
pub mod gateway;
#[path = "messaging/google_chat.rs"]
pub mod google_chat;
pub mod http;
#[path = "messaging/imessage.rs"]
pub mod imessage;
#[path = "messaging/line.rs"]
pub mod line;
#[path = "messaging/matrix.rs"]
pub mod matrix;
#[path = "dispatch/messaging_dispatch.rs"]
pub mod messaging_dispatch;
#[path = "dispatch/messaging_registry.rs"]
pub mod messaging_registry;
#[path = "outbound/outbound_rate_limit.rs"]
pub(crate) mod outbound_rate_limit;
#[path = "outbound/outbound_split.rs"]
pub(crate) mod outbound_split;
#[path = "messaging/qq.rs"]
pub mod qq;
#[path = "messaging/signal.rs"]
pub mod signal;
#[path = "messaging/slack.rs"]
pub mod slack;
#[path = "messaging/teams.rs"]
pub mod teams;
#[path = "web/web.rs"]
pub mod web;
#[path = "messaging/wechat.rs"]
pub mod wechat;
#[path = "messaging/whatsapp.rs"]
pub mod whatsapp;

#[cfg(feature = "telegram")]
#[path = "messaging/telegram.rs"]
pub mod telegram;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelError {
    #[error("ERR/channel/missing_input: {message}")]
    MissingInput { channel: String, message: String },
    #[error("ERR/channel/invalid_input: {message}")]
    InvalidInput { channel: String, message: String },
    #[error("ERR/channel/not_connected: {message}")]
    NotConnected { channel: String, message: String },
    #[error("ERR/channel/unavailable: {message}")]
    Unavailable { channel: String, message: String },
    #[error("ERR/channel/permission_denied: {message}")]
    PermissionDenied { channel: String, message: String },
    #[error("ERR/channel/rate_limited: {message}")]
    RateLimited { channel: String, message: String },
    #[error("ERR/channel/timeout: {message}")]
    Timeout { channel: String, message: String },
    #[error("ERR/channel/failed: {message}")]
    Failed { channel: String, message: String },
}

impl ChannelError {
    pub fn not_connected(channel: impl Into<String>, message: impl Into<String>) -> Self {
        Self::NotConnected {
            channel: channel.into(),
            message: message.into(),
        }
    }

    pub fn channel(&self) -> &str {
        match self {
            Self::MissingInput { channel, .. }
            | Self::InvalidInput { channel, .. }
            | Self::NotConnected { channel, .. }
            | Self::Unavailable { channel, .. }
            | Self::PermissionDenied { channel, .. }
            | Self::RateLimited { channel, .. }
            | Self::Timeout { channel, .. }
            | Self::Failed { channel, .. } => channel,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::MissingInput { message, .. }
            | Self::InvalidInput { message, .. }
            | Self::NotConnected { message, .. }
            | Self::Unavailable { message, .. }
            | Self::PermissionDenied { message, .. }
            | Self::RateLimited { message, .. }
            | Self::Timeout { message, .. }
            | Self::Failed { message, .. } => message,
        }
    }

    pub fn reason(&self) -> crate::actions::ActionErrorReason {
        match self {
            Self::MissingInput { .. } => crate::actions::ActionErrorReason::MissingInput,
            Self::InvalidInput { .. } => crate::actions::ActionErrorReason::InvalidInput,
            Self::NotConnected { .. } => crate::actions::ActionErrorReason::NotConnected,
            Self::Unavailable { .. } => crate::actions::ActionErrorReason::Unavailable,
            Self::PermissionDenied { .. } => crate::actions::ActionErrorReason::PermissionDenied,
            Self::RateLimited { .. } => crate::actions::ActionErrorReason::RateLimited,
            Self::Timeout { .. } => crate::actions::ActionErrorReason::Timeout,
            Self::Failed { .. } => crate::actions::ActionErrorReason::Failed,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingInput { .. } => "channel_missing_input",
            Self::InvalidInput { .. } => "channel_invalid_input",
            Self::NotConnected { .. } => "channel_not_connected",
            Self::Unavailable { .. } => "channel_unavailable",
            Self::PermissionDenied { .. } => "channel_permission_denied",
            Self::RateLimited { .. } => "channel_rate_limited",
            Self::Timeout { .. } => "channel_timeout",
            Self::Failed { .. } => "channel_failed",
        }
    }

    pub fn as_action_error(&self) -> crate::actions::ActionError {
        crate::actions::ActionError::new(
            crate::actions::ActionErrorDomain::Channel,
            self.reason(),
            self.message(),
        )
    }
}

impl From<ChannelError> for crate::actions::ActionError {
    fn from(error: ChannelError) -> Self {
        error.as_action_error()
    }
}

/// Send a screenshot image with caption to the appropriate channel
#[allow(unused_variables, dead_code)]
pub async fn send_screenshot(
    agent: &crate::core::Agent,
    channel: &str,
    image_bytes: &[u8],
    caption: &str,
    image_url: Option<&str>,
) -> anyhow::Result<()> {
    match channel {
        #[cfg(feature = "telegram")]
        "telegram" => {
            telegram::send_photo(agent, image_bytes, caption).await?;
        }
        "whatsapp" => {
            // WhatsApp image sending — for now send text notification
            // Full image support requires bridge media upload endpoint
            whatsapp::send_image(agent, image_bytes, caption, image_url).await?;
        }
        _ => {
            // Web UI — screenshots are delivered via browser session HTTP endpoints
            tracing::debug!("Screenshot for web channel stored in session state");
        }
    }
    Ok(())
}

/// Send a video with caption to the appropriate channel
#[allow(unused_variables, dead_code)]
pub async fn send_video_to_channel(
    agent: &crate::core::Agent,
    channel: &str,
    video_bytes: &[u8],
    caption: &str,
    download_url: Option<&str>,
) -> anyhow::Result<()> {
    match channel {
        #[cfg(feature = "telegram")]
        "telegram" => {
            telegram::send_video(agent, video_bytes, caption).await?;
        }
        "whatsapp" => {
            whatsapp::send_video(agent, video_bytes, caption, download_url).await?;
        }
        _ => {
            // Web UI — videos are delivered via the [VIDEO_RESULT] marker in the response text
            tracing::debug!("Video for web channel delivered via response URL");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_errors_have_machine_readable_codes() {
        let error = ChannelError::not_connected("telegram", "Telegram delivery is not connected");

        assert_eq!(error.code(), "channel_not_connected");
        assert_eq!(
            error.to_string(),
            "ERR/channel/not_connected: Telegram delivery is not connected"
        );

        let action_error = error.as_action_error();
        assert_eq!(action_error.code(), "channel_not_connected");
    }
}
