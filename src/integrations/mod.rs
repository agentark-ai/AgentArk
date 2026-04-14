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
pub mod lightpanda;
pub mod media_gen;
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
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

const INTEGRATION_STATUS_TIMEOUT: Duration = Duration::from_secs(4);

fn integration_action_is_read_only(action: &str) -> bool {
    let normalized = action.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    let tokens = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    [
        "get", "list", "read", "fetch", "search", "status", "feed", "preview", "inspect",
        "discover", "check", "test", "validate", "health",
    ]
    .iter()
    .any(|keyword| {
        tokens.first().copied() == Some(*keyword)
            || tokens.last().copied() == Some(*keyword)
            || normalized == *keyword
    })
}

fn integration_action_requires_outbound_gate(integration: &dyn Integration, action: &str) -> bool {
    if integration_action_is_read_only(action) {
        return false;
    }

    let normalized = action.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    let tokens = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if [
        "notify", "send", "message", "post", "create", "comment", "reply", "publish", "update",
        "delete", "upvote", "react", "submit", "register", "deliver", "write",
    ]
    .iter()
    .any(|keyword| tokens.iter().any(|token| token == keyword) || normalized == *keyword)
    {
        return true;
    }

    let capabilities = integration.capabilities();
    capabilities.contains(&Capability::Write)
        || capabilities.contains(&Capability::Notify)
        || capabilities.contains(&Capability::Delete)
}

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

pub fn integration_enabled_key(id: &str) -> String {
    format!("integration_enabled:{}", id)
}

pub fn integration_user_disabled_key(id: &str) -> String {
    format!("integration_user_disabled:{}", id)
}

fn stored_bool_secret(
    manager: &crate::core::config::SecureConfigManager,
    key: &str,
) -> Option<bool> {
    manager
        .get_custom_secret(key)
        .ok()
        .flatten()
        .and_then(|value| parse_boolish(&value))
}

fn builtin_integration_is_connected(
    config_dir: &Path,
    _manager: &crate::core::config::SecureConfigManager,
    integration_id: &str,
) -> bool {
    match integration_id {
        "gmail" => crate::actions::google_workspace::granted_bundles(config_dir)
            .map(|granted| granted.iter().any(|bundle| bundle == "gmail"))
            .unwrap_or(false),
        "google_calendar" => crate::actions::google_workspace::granted_bundles(config_dir)
            .map(|granted| granted.iter().any(|bundle| bundle == "calendar"))
            .unwrap_or(false),
        "google_workspace" => {
            crate::actions::google_workspace::summarize_connection_status(config_dir)
                .map(|(connected, granted, missing)| {
                    connected && !granted.is_empty() && missing.is_empty()
                })
                .unwrap_or(false)
        }
        _ => false,
    }
}

fn configured_secret_present(
    manager: &crate::core::config::SecureConfigManager,
    user_key: &str,
) -> bool {
    if std::env::var(user_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }

    crate::core::secrets::storage_keys_for_user_key(user_key)
        .into_iter()
        .any(|key| {
            manager
                .get_custom_secret(&key)
                .ok()
                .flatten()
                .is_some_and(|value| !value.trim().is_empty())
        })
}

fn external_integration_is_connected(
    _config_dir: &Path,
    manager: &crate::core::config::SecureConfigManager,
    integration_id: &str,
) -> Option<bool> {
    let spec = crate::core::connect_flow::spec_by_id(integration_id)?;
    let connected = match spec.required.kind {
        crate::core::connect_flow::SecretRequirementKind::All => spec
            .required
            .keys
            .iter()
            .all(|key| configured_secret_present(manager, key)),
        crate::core::connect_flow::SecretRequirementKind::Any => spec
            .required
            .keys
            .iter()
            .any(|key| configured_secret_present(manager, key)),
    };
    Some(connected)
}

