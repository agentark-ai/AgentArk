//! Messaging channel registry — single source of truth for "what channels can
//! receive a notification right now."
//!
//! The old `EXTERNAL_NOTIFICATION_CHANNELS` compile-time array has been
//! replaced by this registry. Two first-class sources ship today:
//!
//! - [`BundledChannelSource`] — the 13 Rust-implemented channels (Slack,
//!   Telegram, …) keep their existing dispatch code paths. The registry just
//!   advertises them; it doesn't change how they send.
//! - [`ExtensionPackChannelSource`] — any extension pack whose manifest
//!   declares a [`crate::extension_packs::MessagingChannelSpec`] becomes a
//!   channel. Pack channels are namespaced as `ext.<pack_id>` so a pack
//!   cannot shadow a bundled id.
//!
//! The registry is extensible: adding a third source (a remote registry, a
//! user-configured generic webhook registry, etc.) is a one-line addition
//! in [`MessagingChannelRegistry::default`]. Callers never see which source a
//! descriptor came from unless they explicitly branch on [`ChannelSource`].
//!
//! Configured-check uses the pack's own auth manifest / declared secret slots
//! against the secret store. Unconfigured pack channels are filtered out of
//! the LLM-visible action-schema surface so the assistant cannot hallucinate
//! a send to a half-configured channel.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;

use crate::core::connectivity::integration_auth::{
    manifest_all_storage_targets, manifest_from_extension_pack, IntegrationAuthManifest,
};
use crate::core::runtime::config::SecureConfigManager;
use crate::extension_packs::{
    AuthTransportBinding, ExtensionPackAuthMode, ExtensionPackManifest, ExtensionPackRegistry,
    MessagingSendSpec,
};
use crate::storage::Storage;

/// Namespace prefix for extension-pack-declared channel ids. Keeps pack
/// channels out of the bundled id space.
pub const EXTENSION_CHANNEL_ID_PREFIX: &str = "ext.";

/// Canonical set of bundled channel ids. Bundled channels keep their
/// existing Rust dispatch; this list only advertises them to the registry.
pub const BUNDLED_CHANNEL_IDS: &[&str] = &[
    "telegram",
    "whatsapp",
    "slack",
    "discord",
    "matrix",
    "teams",
    "google_chat",
    "signal",
    "imessage",
    "line",
    "wechat",
    "qq",
    "email",
];

/// What kind of source produced this descriptor.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelSource {
    /// One of the 13 compile-time bundled channels. Dispatched via the
    /// matching `src/channels/<name>.rs` module.
    Bundled,
    /// Declared by an installed extension pack. Dispatched by the HTTP
    /// template sender in [`super::messaging_dispatch`].
    ExtensionPack { pack_id: String },
    /// User-added custom messaging channel stored in AgentArk encrypted
    /// config. Dispatched by the same HTTP template sender as pack channels.
    CustomMessagingChannel { channel_id: String },
}

/// Stable, user-facing description of a channel. Shape is stable across
/// sources so UI / agent tooling can render one list.
///
/// `auth_manifest`, `send_spec`, and `auth_transport` are currently set on
/// every pack descriptor but only read once `messaging_dispatch` gets a
/// caller inside the notify_user path — they're the handoff between the
/// registry and the dispatcher. Flagged `#[allow(dead_code)]` until that
/// wiring lands.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct ChannelDescriptor {
    pub id: String,
    pub display_name: String,
    pub source: ChannelSource,
    /// True when every secret slot the channel's auth manifest requires is
    /// present in the secret store. Bundled channels compute this via the
    /// existing `notification_channel_is_configured` hook on the agent.
    pub configured: bool,
    /// True when the channel advertises an auth contract (pack-declared
    /// channels always do; bundled channels do, implicitly, via their
    /// per-channel settings — we just don't model those here).
    pub requires_auth: bool,
    /// Optional "where do I get credentials?" help URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// Auth manifest for pack channels, enabling the inline credential
    /// prompt when the channel is not yet configured. `None` for bundled
    /// (their auth lives in the existing settings UI).
    #[serde(skip)]
    pub auth_manifest: Option<IntegrationAuthManifest>,
    /// Dispatch spec for pack channels. `None` for bundled.
    #[serde(skip)]
    pub send_spec: Option<MessagingSendSpec>,
    /// Auth transport binding cached from the send spec for fast dispatch.
    /// Duplicated here so `dispatch` doesn't need to re-derive.
    #[serde(skip)]
    pub auth_transport: Option<AuthTransportBinding>,
    /// Optional reusable auth profile used by custom messaging channels.
    #[serde(skip)]
    pub auth_profile_id: Option<String>,
}

