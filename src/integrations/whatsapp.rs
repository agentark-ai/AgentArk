//! WhatsApp Business Integration
//!
//! Provides WhatsApp messaging via Meta Business API.
//! Supports sending messages, receiving webhooks, and managing templates.

use super::oauth::{OAuthClient, OAuthConfig, OAuthTokens, TokenStorage};
use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Template component for dynamic content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    pub parameters: Vec<TemplateParameter>,
}

/// Template parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TemplateParameter {
    Text { text: String },
    Image { link: String },
    Document { link: String },
}

/// WhatsApp Business connector
pub struct WhatsAppConnector {
    oauth_config: Option<OAuthConfig>,
    tokens: Arc<RwLock<Option<OAuthTokens>>>,
    token_storage: Option<TokenStorage>,
    /// WhatsApp Business Account ID
    waba_id: Option<String>,
    /// Phone number ID for sending messages
    phone_number_id: Option<String>,
    http: reqwest::Client,
    oauth_client: OAuthClient,
}

impl WhatsAppConnector {
    const SERVICE_ID: &'static str = "whatsapp";
    const API_BASE: &'static str = "https://graph.facebook.com/v18.0";

    pub fn new() -> Self {
        Self {
            oauth_config: None,
            tokens: Arc::new(RwLock::new(None)),
            token_storage: None,
            waba_id: None,
            phone_number_id: None,
            http: crate::core::net::default_outgoing_http_client(),
            oauth_client: OAuthClient::new(),
        }
    }

    /// Get the OAuth authorization URL
    pub fn get_auth_url(&self, state: &str) -> Result<String> {
        let config = self
            .oauth_config
            .as_ref()
            .ok_or_else(|| anyhow!("OAuth not configured"))?;
        Ok(config.auth_url(state))
    }

    /// Handle OAuth callback with authorization code
    pub async fn handle_auth_callback(&self, code: &str) -> Result<()> {
        let config = self
            .oauth_config
            .as_ref()
            .ok_or_else(|| anyhow!("OAuth not configured"))?;

        let tokens = self.oauth_client.exchange_code(config, code).await?;

        // Save tokens
        if let Some(ref storage) = self.token_storage {
            storage.save_async(Self::SERVICE_ID, &tokens).await?;
        }

        *self.tokens.write().await = Some(tokens);

        // Fetch and store WABA ID and phone number
        self.fetch_business_info().await?;

        Ok(())
    }

    /// Fetch WhatsApp Business Account info after OAuth
    async fn fetch_business_info(&self) -> Result<()> {
        let token = self.get_access_token().await?;

        // Get user's WhatsApp Business Accounts
        let url = format!("{}/me/businesses", Self::API_BASE);
        let response = self.http.get(&url).bearer_auth(&token).send().await?;

        if !response.status().is_success() {
            tracing::warn!("Failed to fetch WhatsApp business info");
        }

        // In production, parse response and store WABA ID and phone number ID

        Ok(())
    }