pub fn effective_integration_enabled(config_dir: &Path, integration_id: &str) -> bool {
    let Ok(manager) = crate::core::config::SecureConfigManager::new(config_dir) else {
        return false;
    };

    if stored_bool_secret(&manager, &integration_user_disabled_key(integration_id)).unwrap_or(false)
    {
        return false;
    }

    let explicit = stored_bool_secret(&manager, &integration_enabled_key(integration_id));
    if !matches!(
        integration_id,
        "gmail" | "google_calendar" | "google_workspace"
    ) {
        let Some(connected) =
            external_integration_is_connected(config_dir, &manager, integration_id)
        else {
            return explicit.unwrap_or(true);
        };
        if !connected {
            return false;
        }
        return explicit.unwrap_or(true);
    }

    if builtin_integration_is_connected(config_dir, &manager, integration_id) {
        let _ = manager.set_custom_secret(
            &integration_enabled_key(integration_id),
            Some("true".to_string()),
        );
        return true;
    }

    false
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

    fn register(&mut self, integration: impl Integration + 'static) {
        let id = integration.id().to_string();
        self.integrations.insert(id, Box::new(integration));
    }

    /// Register default integrations (Google Calendar, WhatsApp, Media Gen)
    fn register_default_integrations(&mut self) {
        let config_dir = self._config_dir.clone();

        // Register Google Calendar
        self.register(calendar::GoogleCalendarConnector::new());

        // Register WhatsApp
        self.register(whatsapp::WhatsAppConnector::new());

        // Register AI Media Generation (Image/Video)
        self.register(media_gen::MediaGenConnector::new());

        // Register GitHub
        self.register(github::GitHubConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register Notion
        self.register(notion::NotionConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register Twitter/X
        self.register(twitter::TwitterConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register 1Password
        self.register(onepassword::OnePasswordConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register Google Places
        self.register(places::GooglePlacesConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register Twilio Voice & SMS
        self.register(twilio::TwilioConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register Ordering & Purchasing
        self.register(ordering::OrderingConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        // Register Browser Automation (Playwright sidecar)
        self.register(browser::BrowserIntegration::new());

        // Curated health + analytics connectors
        self.register(garmin::GarminConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        self.register(whoop::WhoopConnector::new_with_config_dir(
            config_dir.clone(),
        ));

        self.register(ga4::Ga4Connector::new_with_config_dir(config_dir.clone()));

        self.register(gsc::GscConnector::new_with_config_dir(config_dir.clone()));

        self.register(social_analytics::SocialAnalyticsConnector::new_with_config_dir(config_dir));

        // Register Moltbook
        self.register(moltbook::MoltbookConnector::new_with_config_dir(
            self._config_dir.clone(),
        ));
    }

    /// Get an integration by ID
    pub fn get(&self, id: &str) -> Option<&dyn Integration> {
        self.integrations.get(id).map(|i| i.as_ref())
    }

    /// List registered integration IDs.
    pub fn ids(&self) -> Vec<String> {
        self.integrations.keys().cloned().collect()
    }

    /// Returns true when an integration is ready for agent dispatch.
    /// Setup-dependent integrations stay hidden until they have valid configuration.
    pub fn is_enabled(&self, integration_id: &str) -> bool {
        effective_integration_enabled(&self._config_dir, integration_id)
    }

    /// Returns true when an integration is both enabled and currently usable.
    pub async fn is_ready(&self, integration_id: &str) -> bool {
        if !effective_integration_enabled(&self._config_dir, integration_id) {
            return false;
        }

        if matches!(
            integration_id,
            "gmail" | "google_calendar" | "google_workspace"
        ) {
            return true;
        }

        let Some(integration) = self.integrations.get(integration_id) else {
            return false;
        };
        matches!(integration.status().await, IntegrationStatus::Connected)
    }

    /// List integration IDs that are both enabled and currently ready to use.
    pub async fn ready_ids(&self) -> Vec<String> {
        let mut ready = Vec::new();
        for id in self.integrations.keys() {
            if self.is_ready(id).await {
                ready.push(id.clone());
            }
        }
        ready
    }

    /// List all integrations with their status
    pub async fn list(&self) -> Vec<IntegrationInfo> {
        join_all(
            self.integrations
                .iter()
                .map(|(id, integration)| async move {
                    let status = match tokio::time::timeout(
                        INTEGRATION_STATUS_TIMEOUT,
                        integration.status(),
                    )
                    .await
                    {
                        Ok(status) => status,
                        Err(_) => IntegrationStatus::Error("Status check timed out".to_string()),
                    };
                    IntegrationInfo {
                        id: id.clone(),
                        name: integration.name().to_string(),
                        description: integration.description().to_string(),
                        icon: integration.icon().to_string(),
                        capabilities: integration.capabilities(),
                        status,
                    }
                }),
        )
        .await
    }

    /// Return all connected integrations that support notifications
    pub async fn notifiable_integrations(&self) -> Vec<String> {
        let mut result = Vec::new();
        for (id, integration) in &self.integrations {
            if integration.capabilities().contains(&Capability::Notify)
                && self.is_enabled(id)
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

        let sanitized_params =
            if integration_action_requires_outbound_gate(integration.as_ref(), action) {
                let privacy = crate::security::sanitize_outbound_json(
                    params,
                    &crate::security::OutboundPrivacyPolicy::default(),
                );
                match privacy.decision {
                    crate::security::OutboundPrivacyDecision::Allow => params.clone(),
                    crate::security::OutboundPrivacyDecision::RedactedAllow => {
                        tracing::warn!(
                            integration_id = integration_id,
                            action = action,
                            redactions = ?privacy.redactions,
                            reasons = ?privacy.reasons,
                            "Outbound privacy gate redacted integration payload"
                        );
                        privacy.sanitized_value
                    }
                    crate::security::OutboundPrivacyDecision::Block => {
                        anyhow::bail!(
                            "{}",
                            crate::security::format_outbound_privacy_block(
                                &format!("integration '{}:{}'", integration_id, action),
                                &privacy.reasons,
                            )
                        );
                    }
                }
            } else {
                params.clone()
            };

        integration.execute(action, &sanitized_params).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_integration_enabled_autoheals_connected_google_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager =
            crate::core::config::SecureConfigManager::new(dir.path()).expect("secure manager");
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
                Some(
                    serde_json::json!({
                        "access_token": "access",
                        "refresh_token": "refresh",
                        "expires_at": chrono::Utc::now().timestamp() + 3600,
                        "granted_scopes": [
                            "https://www.googleapis.com/auth/gmail.readonly",
                            "https://www.googleapis.com/auth/gmail.send",
                            "https://www.googleapis.com/auth/calendar"
                        ],
                        "granted_bundles": ["gmail", "calendar"]
                    })
                    .to_string(),
                ),
            )
            .expect("workspace tokens saved");
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
                Some(serde_json::json!(["gmail", "calendar"]).to_string()),
            )
            .expect("workspace bundles saved");
        manager
            .set_custom_secret(
                &integration_enabled_key("google_workspace"),
                Some("false".to_string()),
            )
            .expect("stale disabled flag saved");

        assert!(effective_integration_enabled(
            dir.path(),
            "google_workspace"
        ));
        assert_eq!(
            manager
                .get_custom_secret(&integration_enabled_key("google_workspace"))
                .expect("load enabled flag")
                .as_deref(),
            Some("true")
        );
    }

    #[test]
    fn effective_integration_enabled_respects_manual_disable_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager =
            crate::core::config::SecureConfigManager::new(dir.path()).expect("secure manager");
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
                Some(
                    serde_json::json!({
                        "access_token": "access",
                        "refresh_token": "refresh",
                        "expires_at": chrono::Utc::now().timestamp() + 3600,
                        "granted_scopes": [
                            "https://www.googleapis.com/auth/gmail.readonly",
                            "https://www.googleapis.com/auth/gmail.send"
                        ],
                        "granted_bundles": ["gmail"]
                    })
                    .to_string(),
                ),
            )
            .expect("workspace tokens saved");
        manager
            .set_custom_secret(
                &integration_user_disabled_key("google_workspace"),
                Some("true".to_string()),
            )
            .expect("manual disable marker saved");
        manager
            .set_custom_secret(
                &integration_enabled_key("google_workspace"),
                Some("false".to_string()),
            )
            .expect("disabled flag saved");

        assert!(!effective_integration_enabled(
            dir.path(),
            "google_workspace"
        ));
        assert_eq!(
            manager
                .get_custom_secret(&integration_enabled_key("google_workspace"))
                .expect("load enabled flag")
                .as_deref(),
            Some("false")
        );
    }

    #[test]
    fn effective_integration_enabled_defaults_false_for_disconnected_builtin_integrations() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(!effective_integration_enabled(dir.path(), "gmail"));
        assert!(!effective_integration_enabled(
            dir.path(),
            "google_calendar"
        ));
        assert!(!effective_integration_enabled(
            dir.path(),
            "google_workspace"
        ));
    }

    #[test]
    fn effective_integration_enabled_defaults_false_for_disconnected_external_integrations() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(!effective_integration_enabled(dir.path(), "google_places"));
        assert!(!effective_integration_enabled(dir.path(), "twilio"));
    }

    #[test]
    fn effective_integration_enabled_requires_ready_external_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager =
            crate::core::config::SecureConfigManager::new(dir.path()).expect("secure manager");
        manager
            .set_custom_secret("google_places_api_key", Some("test-key".to_string()))
            .expect("places key saved");

        assert!(effective_integration_enabled(dir.path(), "google_places"));

        manager
            .set_custom_secret(
                &integration_enabled_key("google_places"),
                Some("false".to_string()),
            )
            .expect("places disable flag saved");

        assert!(!effective_integration_enabled(dir.path(), "google_places"));
    }

    #[tokio::test]
    async fn ready_ids_hide_unconfigured_external_connectors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = IntegrationManager::new(dir.path());
        let ready = manager.ready_ids().await;
        assert!(!ready.iter().any(|id| id == "google_places"));

        let secure =
            crate::core::config::SecureConfigManager::new(dir.path()).expect("secure manager");
        secure
            .set_custom_secret("google_places_api_key", Some("test-key".to_string()))
            .expect("places key saved");

        let manager = IntegrationManager::new(dir.path());
        let ready = manager.ready_ids().await;
        assert!(ready.iter().any(|id| id == "google_places"));
    }

    #[tokio::test]
    async fn is_ready_accepts_connected_google_workspace_surfaces() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager =
            crate::core::config::SecureConfigManager::new(dir.path()).expect("secure manager");
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
                Some(
                    serde_json::json!({
                        "access_token": "access",
                        "refresh_token": "refresh",
                        "expires_at": chrono::Utc::now().timestamp() + 3600,
                        "granted_scopes": [
                            "https://www.googleapis.com/auth/gmail.readonly",
                            "https://www.googleapis.com/auth/gmail.send",
                            "https://www.googleapis.com/auth/calendar"
                        ],
                        "granted_bundles": ["gmail", "calendar"]
                    })
                    .to_string(),
                ),
            )
            .expect("workspace tokens saved");
        manager
            .set_custom_secret(
                crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
                Some(serde_json::json!(["gmail", "calendar"]).to_string()),
            )
            .expect("workspace bundles saved");

        let integrations = IntegrationManager::new(dir.path());
        assert!(integrations.is_ready("gmail").await);
        assert!(integrations.is_ready("google_calendar").await);
        assert!(integrations.is_ready("google_workspace").await);
    }
}