/// Contract every registry source implements. `list` returns all channels
/// it knows about regardless of `configured` state — filtering is done by
/// the composite [`MessagingChannelRegistry`].
#[async_trait]
pub trait MessagingChannelRegistrySource: Send + Sync {
    async fn list(&self, ctx: &ChannelQueryContext<'_>) -> Result<Vec<ChannelDescriptor>>;
    /// Default lookup via a full `list + find`. Sources that can answer
    /// lookups more cheaply (e.g. a future remote-cache source) should
    /// override this. Flagged `#[allow(dead_code)]` until a caller beyond
    /// the HTTP `GET /channels/available` endpoint materialises — the
    /// registry's composite `lookup` covers today's callers.
    #[allow(dead_code)]
    async fn lookup(
        &self,
        ctx: &ChannelQueryContext<'_>,
        id: &str,
    ) -> Result<Option<ChannelDescriptor>> {
        let all = self.list(ctx).await?;
        Ok(all.into_iter().find(|descriptor| descriptor.id == id))
    }
}

/// Context passed to each source so it can resolve configured-state without
/// pulling in the full `Agent`. Keeps sources testable with a mocked
/// `SecureConfigManager`.
pub struct ChannelQueryContext<'a> {
    pub bundled_configured: &'a dyn BundledConfiguredCheck,
    pub extension_packs: &'a ExtensionPackRegistry,
    pub storage: &'a Storage,
    pub config_dir: &'a Path,
    pub data_dir: &'a Path,
    pub config_manager: Option<&'a SecureConfigManager>,
}

/// Callback that answers "is this bundled channel id configured right now?"
/// The agent holds the real implementation (email vs push vs provider-specific
/// settings). Tests can substitute a closure.
pub trait BundledConfiguredCheck: Send + Sync {
    fn is_configured(&self, channel_id: &str) -> bool;
}

impl<F> BundledConfiguredCheck for F
where
    F: Fn(&str) -> bool + Send + Sync,
{
    fn is_configured(&self, channel_id: &str) -> bool {
        (self)(channel_id)
    }
}

/// Source for the 13 bundled channels.
pub struct BundledChannelSource;

#[async_trait]
impl MessagingChannelRegistrySource for BundledChannelSource {
    async fn list(&self, ctx: &ChannelQueryContext<'_>) -> Result<Vec<ChannelDescriptor>> {
        Ok(BUNDLED_CHANNEL_IDS
            .iter()
            .map(|id| {
                let configured = ctx.bundled_configured.is_configured(id);
                ChannelDescriptor {
                    id: (*id).to_string(),
                    display_name: bundled_display_name(id).to_string(),
                    source: ChannelSource::Bundled,
                    configured,
                    requires_auth: true,
                    docs_url: None,
                    auth_manifest: None,
                    send_spec: None,
                    auth_transport: None,
                    auth_profile_id: None,
                }
            })
            .collect())
    }
}

/// Source that walks installed extension packs and surfaces any whose
/// manifest declares a `channel` block. Pack ids become `ext.<pack_id>` so
/// they live in a disjoint namespace from bundled channels.
pub struct ExtensionPackChannelSource;

#[async_trait]
impl MessagingChannelRegistrySource for ExtensionPackChannelSource {
    async fn list(&self, ctx: &ChannelQueryContext<'_>) -> Result<Vec<ChannelDescriptor>> {
        let packs = ctx.extension_packs.list_installed(None).await?;
        let mut out = Vec::new();
        for view in packs {
            if !view.enabled {
                continue;
            }
            if let Some(descriptor) =
                descriptor_for_pack_manifest(&view.manifest, ctx.config_manager)
            {
                out.push(descriptor);
            }
        }
        Ok(out)
    }
}