    /// Disconnect (revoke tokens)
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(ref storage) = self.token_storage {
            storage.delete_async(Self::SERVICE_ID).await?;
        }
        *self.tokens.write().await = None;
        Ok(())
    }

    /// Get a valid access token
    async fn get_access_token(&self) -> Result<String> {
        let mut tokens_guard = self.tokens.write().await;
        let tokens = tokens_guard
            .as_mut()
            .ok_or_else(|| anyhow!("Not authenticated with WhatsApp"))?;

        // Check if token needs refresh
        if tokens.is_expired() {
            if let Some(refresh_token) = tokens.refresh_token() {
                let config = self
                    .oauth_config
                    .as_ref()
                    .ok_or_else(|| anyhow!("OAuth not configured"))?;

                let new_tokens = self
                    .oauth_client
                    .refresh_token(config, refresh_token)
                    .await?;

                if let Some(ref storage) = self.token_storage {
                    storage.save_async(Self::SERVICE_ID, &new_tokens).await?;
                }

                *tokens = new_tokens;
            } else {
                return Err(anyhow!("Token expired and no refresh token available"));
            }
        }

        Ok(tokens.access_token().to_string())
    }

    /// Send a text message
    pub async fn send_text(&self, to: &str, message: &str) -> Result<String> {
        let phone_id = self
            .phone_number_id
            .as_ref()
            .ok_or_else(|| anyhow!("Phone number ID not configured"))?;

        let token = self.get_access_token().await?;

        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            "text": {
                "body": message
            }
        });

        let url = format!("{}/{}/messages", Self::API_BASE, phone_id);

        let response = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to send message: {}", error_text));
        }

        #[derive(Deserialize)]
        struct SendResponse {
            messages: Vec<MessageId>,
        }

        #[derive(Deserialize)]
        struct MessageId {
            id: String,
        }

        let result: SendResponse = response.json().await?;
        Ok(result
            .messages
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_default())
    }

    /// Send a template message (for business-initiated conversations)
    pub async fn send_template(
        &self,
        to: &str,
        template_name: &str,
        language: &str,
        components: Option<Vec<TemplateComponent>>,
    ) -> Result<String> {
        let phone_id = self
            .phone_number_id
            .as_ref()
            .ok_or_else(|| anyhow!("Phone number ID not configured"))?;

        let token = self.get_access_token().await?;

        let mut template = serde_json::json!({
            "name": template_name,
            "language": {
                "code": language
            }
        });

        if let Some(comps) = components {
            template["components"] = serde_json::to_value(comps)?;
        }

        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "template",
            "template": template
        });

        let url = format!("{}/{}/messages", Self::API_BASE, phone_id);

        let response = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to send template: {}", error_text));
        }

        #[derive(Deserialize)]
        struct SendResponse {
            messages: Vec<MessageId>,
        }

        #[derive(Deserialize)]
        struct MessageId {
            id: String,
        }

        let result: SendResponse = response.json().await?;
        Ok(result
            .messages
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_default())
    }

    /// Get message templates
    pub async fn list_templates(&self) -> Result<Vec<serde_json::Value>> {
        let waba_id = self
            .waba_id
            .as_ref()
            .ok_or_else(|| anyhow!("WABA ID not configured"))?;

        let token = self.get_access_token().await?;

        let url = format!("{}/{}/message_templates", Self::API_BASE, waba_id);

        let response = self.http.get(&url).bearer_auth(&token).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to list templates: {}", error_text));
        }

        #[derive(Deserialize)]
        struct TemplatesResponse {
            data: Vec<serde_json::Value>,
        }

        let result: TemplatesResponse = response.json().await?;
        Ok(result.data)
    }
}

#[async_trait]
impl Integration for WhatsAppConnector {
    fn id(&self) -> &str {
        Self::SERVICE_ID
    }

    fn name(&self) -> &str {
        "WhatsApp Business"
    }

    fn description(&self) -> &str {
        "Send and receive WhatsApp messages via Meta Business API"
    }

    fn icon(&self) -> &str {
        "📱"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability::Read,
            Capability::Write,
            Capability::Subscribe,
            Capability::Notify,
        ]
    }

    async fn status(&self) -> IntegrationStatus {
        if self.oauth_config.is_none() {
            return IntegrationStatus::NotConfigured;
        }

        let tokens = self.tokens.read().await;
        if tokens.is_none() {
            return IntegrationStatus::NeedsAuth;
        }

        if self.phone_number_id.is_none() {
            return IntegrationStatus::Error("Phone number not configured".to_string());
        }

        IntegrationStatus::Connected
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "send_text" => {
                let to = params
                    .get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'to' parameter"))?;
                let message = params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'message' parameter"))?;

                let msg_id = self.send_text(to, message).await?;
                Ok(serde_json::json!({ "message_id": msg_id }))
            }
            "send_template" => {
                let to = params
                    .get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'to' parameter"))?;
                let template = params
                    .get("template")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'template' parameter"))?;
                let language = params
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("en_US");

                let msg_id = self.send_template(to, template, language, None).await?;
                Ok(serde_json::json!({ "message_id": msg_id }))
            }
            "list_templates" => {
                let templates = self.list_templates().await?;
                Ok(serde_json::json!({ "templates": templates }))
            }
            "get_auth_url" => {
                let state = params
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("whatsapp");
                let url = self.get_auth_url(state)?;
                Ok(serde_json::json!({ "url": url }))
            }
            "auth_callback" => {
                let code = params
                    .get("code")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing authorization code"))?;
                self.handle_auth_callback(code).await?;
                Ok(serde_json::json!({ "status": "connected" }))
            }
            "disconnect" => {
                self.disconnect().await?;
                Ok(serde_json::json!({ "status": "disconnected" }))
            }
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for WhatsAppConnector {
    fn default() -> Self {
        Self::new()
    }
}
