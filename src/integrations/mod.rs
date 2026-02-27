//! External Service Integrations
//!
//! Connects AgentArk to external services like Google Calendar, WhatsApp, etc.
//! Each integration implements the `Integration` trait for unified handling.

pub mod browser;
pub mod calendar;
pub mod ga4;
pub mod garmin;
pub mod github;
pub mod gsc;
pub mod media_gen;
pub mod mem0;
pub mod moltbook;
pub mod notion;
pub mod oauth;
pub mod onepassword;
pub mod ordering;
pub mod places;
pub mod social_analytics;
pub mod twilio;
pub mod twitter;
pub mod whatsapp;
pub mod whoop;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn parse_boolish(value: &str) -> Option<bool> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() {
        return None;
    }
    match v.as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

/// Capabilities an integration can provide
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Can read data
    Read,
    /// Can write/create data
    Write,
    /// Can subscribe to updates (webhooks)
    Subscribe,
    /// Can search/query data
    Search,
    /// Can delete data
    Delete,
    /// Can send notifications/messages to the user
    Notify,
}

/// Integration status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IntegrationStatus {
    /// Not configured
    NotConfigured,
    /// Configured but not connected (needs OAuth)
    NeedsAuth,
    /// Connected and working
    Connected,
    /// Connection error
    Error(String),
}

/// Base trait for all integrations
#[async_trait]
#[allow(dead_code)]
pub trait Integration: Send + Sync {
    /// Unique identifier for this integration
    fn id(&self) -> &str;

    /// Human-readable name
    fn name(&self) -> &str;

    /// Description of what this integration does
    fn description(&self) -> &str;

    /// Icon/emoji for UI
    fn icon(&self) -> &str;

    /// What this integration can do
    fn capabilities(&self) -> Vec<Capability>;

    /// Current status
    async fn status(&self) -> IntegrationStatus;

    /// Check if the integration is ready to use
    async fn is_connected(&self) -> bool {
        matches!(self.status().await, IntegrationStatus::Connected)
    }

    /// Execute an action
    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value>;

    /// Handle incoming webhook (if supported)
    async fn handle_webhook(&self, _payload: &serde_json::Value) -> Result<()> {
        Ok(()) // Default: no-op
    }
}

/// Integration manager - holds all configured integrations
pub struct IntegrationManager {
    integrations: HashMap<String, Box<dyn Integration>>,
    _config_dir: std::path::PathBuf,
}

impl IntegrationManager {
    pub fn new(config_dir: &std::path::Path) -> Self {
        let mut manager = Self {
            integrations: HashMap::new(),
            _config_dir: config_dir.to_path_buf(),
        };

        // Register available integrations
        manager.register_default_integrations();
        manager
    }

    /// Register default integrations (Google Calendar, WhatsApp, Media Gen)
    fn register_default_integrations(&mut self) {
        let config_dir = self._config_dir.clone();

        // Register Google Calendar
        let calendar = calendar::GoogleCalendarConnector::new();
        self.integrations
            .insert("google_calendar".to_string(), Box::new(calendar));

        // Register WhatsApp
        let whatsapp = whatsapp::WhatsAppConnector::new();
        self.integrations
            .insert("whatsapp".to_string(), Box::new(whatsapp));

        // Register AI Media Generation (Image/Video)
        let media_gen = media_gen::MediaGenConnector::new();
        self.integrations
            .insert("media_gen".to_string(), Box::new(media_gen));

        // Register GitHub
        let github = github::GitHubConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("github".to_string(), Box::new(github));

        // Register Notion
        let notion = notion::NotionConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("notion".to_string(), Box::new(notion));

        // Register Twitter/X
        let twitter = twitter::TwitterConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("twitter".to_string(), Box::new(twitter));

        // Register 1Password
        let onepassword =
            onepassword::OnePasswordConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("onepassword".to_string(), Box::new(onepassword));

        // Register Google Places
        let places = places::GooglePlacesConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("google_places".to_string(), Box::new(places));

        // Register Twilio Voice & SMS
        let twilio = twilio::TwilioConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("twilio".to_string(), Box::new(twilio));

        // Register Ordering & Purchasing
        let ordering = ordering::OrderingConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("ordering".to_string(), Box::new(ordering));

        // Register Browser Automation (Playwright sidecar)
        let browser = browser::BrowserIntegration::new();
        self.integrations
            .insert("browser".to_string(), Box::new(browser));

        // Curated health + analytics connectors
        let garmin = garmin::GarminConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("garmin".to_string(), Box::new(garmin));

        let whoop = whoop::WhoopConnector::new_with_config_dir(config_dir.clone());
        self.integrations
            .insert("whoop".to_string(), Box::new(whoop));

        let ga4 = ga4::Ga4Connector::new_with_config_dir(config_dir.clone());
        self.integrations.insert("ga4".to_string(), Box::new(ga4));

        let gsc = gsc::GscConnector::new_with_config_dir(config_dir.clone());
        self.integrations.insert("gsc".to_string(), Box::new(gsc));

        let social = social_analytics::SocialAnalyticsConnector::new_with_config_dir(config_dir);
        self.integrations
            .insert("social_analytics".to_string(), Box::new(social));

        // Register Moltbook
        let moltbook = moltbook::MoltbookConnector::new_with_config_dir(self._config_dir.clone());
        self.integrations
            .insert("moltbook".to_string(), Box::new(moltbook));
    }

