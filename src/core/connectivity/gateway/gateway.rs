//! Gateway inventory and deterministic routing foundations.
//!
//! This module provides a low-risk persistence layer for milestone M1 using the
//! existing encrypted KV store instead of introducing new relational tables.

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::core::AgentConfig;
use crate::storage::Storage;

const CHANNEL_ACCOUNTS_KEY: &str = "gateway:channel_accounts:v1";
const ROUTE_RULES_KEY: &str = "gateway:route_rules:v1";
const BROADCAST_GROUPS_KEY: &str = "gateway:broadcast_groups:v1";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayChannelsSummary {
    pub supported: usize,
    pub configured: usize,
    pub connected: usize,
    pub attention_needed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelDescriptor {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub enabled: bool,
    pub configured: bool,
    pub supports_pairing: bool,
    pub supports_threads: bool,
    pub supports_groups: bool,
    pub supports_broadcast: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_mode: Option<String>,
    #[serde(default)]
    pub account_count: usize,
    #[serde(default)]
    pub route_count: usize,
    #[serde(default)]
    pub connected_account_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelAccount {
    pub id: String,
    pub channel_id: String,
    pub label: String,
    pub enabled: bool,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayChannelsResponse {
    pub summary: GatewayChannelsSummary,
    pub channels: Vec<GatewayChannelDescriptor>,
    pub accounts: Vec<GatewayChannelAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayBroadcastGroup {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub member_count: usize,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayRouteRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub match_kind: String,
    pub match_value: String,
    pub target_kind: String,
    pub target_value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_matched_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayRoutingSummary {
    pub rules: usize,
    pub enabled_rules: usize,
    pub broadcast_groups: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayRoutingResponse {
    pub summary: GatewayRoutingSummary,
    pub rules: Vec<GatewayRouteRule>,
    pub broadcast_groups: Vec<GatewayBroadcastGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayRoutingSimulation {
    pub matched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayChannelAccountUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayRouteRuleUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayBroadcastGroupCreate {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayRoutingSimulationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_value: Option<String>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode gateway payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode gateway payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

fn channel_runtime_label(state: crate::channels::gateway::ChannelRuntimeState) -> &'static str {
    match state {
        crate::channels::gateway::ChannelRuntimeState::Planned => "planned",
        crate::channels::gateway::ChannelRuntimeState::Configured => "configured",
        crate::channels::gateway::ChannelRuntimeState::Connecting => "connecting",
        crate::channels::gateway::ChannelRuntimeState::Ready => "ready",
        crate::channels::gateway::ChannelRuntimeState::Degraded => "degraded",
        crate::channels::gateway::ChannelRuntimeState::Disabled => "disabled",
        crate::channels::gateway::ChannelRuntimeState::Error => "error",
    }
}

fn channel_delivery_mode(kind: crate::channels::gateway::ChannelTransportKind) -> &'static str {
    match kind {
        crate::channels::gateway::ChannelTransportKind::Native => "native",
        crate::channels::gateway::ChannelTransportKind::Bridge => "bridge",
        crate::channels::gateway::ChannelTransportKind::Node => "node",
        crate::channels::gateway::ChannelTransportKind::Plugin => "plugin",
        crate::channels::gateway::ChannelTransportKind::Web => "embedded",
    }
}

fn channel_capability_name(
    capability: crate::channels::gateway::ChannelCapability,
) -> &'static str {
    match capability {
        crate::channels::gateway::ChannelCapability::Inbound => "inbound",
        crate::channels::gateway::ChannelCapability::Outbound => "outbound",
        crate::channels::gateway::ChannelCapability::Realtime => "realtime",
        crate::channels::gateway::ChannelCapability::Threads => "threads",
        crate::channels::gateway::ChannelCapability::Groups => "groups",
        crate::channels::gateway::ChannelCapability::DirectMessages => "direct_messages",
        crate::channels::gateway::ChannelCapability::Attachments => "attachments",
        crate::channels::gateway::ChannelCapability::ReadReceipts => "read_receipts",
        crate::channels::gateway::ChannelCapability::Presence => "presence",
        crate::channels::gateway::ChannelCapability::Voice => "voice",
        crate::channels::gateway::ChannelCapability::ScreenShare => "screen_share",
        crate::channels::gateway::ChannelCapability::Location => "location",
        crate::channels::gateway::ChannelCapability::InteractiveButtons => "interactive_buttons",
    }
}

fn descriptor_metadata(
    descriptor: &crate::channels::gateway::ChannelAdapterDescriptor,
    status: &crate::channels::gateway::ChannelRuntimeStatus,
) -> Option<serde_json::Value> {
    Some(serde_json::json!({
        "routing_scope_hint": descriptor.routing_scope_hint.as_deref(),
        "docs_url": descriptor.docs_url.as_deref(),
        "setup_url": descriptor.setup_url.as_deref(),
        "credential_model": descriptor.credential_model.as_deref(),
        "integration_model": descriptor.integration_model.as_deref(),
        "transport": {
            "kind": channel_delivery_mode(descriptor.transport.kind),
            "description": descriptor.transport.description.as_str(),
            "bridge_name": descriptor.transport.bridge_name.as_deref(),
            "feature_flag": descriptor.transport.feature_flag.as_deref(),
        },
        "setup": &descriptor.setup,
        "notes": descriptor.notes.as_deref(),
        "runtime": {
            "state": channel_runtime_label(status.state),
            "connected": status.connected,
            "last_error": status.last_error.as_deref(),
            "last_checked_at": status.last_checked_at.as_deref(),
            "details": &status.details,
        }
    }))
}

#[allow(unreachable_patterns)]
fn default_channel_catalog(config: &AgentConfig) -> Vec<GatewayChannelDescriptor> {
    let telegram = config.telegram.as_ref();
    let slack = config.slack.as_ref();
    let discord = config.discord.as_ref();
    let matrix = config.matrix.as_ref();
    let teams = config.teams.as_ref();
    let whatsapp = config.whatsapp.as_ref();
    let google_chat = config.google_chat.as_ref();
    let signal = config.signal.as_ref();
    let imessage = config.imessage.as_ref();
    let line = config.line.as_ref();
    let wechat = config.wechat.as_ref();
    let qq = config.qq.as_ref();

    let telegram_configured = telegram
        .map(|cfg| !cfg.bot_token.trim().is_empty())
        .unwrap_or(false);
    let slack_configured = slack.map(slack_channel_ready).unwrap_or(false);
    let discord_configured = discord.map(discord_channel_ready).unwrap_or(false);
    let matrix_configured = matrix.map(matrix_channel_ready).unwrap_or(false);
    let teams_configured = teams.map(teams_channel_ready).unwrap_or(false);
    let whatsapp_configured = whatsapp.map(channel_ready).unwrap_or(false);
    let google_chat_configured = google_chat.map(google_chat_channel_ready).unwrap_or(false);
    let signal_configured = signal.map(signal_channel_ready).unwrap_or(false);
    let imessage_configured = imessage.map(imessage_channel_ready).unwrap_or(false);
    let line_configured = line.map(line_channel_ready).unwrap_or(false);
    let wechat_configured = wechat.map(wechat_channel_ready).unwrap_or(false);
    let qq_configured = qq.map(qq_channel_ready).unwrap_or(false);
    let whatsapp_mode = whatsapp.map(|cfg| match cfg.mode {
        crate::channels::whatsapp::WhatsAppMode::CloudApi => "cloud_api".to_string(),
        crate::channels::whatsapp::WhatsAppMode::Baileys => "baileys".to_string(),
    });

    let mut channels = vec![GatewayChannelDescriptor {
        id: "web".to_string(),
        kind: "web".to_string(),
        name: "Web UI".to_string(),
        description: "Built-in browser workspace for local and remote control-plane access."
            .to_string(),
        status: "ready".to_string(),
        enabled: true,
        configured: true,
        supports_pairing: false,
        supports_threads: true,
        supports_groups: false,
        supports_broadcast: false,
        delivery_mode: Some("control_plane".to_string()),
        account_count: 0,
        route_count: 0,
        connected_account_count: 0,
        last_error: None,
        docs_url: None,
        capabilities: vec![
            "chat".to_string(),
            "streaming".to_string(),
            "artifacts".to_string(),
        ],
        metadata: Some(serde_json::json!({
            "kind": "native_workspace",
            "notes": "Primary local and remote control-plane workspace."
        })),
    }];

    let registry = crate::channels::gateway::ChannelGatewayRegistry::with_config(Some(config));
    channels.extend(registry.list_statuses().into_iter().map(|view| {
        let descriptor = view.descriptor;
        let runtime = view.status;
        let id = descriptor.kind.as_str().to_string();
        let supports_threads = descriptor
            .supports(crate::channels::gateway::ChannelCapability::Threads)
            || matches!(
                descriptor.kind,
                crate::channels::gateway::ChannelKind::WebChat
            );
        let supports_groups =
            descriptor.supports(crate::channels::gateway::ChannelCapability::Groups);
        let supports_broadcast = supports_groups
            || supports_threads
            || matches!(
                descriptor.kind,
                crate::channels::gateway::ChannelKind::WebChat
            );
        let metadata = descriptor_metadata(&descriptor, &runtime);
        let capabilities = descriptor
            .capabilities
            .iter()
            .map(|capability| channel_capability_name(*capability).to_string())
            .collect::<Vec<_>>();

        match descriptor.kind {
            crate::channels::gateway::ChannelKind::WebChat => GatewayChannelDescriptor {
                id,
                kind: "webchat".to_string(),
                name: descriptor.display_name,
                description: format!(
                    "External embedded web chat surface backed by the {} gateway.",
                    crate::branding::PRODUCT_NAME
                ),
                status: "missing_config".to_string(),
                enabled: false,
                configured: false,
                supports_pairing: true,
                supports_threads: true,
                supports_groups: false,
                supports_broadcast: true,
                delivery_mode: Some("embedded".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities: {
                    let mut values = capabilities;
                    values.push("public_embed".to_string());
                    values.sort();
                    values.dedup();
                    values
                },
                metadata,
            },
            crate::channels::gateway::ChannelKind::Telegram => GatewayChannelDescriptor {
                id,
                kind: "telegram".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if telegram_configured {
                    "connected".to_string()
                } else {
                    "missing_token".to_string()
                },
                enabled: telegram.is_some(),
                configured: telegram_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("bot".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::WhatsApp => GatewayChannelDescriptor {
                id,
                kind: "whatsapp".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if whatsapp_configured {
                    "connected".to_string()
                } else if whatsapp.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: whatsapp.is_some(),
                configured: whatsapp_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: whatsapp_mode.clone(),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Slack => GatewayChannelDescriptor {
                id,
                kind: "slack".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if slack_configured {
                    "connected".to_string()
                } else if slack.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: slack.is_some(),
                configured: slack_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("webhook_api".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Discord => GatewayChannelDescriptor {
                id,
                kind: "discord".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if discord_configured {
                    "connected".to_string()
                } else if discord.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: discord.is_some(),
                configured: discord_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("gateway_rest".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Matrix => GatewayChannelDescriptor {
                id,
                kind: "matrix".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if matrix_configured {
                    "connected".to_string()
                } else if matrix.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: matrix.is_some(),
                configured: matrix_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("sync_api".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Teams => GatewayChannelDescriptor {
                id,
                kind: "teams".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if teams_configured {
                    "connected".to_string()
                } else if teams.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: teams.is_some(),
                configured: teams_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("bot_framework_graph".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::GoogleChat => GatewayChannelDescriptor {
                id,
                kind: "google_chat".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if google_chat_configured {
                    "connected".to_string()
                } else if google_chat.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: google_chat.is_some(),
                configured: google_chat_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("workspace_api".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Signal => GatewayChannelDescriptor {
                id,
                kind: "signal".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if signal_configured {
                    "connected".to_string()
                } else if signal.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: signal.is_some(),
                configured: signal_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("bridge".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::IMessage => GatewayChannelDescriptor {
                id,
                kind: "imessage".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if imessage_configured {
                    "connected".to_string()
                } else if imessage.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: imessage.is_some(),
                configured: imessage_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("bridge".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Line => GatewayChannelDescriptor {
                id,
                kind: "line".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if line_configured {
                    "connected".to_string()
                } else if line.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: line.is_some(),
                configured: line_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("webhook_api".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::WeChat => GatewayChannelDescriptor {
                id,
                kind: "wechat".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if wechat_configured {
                    "connected".to_string()
                } else if wechat.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: wechat.is_some(),
                configured: wechat_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("bridge".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            crate::channels::gateway::ChannelKind::Qq => GatewayChannelDescriptor {
                id,
                kind: "qq".to_string(),
                name: descriptor.display_name,
                description: descriptor.summary,
                status: if qq_configured {
                    "connected".to_string()
                } else if qq.is_some() {
                    "missing_token".to_string()
                } else {
                    "missing_config".to_string()
                },
                enabled: qq.is_some(),
                configured: qq_configured,
                supports_pairing: true,
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some("bridge".to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
            _ => GatewayChannelDescriptor {
                id: id.clone(),
                kind: id,
                name: descriptor.display_name,
                description: descriptor.summary,
                status: match runtime.state {
                    crate::channels::gateway::ChannelRuntimeState::Ready => "ready".to_string(),
                    crate::channels::gateway::ChannelRuntimeState::Configured
                    | crate::channels::gateway::ChannelRuntimeState::Connecting => {
                        "configured".to_string()
                    }
                    crate::channels::gateway::ChannelRuntimeState::Degraded => {
                        "degraded".to_string()
                    }
                    crate::channels::gateway::ChannelRuntimeState::Disabled => {
                        "disabled".to_string()
                    }
                    crate::channels::gateway::ChannelRuntimeState::Error => "error".to_string(),
                    crate::channels::gateway::ChannelRuntimeState::Planned => {
                        "missing_config".to_string()
                    }
                },
                enabled: !matches!(
                    runtime.state,
                    crate::channels::gateway::ChannelRuntimeState::Planned
                        | crate::channels::gateway::ChannelRuntimeState::Disabled
                ),
                configured: matches!(
                    runtime.state,
                    crate::channels::gateway::ChannelRuntimeState::Configured
                        | crate::channels::gateway::ChannelRuntimeState::Connecting
                        | crate::channels::gateway::ChannelRuntimeState::Ready
                        | crate::channels::gateway::ChannelRuntimeState::Degraded
                ),
                supports_pairing: descriptor.setup_url.is_some(),
                supports_threads,
                supports_groups,
                supports_broadcast,
                delivery_mode: Some(channel_delivery_mode(descriptor.transport.kind).to_string()),
                account_count: 0,
                route_count: 0,
                connected_account_count: 0,
                last_error: runtime.last_error,
                docs_url: descriptor.docs_url,
                capabilities,
                metadata,
            },
        }
    }));

    channels
}

fn channel_ready(config: &crate::channels::whatsapp::WhatsAppChannelConfig) -> bool {
    match config.mode {
        crate::channels::whatsapp::WhatsAppMode::CloudApi => {
            !config.access_token.trim().is_empty()
                && !config.app_secret.trim().is_empty()
                && !config.phone_number_id.trim().is_empty()
                && !config.verify_token.trim().is_empty()
        }
        crate::channels::whatsapp::WhatsAppMode::Baileys => match config.bridge_runtime() {
            crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded => true,
            crate::channels::whatsapp::WhatsAppBridgeRuntime::External => {
                !config.bridge_url.trim().is_empty()
            }
        },
    }
}

fn slack_channel_ready(config: &crate::channels::slack::SlackChannelConfig) -> bool {
    !config.bot_token.trim().is_empty() && !config.signing_secret.trim().is_empty()
}

fn discord_channel_ready(config: &crate::channels::discord::DiscordChannelConfig) -> bool {
    !config.bot_token.trim().is_empty()
}

fn matrix_channel_ready(config: &crate::channels::matrix::MatrixTransportConfig) -> bool {
    !config.homeserver_url.trim().is_empty()
        && !config.access_token.trim().is_empty()
        && !config.user_id.trim().is_empty()
}

fn teams_channel_ready(config: &crate::channels::teams::TeamsTransportConfig) -> bool {
    !config.service_url.trim().is_empty()
        && !config.access_token.trim().is_empty()
        && config
            .bot_app_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn google_chat_channel_ready(
    config: &crate::channels::google_chat::GoogleChatChannelConfig,
) -> bool {
    !config.access_token.trim().is_empty()
        && !config.verify_token.trim().is_empty()
        && config
            .space
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn signal_channel_ready(config: &crate::channels::signal::SignalChannelConfig) -> bool {
    !config.bridge_url.trim().is_empty() && !config.bridge_token.trim().is_empty()
}

fn imessage_channel_ready(config: &crate::channels::imessage::IMessageChannelConfig) -> bool {
    !config.bridge_url.trim().is_empty() && !config.bridge_token.trim().is_empty()
}

fn line_channel_ready(config: &crate::channels::line::LineChannelConfig) -> bool {
    !config.channel_access_token.trim().is_empty() && !config.channel_secret.trim().is_empty()
}

fn wechat_channel_ready(config: &crate::channels::wechat::WeChatChannelConfig) -> bool {
    !config.bridge_url.trim().is_empty() && !config.bridge_token.trim().is_empty()
}

fn qq_channel_ready(config: &crate::channels::qq::QqChannelConfig) -> bool {
    !config.bridge_url.trim().is_empty() && !config.bridge_token.trim().is_empty()
}

pub async fn load_channels(
    storage: &Storage,
    config: &AgentConfig,
) -> Result<GatewayChannelsResponse> {
    let accounts: Vec<GatewayChannelAccount> = load_json(storage, CHANNEL_ACCOUNTS_KEY).await?;
    let rules: Vec<GatewayRouteRule> = load_json(storage, ROUTE_RULES_KEY).await?;
    let mut channels = default_channel_catalog(config);

    for channel in &mut channels {
        let channel_accounts: Vec<&GatewayChannelAccount> = accounts
            .iter()
            .filter(|account| account.channel_id == channel.id)
            .collect();
        let channel_rules = rules
            .iter()
            .filter(|rule| rule.channel_id.as_deref() == Some(channel.id.as_str()))
            .count();
        channel.account_count = channel_accounts.len();
        channel.route_count = channel_rules;
        channel.connected_account_count = channel_accounts
            .iter()
            .filter(|account| {
                account.enabled
                    && matches!(
                        account.status.trim().to_ascii_lowercase().as_str(),
                        "connected" | "ready" | "syncing"
                    )
            })
            .count();
        if let Some(err) = channel_accounts
            .iter()
            .find_map(|account| account.last_error.clone())
        {
            channel.last_error = Some(err);
        }

        if channel.route_count > 0 || channel.account_count > 0 {
            channel.enabled = channel.enabled || channel.account_count > 0;
        }
    }

    let summary = GatewayChannelsSummary {
        supported: channels.len(),
        configured: channels.iter().filter(|channel| channel.configured).count(),
        connected: channels
            .iter()
            .filter(|channel| {
                matches!(
                    channel.status.as_str(),
                    "connected" | "ready" | "configured"
                )
            })
            .count(),
        attention_needed: channels
            .iter()
            .filter(|channel| {
                matches!(
                    channel.status.as_str(),
                    "missing_config" | "missing_token" | "error"
                )
            })
            .count(),
    };

    Ok(GatewayChannelsResponse {
        summary,
        channels,
        accounts,
    })
}

pub async fn upsert_channel_account(
    storage: &Storage,
    input: GatewayChannelAccountUpsert,
) -> Result<GatewayChannelAccount> {
    let mut accounts: Vec<GatewayChannelAccount> = load_json(storage, CHANNEL_ACCOUNTS_KEY).await?;
    let now = now_rfc3339();
    let id = input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let account = if let Some(existing) = accounts.iter_mut().find(|account| account.id == id) {
        existing.channel_id = input.channel_id;
        if let Some(label) = input.label {
            existing.label = label;
        }
        if let Some(enabled) = input.enabled {
            existing.enabled = enabled;
        }
        if let Some(status) = input.status {
            existing.status = status;
        }
        if input.peer_scope.is_some() {
            existing.peer_scope = input.peer_scope;
        }
        if input.default_agent_id.is_some() {
            existing.default_agent_id = input.default_agent_id;
        }
        if input.last_seen_at.is_some() {
            existing.last_seen_at = input.last_seen_at;
        } else if existing.last_seen_at.is_none() {
            existing.last_seen_at = Some(now.clone());
        }
        if input.last_error.is_some() {
            existing.last_error = input.last_error;
        }
        if input.note.is_some() {
            existing.note = input.note;
        }
        if input.metadata.is_some() {
            existing.metadata = input.metadata;
        }
        existing.clone()
    } else {
        let account = GatewayChannelAccount {
            id: id.clone(),
            channel_id: input.channel_id,
            label: input
                .label
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "Primary account".to_string()),
            enabled: input.enabled.unwrap_or(true),
            status: input.status.unwrap_or_else(|| "connected".to_string()),
            peer_scope: input.peer_scope,
            default_agent_id: input.default_agent_id,
            last_seen_at: input.last_seen_at.or(Some(now)),
            last_error: input.last_error,
            note: input.note,
            metadata: input.metadata,
        };
        accounts.push(account.clone());
        account
    };

    save_json(storage, CHANNEL_ACCOUNTS_KEY, &accounts).await?;
    Ok(account)
}

pub async fn delete_channel_account(storage: &Storage, id: &str) -> Result<bool> {
    let mut accounts: Vec<GatewayChannelAccount> = load_json(storage, CHANNEL_ACCOUNTS_KEY).await?;
    let before = accounts.len();
    accounts.retain(|account| account.id != id);
    if before == accounts.len() {
        return Ok(false);
    }
    save_json(storage, CHANNEL_ACCOUNTS_KEY, &accounts).await?;
    Ok(true)
}

pub async fn load_routing(storage: &Storage) -> Result<GatewayRoutingResponse> {
    let rules: Vec<GatewayRouteRule> = load_json(storage, ROUTE_RULES_KEY).await?;
    let groups: Vec<GatewayBroadcastGroup> = load_json(storage, BROADCAST_GROUPS_KEY).await?;
    let summary = GatewayRoutingSummary {
        rules: rules.len(),
        enabled_rules: rules.iter().filter(|rule| rule.enabled).count(),
        broadcast_groups: groups.len(),
    };
    Ok(GatewayRoutingResponse {
        summary,
        rules,
        broadcast_groups: groups,
    })
}

pub async fn upsert_route_rule(
    storage: &Storage,
    input: GatewayRouteRuleUpsert,
) -> Result<GatewayRouteRule> {
    let mut rules: Vec<GatewayRouteRule> = load_json(storage, ROUTE_RULES_KEY).await?;
    let now = now_rfc3339();
    let id = input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let rule = if let Some(existing) = rules.iter_mut().find(|rule| rule.id == id) {
        if let Some(name) = input.name {
            existing.name = name;
        }
        if let Some(enabled) = input.enabled {
            existing.enabled = enabled;
        }
        if let Some(priority) = input.priority {
            existing.priority = priority;
        }
        if input.channel_id.is_some() {
            existing.channel_id = input.channel_id;
        }
        if input.account_id.is_some() {
            existing.account_id = input.account_id;
        }
        if let Some(match_kind) = input.match_kind {
            existing.match_kind = match_kind;
        }
        if let Some(match_value) = input.match_value {
            existing.match_value = match_value;
        }
        if let Some(target_kind) = input.target_kind {
            existing.target_kind = target_kind;
        }
        if let Some(target_value) = input.target_value {
            existing.target_value = target_value;
        }
        if input.agent_id.is_some() {
            existing.agent_id = input.agent_id;
        }
        if input.conversation_scope.is_some() {
            existing.conversation_scope = input.conversation_scope;
        }
        if input.broadcast_group_id.is_some() {
            existing.broadcast_group_id = input.broadcast_group_id;
        }
        if input.notes.is_some() {
            existing.notes = input.notes;
        }
        existing.updated_at = Some(now.clone());
        existing.clone()
    } else {
        let rule = GatewayRouteRule {
            id: id.clone(),
            name: input
                .name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "New route".to_string()),
            enabled: input.enabled.unwrap_or(true),
            priority: input.priority.unwrap_or(100),
            channel_id: input.channel_id,
            account_id: input.account_id,
            match_kind: input.match_kind.unwrap_or_else(|| "channel".to_string()),
            match_value: input.match_value.unwrap_or_else(|| "web".to_string()),
            target_kind: input.target_kind.unwrap_or_else(|| "agent".to_string()),
            target_value: input
                .target_value
                .unwrap_or_else(|| "agent:primary-agent".to_string()),
            agent_id: input.agent_id,
            conversation_scope: input.conversation_scope.or(Some("per_channel".to_string())),
            broadcast_group_id: input.broadcast_group_id,
            notes: input.notes,
            created_at: Some(now.clone()),
            updated_at: Some(now),
            last_matched_at: None,
        };
        rules.push(rule.clone());
        rule
    };

    save_json(storage, ROUTE_RULES_KEY, &rules).await?;
    Ok(rule)
}

pub async fn delete_route_rule(storage: &Storage, id: &str) -> Result<bool> {
    let mut rules: Vec<GatewayRouteRule> = load_json(storage, ROUTE_RULES_KEY).await?;
    let before = rules.len();
    rules.retain(|rule| rule.id != id);
    if before == rules.len() {
        return Ok(false);
    }
    save_json(storage, ROUTE_RULES_KEY, &rules).await?;
    Ok(true)
}

pub async fn create_broadcast_group(
    storage: &Storage,
    input: GatewayBroadcastGroupCreate,
) -> Result<GatewayBroadcastGroup> {
    let mut groups: Vec<GatewayBroadcastGroup> = load_json(storage, BROADCAST_GROUPS_KEY).await?;
    let group = GatewayBroadcastGroup {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name.trim().to_string(),
        description: input.description.filter(|value| !value.trim().is_empty()),
        enabled: true,
        member_count: input.channels.len() + input.targets.len(),
        channels: input.channels,
        targets: input.targets,
    };
    groups.push(group.clone());
    save_json(storage, BROADCAST_GROUPS_KEY, &groups).await?;
    Ok(group)
}

pub fn simulate_routing(
    rules: &[GatewayRouteRule],
    request: &GatewayRoutingSimulationRequest,
) -> GatewayRoutingSimulation {
    let mut candidates = rules.iter().filter(|rule| rule.enabled).collect::<Vec<_>>();
    candidates.sort_by_key(|rule| rule.priority);

    for rule in candidates {
        let matches = match rule.match_kind.trim().to_ascii_lowercase().as_str() {
            "channel" => request.channel_id.as_deref() == Some(rule.match_value.as_str()),
            "account" => request.account_id.as_deref() == Some(rule.match_value.as_str()),
            "scope" => request
                .match_kind
                .as_deref()
                .map(|kind| kind.eq_ignore_ascii_case(&rule.match_value))
                .unwrap_or(false),
            "contains" => request
                .match_value
                .as_deref()
                .map(|value| value.contains(&rule.match_value))
                .unwrap_or(false),
            _ => {
                request.match_kind.as_deref() == Some(rule.match_kind.as_str())
                    && request.match_value.as_deref() == Some(rule.match_value.as_str())
            }
        };
        if matches {
            return GatewayRoutingSimulation {
                matched: true,
                rule_id: Some(rule.id.clone()),
                rule_name: Some(rule.name.clone()),
                target_kind: Some(rule.target_kind.clone()),
                target_value: Some(rule.target_value.clone()),
                conversation_scope: rule.conversation_scope.clone(),
                broadcast_group_id: rule.broadcast_group_id.clone(),
                reason: Some(format!("Matched {}:{}", rule.match_kind, rule.match_value)),
            };
        }
    }

    GatewayRoutingSimulation {
        matched: false,
        reason: Some("No enabled rule matched the simulation input.".to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(
        id: &str,
        enabled: bool,
        priority: i32,
        match_kind: &str,
        match_value: &str,
        target_value: &str,
    ) -> GatewayRouteRule {
        GatewayRouteRule {
            id: id.to_string(),
            name: format!("rule-{}", id),
            enabled,
            priority,
            channel_id: None,
            account_id: None,
            match_kind: match_kind.to_string(),
            match_value: match_value.to_string(),
            target_kind: "agent".to_string(),
            target_value: target_value.to_string(),
            agent_id: None,
            conversation_scope: Some("per_channel".to_string()),
            broadcast_group_id: None,
            notes: None,
            created_at: None,
            updated_at: None,
            last_matched_at: None,
        }
    }

    #[test]
    fn simulate_routing_prefers_lowest_priority_match() {
        let rules = vec![
            make_rule("slow", true, 50, "channel", "slack", "agent-slow"),
            make_rule("fast", true, 10, "channel", "slack", "agent-fast"),
        ];
        let simulation = simulate_routing(
            &rules,
            &GatewayRoutingSimulationRequest {
                channel_id: Some("slack".to_string()),
                ..Default::default()
            },
        );

        assert!(simulation.matched);
        assert_eq!(simulation.rule_id.as_deref(), Some("fast"));
        assert_eq!(simulation.target_value.as_deref(), Some("agent-fast"));
    }

    #[test]
    fn simulate_routing_ignores_disabled_rules() {
        let rules = vec![
            make_rule("disabled", false, 1, "channel", "discord", "agent-disabled"),
            make_rule("enabled", true, 2, "channel", "discord", "agent-enabled"),
        ];
        let simulation = simulate_routing(
            &rules,
            &GatewayRoutingSimulationRequest {
                channel_id: Some("discord".to_string()),
                ..Default::default()
            },
        );

        assert!(simulation.matched);
        assert_eq!(simulation.rule_id.as_deref(), Some("enabled"));
        assert_eq!(simulation.target_value.as_deref(), Some("agent-enabled"));
    }

    #[test]
    fn simulate_routing_matches_contains_rules() {
        let rules = vec![make_rule(
            "contains",
            true,
            1,
            "contains",
            "billing",
            "billing-agent",
        )];
        let simulation = simulate_routing(
            &rules,
            &GatewayRoutingSimulationRequest {
                match_value: Some("urgent billing escalation".to_string()),
                ..Default::default()
            },
        );

        assert!(simulation.matched);
        assert_eq!(simulation.rule_id.as_deref(), Some("contains"));
        assert_eq!(simulation.target_value.as_deref(), Some("billing-agent"));
    }
}