/// Source for user-added custom messaging channels. These are stored in the
/// same encrypted configuration plane as custom APIs, not in a SQL table.
pub struct CustomMessagingChannelSource;

#[async_trait]
impl MessagingChannelRegistrySource for CustomMessagingChannelSource {
    async fn list(&self, ctx: &ChannelQueryContext<'_>) -> Result<Vec<ChannelDescriptor>> {
        let views = crate::custom_messaging_channels::list_custom_messaging_channels(
            ctx.storage,
            ctx.config_dir,
            ctx.data_dir,
        )
        .await?;
        Ok(views
            .into_iter()
            .map(|view| ChannelDescriptor {
                id: view.runtime_channel_id,
                display_name: view.config.name.clone(),
                source: ChannelSource::CustomMessagingChannel {
                    channel_id: view.config.id.clone(),
                },
                configured: view.configured,
                requires_auth: view.requires_auth,
                docs_url: view.config.docs_url.clone(),
                auth_manifest: view.config.auth_manifest.clone(),
                send_spec: Some(view.config.send.clone()),
                auth_transport: Some(view.config.send.auth.clone()),
                auth_profile_id: view.config.auth_profile_id.clone(),
            })
            .collect())
    }
}

/// Build a channel descriptor from a pack manifest, if the manifest declares
/// one. Returns `None` when the pack is not a messaging channel.
pub fn descriptor_for_pack_manifest(
    manifest: &ExtensionPackManifest,
    config_manager: Option<&SecureConfigManager>,
) -> Option<ChannelDescriptor> {
    let spec = manifest.channel.as_ref()?;
    let pack_id = manifest.id.trim();
    if pack_id.is_empty() {
        return None;
    }
    let id = format!("{}{}", EXTENSION_CHANNEL_ID_PREFIX, pack_id);
    let display_name = spec
        .display_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if manifest.name.trim().is_empty() {
                pack_id.to_string()
            } else {
                manifest.name.trim().to_string()
            }
        });
    let auth_manifest = manifest_from_extension_pack(manifest);
    let requires_auth = auth_manifest.is_some();
    let configured = channel_is_configured(manifest, auth_manifest.as_ref(), config_manager);
    let send_spec = auth_manifest
        .as_ref()
        .map(extension_pack_secret_aliases_for_manifest)
        .map(|aliases| {
            crate::channels::messaging_dispatch::rewrite_send_spec_secret_refs(&spec.send, &aliases)
        })
        .unwrap_or_else(|| spec.send.clone());

    Some(ChannelDescriptor {
        id,
        display_name,
        source: ChannelSource::ExtensionPack {
            pack_id: pack_id.to_string(),
        },
        configured,
        requires_auth,
        docs_url: spec.docs_url.clone(),
        auth_manifest,
        send_spec: Some(send_spec.clone()),
        auth_transport: Some(send_spec.auth.clone()),
        auth_profile_id: None,
    })
}

fn channel_is_configured(
    manifest: &ExtensionPackManifest,
    auth_manifest: Option<&IntegrationAuthManifest>,
    config_manager: Option<&SecureConfigManager>,
) -> bool {
    // A pack channel is "configured" when every required secret slot the
    // pack declares is present in the secret store. Packs that declare no
    // auth at all (`ExtensionPackAuthMode::None`) are always configured;
    // they're rare — typically public-webhook demos.
    if matches!(manifest.auth.mode, ExtensionPackAuthMode::None) {
        return true;
    }
    crate::custom_messaging_channels::manifest_is_configured(auth_manifest, config_manager)
}