    /// Get an integration by ID
    pub fn get(&self, id: &str) -> Option<&dyn Integration> {
        self.integrations.get(id).map(|i| i.as_ref())
    }

    /// List registered integration IDs.
    pub fn ids(&self) -> Vec<String> {
        self.integrations.keys().cloned().collect()
    }

    fn enabled_overrides(&self) -> HashMap<String, bool> {
        let mut enabled = HashMap::new();
        let Ok(manager) = crate::core::config::SecureConfigManager::new(&self._config_dir) else {
            return enabled;
        };
        let Ok(secrets) = manager.load_secrets() else {
            return enabled;
        };
        for (key, value) in secrets.custom {
            let Some(id) = key.strip_prefix("integration_enabled:") else {
                continue;
            };
            if let Some(parsed) = parse_boolish(&value) {
                enabled.insert(id.to_string(), parsed);
            }
        }
        enabled
    }

    /// Returns true when an integration is enabled for agent dispatch.
    /// Missing/invalid flags default to enabled (matching execute-time behavior).
    pub fn is_enabled(&self, integration_id: &str) -> bool {
        self.enabled_overrides()
            .get(integration_id)
            .copied()
            .unwrap_or(true)
    }

    /// List integration IDs that are currently enabled for agent dispatch.
    pub fn enabled_ids(&self) -> Vec<String> {
        let overrides = self.enabled_overrides();
        self.integrations
            .keys()
            .filter(|id| overrides.get(id.as_str()).copied().unwrap_or(true))
            .cloned()
            .collect()
    }

    /// List all integrations with their status
    pub async fn list(&self) -> Vec<IntegrationInfo> {
        let mut result = Vec::new();
        for (id, integration) in &self.integrations {
            result.push(IntegrationInfo {
                id: id.clone(),
                name: integration.name().to_string(),
                description: integration.description().to_string(),
                icon: integration.icon().to_string(),
                capabilities: integration.capabilities(),
                status: integration.status().await,
            });
        }
        result
    }

    /// Return all connected integrations that support notifications
    pub async fn notifiable_integrations(&self) -> Vec<String> {
        let mut result = Vec::new();
        for (id, integration) in &self.integrations {
            if integration.capabilities().contains(&Capability::Notify)
                && integration.is_connected().await
            {
                result.push(id.clone());
            }
        }
        result
    }

    /// Execute an action on an integration
    pub async fn execute(
        &self,
        integration_id: &str,
        action: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Enforce user enable/disable toggle (stored in encrypted secrets).
        if !self.is_enabled(integration_id) {
            return Err(anyhow::anyhow!(
                "Integration '{}' is disabled",
                integration_id
            ));
        }

        let integration = self
            .integrations
            .get(integration_id)
            .ok_or_else(|| anyhow::anyhow!("Integration '{}' not found", integration_id))?;

        integration.execute(action, params).await
    }
}

/// Info about an integration for API/UI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub capabilities: Vec<Capability>,
    pub status: IntegrationStatus,
}
