//! Twilio Voice & SMS Integration
//!
//! Provides voice calling and SMS messaging via the Twilio REST API.
//! Supports making calls, sending SMS, and listing recent call/message history.
//! Authentication uses HTTP Basic Auth with Account SID and Auth Token.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Twilio voice and SMS connector
pub struct TwilioConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl TwilioConnector {
    const API_BASE: &'static str = "https://api.twilio.com/2010-04-01";

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: crate::core::runtime::net::default_outgoing_http_client(),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_config_dir(config_dir)
    }

    fn load_account_sid_from(config_dir: &Path) -> Option<String> {
        if let Ok(val) = std::env::var("TWILIO_ACCOUNT_SID") {
            if !val.is_empty() {
                return Some(val);
            }
        }
        match crate::core::runtime::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("twilio_account_sid")
                .ok()
                .flatten(),
            Err(_) => None,
        }
    }

    fn load_auth_token_from(config_dir: &Path) -> Option<String> {
        if let Ok(val) = std::env::var("TWILIO_AUTH_TOKEN") {
            if !val.is_empty() {
                return Some(val);
            }
        }
        match crate::core::runtime::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("twilio_auth_token")
                .ok()
                .flatten(),
            Err(_) => None,
        }
    }

    fn load_from_number_from(config_dir: &Path) -> Option<String> {
        if let Ok(val) = std::env::var("TWILIO_FROM_NUMBER") {
            if !val.is_empty() {
                return Some(val);
            }
        }
        match crate::core::runtime::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager
                .get_custom_secret("twilio_from_number")
                .ok()
                .flatten(),
            Err(_) => None,
        }
    }

    /// Get the authenticated Account SID or return an error
    fn require_sid(&self) -> Result<String> {
        Self::load_account_sid_from(&self.config_dir)
            .ok_or_else(|| anyhow!("Twilio Account SID not configured"))
    }

    /// Get the authenticated Auth Token or return an error
    fn require_token(&self) -> Result<String> {
        Self::load_auth_token_from(&self.config_dir)
            .ok_or_else(|| anyhow!("Twilio Auth Token not configured"))
    }

    /// Get the configured From number or return an error
    fn require_from(&self) -> Result<String> {
        Self::load_from_number_from(&self.config_dir)
            .ok_or_else(|| anyhow!("Twilio From number not configured"))
    }

    /// Make an outbound voice call
    ///
    /// The call will use TwiML `<Say>` to speak the message, or a TwiML URL if provided.
    async fn call(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let sid = self.require_sid()?;
        let token = self.require_token()?;
        let from = self.require_from()?;

        let to = params
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'to' parameter (phone number)"))?;

        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Hello from {}", crate::branding::PRODUCT_NAME));

        // Build the TwiML inline or use a URL
        let twiml_url = params.get("twiml_url").and_then(|v| v.as_str());

        let url = format!("{}/Accounts/{}/Calls.json", Self::API_BASE, sid);

        let mut form = vec![("From", from.to_string()), ("To", to.to_string())];

        if let Some(twiml_url) = twiml_url {
            form.push(("Url", twiml_url.to_string()));
        } else {
            let twiml = format!("<Response><Say>{}</Say></Response>", message);
            form.push(("Twiml", twiml));
        }

        let response = self
            .http
            .post(&url)
            .basic_auth(&sid, Some(&token))
            .form(&form)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Twilio call failed: {}", error_text);
            return Err(anyhow!("Failed to initiate call: {}", error_text));
        }

        #[derive(Deserialize)]
        struct CallResponse {
            sid: String,
            status: String,
        }

        let result: CallResponse = response.json().await?;

        Ok(serde_json::json!({
            "call_sid": result.sid,
            "status": result.status,
            "to": to,
            "from": from,
        }))
    }

    /// Send an SMS message
    async fn sms(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let sid = self.require_sid()?;
        let token = self.require_token()?;
        let from = self.require_from()?;

        let to = params
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'to' parameter (phone number)"))?;

        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'body' parameter (message text)"))?;

        let url = format!("{}/Accounts/{}/Messages.json", Self::API_BASE, sid);

        let form = vec![
            ("From", from.clone()),
            ("To", to.to_string()),
            ("Body", body.to_string()),
        ];

        let response = self
            .http
            .post(&url)
            .basic_auth(&sid, Some(&token))
            .form(&form)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Twilio SMS failed: {}", error_text);
            return Err(anyhow!("Failed to send SMS: {}", error_text));
        }

        #[derive(Deserialize)]
        struct MessageResponse {
            sid: String,
            status: String,
        }

        let result: MessageResponse = response.json().await?;

        Ok(serde_json::json!({
            "message_sid": result.sid,
            "status": result.status,
            "to": to,
            "from": from,
        }))
    }

    /// List recent outbound and inbound calls
    async fn list_calls(&self, _params: &serde_json::Value) -> Result<serde_json::Value> {
        let sid = self.require_sid()?;
        let token = self.require_token()?;

        let url = format!("{}/Accounts/{}/Calls.json?PageSize=20", Self::API_BASE, sid);

        let response = self
            .http
            .get(&url)
            .basic_auth(&sid, Some(&token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Twilio list_calls failed: {}", error_text);
            return Err(anyhow!("Failed to list calls: {}", error_text));
        }

        #[derive(Deserialize)]
        struct CallsResponse {
            calls: Vec<CallRecord>,
        }

        #[derive(Deserialize)]
        struct CallRecord {
            sid: String,
            from: Option<String>,
            to: Option<String>,
            status: String,
            duration: Option<String>,
            date_created: Option<String>,
        }

        let result: CallsResponse = response.json().await?;

        let calls: Vec<serde_json::Value> = result
            .calls
            .into_iter()
            .map(|c| {
                serde_json::json!({
                    "sid": c.sid,
                    "from": c.from,
                    "to": c.to,
                    "status": c.status,
                    "duration": c.duration,
                    "date_created": c.date_created,
                })
            })
            .collect();

        Ok(serde_json::json!({ "calls": calls }))
    }

    /// List recent SMS messages
    async fn list_messages(&self, _params: &serde_json::Value) -> Result<serde_json::Value> {
        let sid = self.require_sid()?;
        let token = self.require_token()?;

        let url = format!(
            "{}/Accounts/{}/Messages.json?PageSize=20",
            Self::API_BASE,
            sid
        );

        let response = self
            .http
            .get(&url)
            .basic_auth(&sid, Some(&token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            tracing::warn!("Twilio list_messages failed: {}", error_text);
            return Err(anyhow!("Failed to list messages: {}", error_text));
        }

        #[derive(Deserialize)]
        struct MessagesResponse {
            messages: Vec<MessageRecord>,
        }

        #[derive(Deserialize)]
        struct MessageRecord {
            sid: String,
            from: Option<String>,
            to: Option<String>,
            body: Option<String>,
            status: String,
            date_sent: Option<String>,
        }

        let result: MessagesResponse = response.json().await?;

        let messages: Vec<serde_json::Value> = result
            .messages
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "sid": m.sid,
                    "from": m.from,
                    "to": m.to,
                    "body": m.body,
                    "status": m.status,
                    "date_sent": m.date_sent,
                })
            })
            .collect();

        Ok(serde_json::json!({ "messages": messages }))
    }
}

#[async_trait]
impl Integration for TwilioConnector {
    fn id(&self) -> &str {
        "twilio"
    }

    fn name(&self) -> &str {
        "Twilio Voice & SMS"
    }

    fn description(&self) -> &str {
        "Make phone calls and send SMS messages via Twilio"
    }

    fn icon(&self) -> &str {
        "📞"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Notify]
    }

    async fn status(&self) -> IntegrationStatus {
        if Self::load_account_sid_from(&self.config_dir).is_none()
            || Self::load_auth_token_from(&self.config_dir).is_none()
        {
            return IntegrationStatus::NotConfigured;
        }

        if Self::load_from_number_from(&self.config_dir).is_none() {
            return IntegrationStatus::Error("From phone number not configured".to_string());
        }

        IntegrationStatus::Connected
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "call" => self.call(params).await,
            "sms" => self.sms(params).await,
            "list_calls" => self.list_calls(params).await,
            "list_messages" => self.list_messages(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for TwilioConnector {
    fn default() -> Self {
        Self::new()
    }
}