fn extension_pack_secret_aliases_for_manifest(
    manifest: &IntegrationAuthManifest,
) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    let prefix = format!("ext.{}.", manifest.integration_id.trim());
    for target in manifest_all_storage_targets(manifest) {
        let target = target.trim().to_string();
        if target.is_empty() {
            continue;
        }
        aliases.insert(target.clone(), target.clone());
        if let Some(suffix) = target.strip_prefix(&prefix) {
            let suffix = suffix.trim();
            if !suffix.is_empty() {
                aliases.insert(suffix.to_string(), target.clone());
            }
        }
    }
    match &manifest.mode {
        crate::core::connectivity::integration_auth::AuthMode::Secrets { fields }
        | crate::core::connectivity::integration_auth::AuthMode::Hybrid { fields, .. } => {
            for field in fields {
                if let Some(target) = field.storage_targets.first() {
                    aliases.insert(field.key.clone(), target.clone());
                }
            }
        }
        crate::core::connectivity::integration_auth::AuthMode::OAuth2AuthorizationCode(_)
        | crate::core::connectivity::integration_auth::AuthMode::OAuth2DeviceCode(_) => {}
    }
    aliases
}

/// Human-readable display name for a bundled channel id. Keeps parity with
/// the old hardcoded labels in the agent.
pub fn bundled_display_name(id: &str) -> &'static str {
    match id.trim().to_ascii_lowercase().as_str() {
        "telegram" => "Telegram",
        "whatsapp" => "WhatsApp",
        "slack" => "Slack",
        "discord" => "Discord",
        "matrix" => "Matrix",
        "teams" => "Microsoft Teams",
        "google_chat" => "Google Chat",
        "signal" => "Signal",
        "imessage" => "iMessage",
        "line" => "LINE",
        "wechat" => "WeChat",
        "qq" => "QQ",
        "email" => "Email",
        _ => "Channel",
    }
}

/// Composite registry: walks each configured source and unions the results.
/// Duplicate ids across sources are resolved by first-source-wins; in the
/// default configuration this means bundled ids cannot be shadowed.
pub struct MessagingChannelRegistry {
    sources: Vec<Arc<dyn MessagingChannelRegistrySource>>,
}

impl Default for MessagingChannelRegistry {
    fn default() -> Self {
        Self {
            sources: vec![
                Arc::new(BundledChannelSource),
                Arc::new(CustomMessagingChannelSource),
                Arc::new(ExtensionPackChannelSource),
            ],
        }
    }
}

