//! Communication channels - HTTP, Telegram, WhatsApp, etc.

pub mod discord;
pub mod gateway;
pub mod google_chat;
pub mod http;
pub mod imessage;
pub mod line;
pub mod matrix;
pub mod messaging_dispatch;
pub mod messaging_registry;
pub(crate) mod outbound_rate_limit;
pub mod qq;
pub mod signal;
pub mod slack;
pub mod teams;
pub mod web;
pub mod wechat;
pub mod whatsapp;

#[cfg(feature = "telegram")]
pub mod telegram;

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