impl MessagingChannelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a registry with a custom source list. Useful for tests that
    /// want to inject a deterministic source, or for a future setup that
    /// loads sources from config. Flagged `#[allow(dead_code)]` until that
    /// caller lands.
    #[allow(dead_code)]
    pub fn with_sources(sources: Vec<Arc<dyn MessagingChannelRegistrySource>>) -> Self {
        Self { sources }
    }

    /// Full list from all sources. Bundled entries come first so first-wins
    /// collision policy leaves bundled ids intact.
    pub async fn list(&self, ctx: &ChannelQueryContext<'_>) -> Result<Vec<ChannelDescriptor>> {
        let mut out: Vec<ChannelDescriptor> = Vec::new();
        let mut seen = std::collections::HashSet::<String>::new();
        for source in &self.sources {
            let list = source.list(ctx).await?;
            for descriptor in list {
                if seen.contains(&descriptor.id) {
                    tracing::warn!(
                        "messaging_registry: dropped duplicate channel id `{}` from a later source",
                        descriptor.id
                    );
                    continue;
                }
                seen.insert(descriptor.id.clone());
                out.push(descriptor);
            }
        }
        Ok(out)
    }

    /// Lookup by id. Returns `None` if no source advertises the id. Used by
    /// dispatch once a caller inside the notify_user path needs to resolve
    /// a pack-channel id to its send spec; flagged `#[allow(dead_code)]`
    /// until that wiring lands.
    #[allow(dead_code)]
    pub async fn lookup(
        &self,
        ctx: &ChannelQueryContext<'_>,
        id: &str,
    ) -> Result<Option<ChannelDescriptor>> {
        let all = self.list(ctx).await?;
        Ok(all.into_iter().find(|descriptor| descriptor.id == id))
    }

    /// Subset of [`Self::list`] where `configured == true`. Designed to feed
    /// the LLM's action-schema enum so unconfigured pack channels are
    /// invisible to the agent. Today that filtering happens through
    /// `Agent::available_notification_channel_ids` instead of this method;
    /// once more callers need the full descriptor (not just the id) this
    /// becomes the canonical filter.
    #[allow(dead_code)]
    pub async fn list_configured(
        &self,
        ctx: &ChannelQueryContext<'_>,
    ) -> Result<Vec<ChannelDescriptor>> {
        let all = self.list(ctx).await?;
        Ok(all
            .into_iter()
            .filter(|descriptor| descriptor.configured)
            .collect())
    }

    /// Ids of every channel (configured or not). Convenience over
    /// [`Self::list`]; kept for parity with the shape of the old
    /// `EXTERNAL_NOTIFICATION_CHANNELS` const.
    #[allow(dead_code)]
    pub async fn all_ids(&self, ctx: &ChannelQueryContext<'_>) -> Result<Vec<String>> {
        Ok(self
            .list(ctx)
            .await?
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension_packs::{
        ExtensionPackAuthSpec, ExtensionPackManifest, ExtensionPackOAuth2Spec, HttpSendMethod,
        MessagingChannelSpec, MessagingSendSpec,
    };

    fn pack_with_channel(id: &str, required_secrets: &[&str]) -> ExtensionPackManifest {
        ExtensionPackManifest {
            id: id.to_string(),
            name: format!("{} pack", id),
            version: "1.0.0".to_string(),
            kind: "pack".to_string(),
            auth: ExtensionPackAuthSpec {
                mode: ExtensionPackAuthMode::ApiKey,
                required_secrets: required_secrets.iter().map(|s| (*s).to_string()).collect(),
                ..ExtensionPackAuthSpec::default()
            },
            channel: Some(MessagingChannelSpec {
                display_name: Some(format!("{} channel", id)),
                send: MessagingSendSpec {
                    method: HttpSendMethod::Post,
                    url_template: "https://example/{{to}}".to_string(),
                    ..MessagingSendSpec::default()
                },
                ..MessagingChannelSpec::default()
            }),
            ..ExtensionPackManifest::default()
        }
    }

    #[test]
    fn descriptor_id_is_namespaced_to_avoid_bundled_collision() {
        let manifest = pack_with_channel("slack", &["token"]);
        let descriptor =
            descriptor_for_pack_manifest(&manifest, None).expect("pack declares a channel");
        assert!(
            descriptor.id.starts_with(EXTENSION_CHANNEL_ID_PREFIX),
            "pack channel id must be namespaced; got {}",
            descriptor.id
        );
        assert_ne!(descriptor.id, "slack", "pack must not shadow bundled slack");
    }

    #[test]
    fn pack_without_channel_field_returns_none() {
        let manifest = ExtensionPackManifest {
            id: "x".to_string(),
            name: "x".to_string(),
            version: "1".to_string(),
            kind: "pack".to_string(),
            ..ExtensionPackManifest::default()
        };
        assert!(descriptor_for_pack_manifest(&manifest, None).is_none());
    }

    #[test]
    fn pack_with_auth_none_is_configured_without_secrets() {
        let mut manifest = pack_with_channel("ping", &[]);
        manifest.auth.mode = ExtensionPackAuthMode::None;
        manifest.auth.required_secrets.clear();
        let descriptor =
            descriptor_for_pack_manifest(&manifest, None).expect("pack declares a channel");
        assert!(descriptor.configured);
    }

    #[test]
    fn pack_with_required_secrets_is_unconfigured_without_manager() {
        let manifest = pack_with_channel("secret_channel", &["token"]);
        let descriptor =
            descriptor_for_pack_manifest(&manifest, None).expect("pack declares a channel");
        assert!(!descriptor.configured);
    }

    #[test]
    fn oauth2_pack_channel_carries_auth_transport_binding() {
        let mut manifest = pack_with_channel("oauth_channel", &[]);
        manifest.auth.mode = ExtensionPackAuthMode::OAuth2External;
        manifest.auth.oauth2 = Some(ExtensionPackOAuth2Spec {
            client_id: "cid".to_string(),
            client_secret: "csec".to_string(),
            auth_url: "https://oauth/authorize".to_string(),
            token_url: "https://oauth/token".to_string(),
            ..ExtensionPackOAuth2Spec::default()
        });
        let descriptor =
            descriptor_for_pack_manifest(&manifest, None).expect("pack declares a channel");
        assert!(descriptor.auth_manifest.is_some());
    }
}
