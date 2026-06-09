use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

use super::*;

pub const CAPABILITY_READINESS_KV_KEY: &str = "capability_readiness_registry_v1";
const CAPABILITY_READINESS_CONTEXT_MAX_ENTRIES: usize = 24;
const CAPABILITY_READINESS_CONTEXT_FIELD_MAX_CHARS: usize = 120;
const INTEGRATION_READINESS_STALE_AFTER_SECONDS: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySurface {
    Integration,
    MessagingChannel,
    McpServer,
    ModelProvider,
    CustomApi,
    ExtensionPack,
    Plugin,
    Other(String),
}

impl CapabilitySurface {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Integration => "integration",
            Self::MessagingChannel => "messaging_channel",
            Self::McpServer => "mcp_server",
            Self::ModelProvider => "model_provider",
            Self::CustomApi => "custom_api",
            Self::ExtensionPack => "extension_pack",
            Self::Plugin => "plugin",
            Self::Other(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CapabilityReadinessKey {
    pub surface: CapabilitySurface,
    pub id: String,
}

impl CapabilityReadinessKey {
    pub fn new(surface: CapabilitySurface, id: impl Into<String>) -> Self {
        Self {
            surface,
            id: id.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReadinessStatus {
    NotConfigured,
    NeedsAuth,
    Connected,
    Error,
}

impl CapabilityReadinessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::NeedsAuth => "needs_auth",
            Self::Connected => "connected",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReadinessSource {
    BootProbe,
    StatusProbe,
    RuntimeEvent,
    OAuthCallback,
    UserToggle,
    SyncDetector,
    UseTimeFailure,
    ProviderHealth,
    WarmStart,
}

impl CapabilityReadinessSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BootProbe => "boot_probe",
            Self::StatusProbe => "status_probe",
            Self::RuntimeEvent => "runtime_event",
            Self::OAuthCallback => "oauth_callback",
            Self::UserToggle => "user_toggle",
            Self::SyncDetector => "sync_detector",
            Self::UseTimeFailure => "use_time_failure",
            Self::ProviderHealth => "provider_health",
            Self::WarmStart => "warm_start",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReadinessEntry {
    pub key: CapabilityReadinessKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: CapabilityReadinessStatus,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub source: CapabilityReadinessSource,
}

impl CapabilityReadinessEntry {
    pub fn stale_at(&self, now: DateTime<Utc>) -> bool {
        if self.expires_at.is_some_and(|expires_at| expires_at <= now) {
            return true;
        }
        let Some(last_verified) = self.last_verified else {
            return true;
        };
        let Some(stale_after_seconds) = self.stale_after_seconds else {
            return false;
        };
        last_verified + chrono::Duration::seconds(stale_after_seconds) <= now
    }

    fn should_appear_in_compact_snapshot(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return true;
        }
        if self.stale_at(now) {
            return true;
        }
        !matches!(self.status, CapabilityReadinessStatus::NotConfigured)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReadinessSnapshotEntry {
    pub surface: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: String,
    pub enabled: bool,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReadinessSnapshot {
    pub generation: u64,
    pub generated_at: String,
    pub entries: Vec<CapabilityReadinessSnapshotEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReadinessRegistry {
    generation: u64,
    entries: BTreeMap<CapabilityReadinessKey, CapabilityReadinessEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedCapabilityReadinessRegistry {
    generation: u64,
    #[serde(default)]
    entries: Vec<CapabilityReadinessEntry>,
}

impl Serialize for CapabilityReadinessRegistry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PersistedCapabilityReadinessRegistry {
            generation: self.generation,
            entries: self.entries.values().cloned().collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CapabilityReadinessRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let persisted = PersistedCapabilityReadinessRegistry::deserialize(deserializer)?;
        Ok(Self {
            generation: persisted.generation,
            entries: persisted
                .entries
                .into_iter()
                .map(|entry| (entry.key.clone(), entry))
                .collect(),
        })
    }
}

impl Default for CapabilityReadinessRegistry {
    fn default() -> Self {
        Self {
            generation: 0,
            entries: BTreeMap::new(),
        }
    }
}

impl CapabilityReadinessRegistry {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub fn get(&self, key: &CapabilityReadinessKey) -> Option<&CapabilityReadinessEntry> {
        self.entries.get(key)
    }

    #[cfg(test)]
    pub fn upsert(&mut self, entry: CapabilityReadinessEntry) -> bool {
        let changed = self.entries.get(&entry.key) != Some(&entry);
        if changed {
            self.entries.insert(entry.key.clone(), entry);
            self.generation = self.generation.saturating_add(1);
        }
        changed
    }

    pub fn replace_surface_entries(
        &mut self,
        surface: CapabilitySurface,
        entries: Vec<CapabilityReadinessEntry>,
    ) -> bool {
        let now = Utc::now();
        let next_keys = entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect::<BTreeSet<_>>();
        let stale_keys = self
            .entries
            .keys()
            .filter(|key| key.surface == surface && !next_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();

        let mut visible_changed = false;
        for key in stale_keys {
            visible_changed |= self.entries.remove(&key).is_some();
        }
        for entry in entries {
            match self.entries.get_mut(&entry.key) {
                Some(existing) => {
                    visible_changed |= readiness_entry_meaningfully_changed(existing, &entry, now);
                    if existing != &entry {
                        *existing = entry;
                    }
                }
                None => {
                    self.entries.insert(entry.key.clone(), entry);
                    visible_changed = true;
                }
            }
        }
        if visible_changed {
            self.generation = self.generation.saturating_add(1);
        }
        visible_changed
    }

    pub fn mark_action_scope_failure(
        &mut self,
        scope: &crate::runtime::ActionScopeHint,
        error: &str,
    ) -> bool {
        let keys = capability_readiness_keys_for_action_scope(scope);
        if keys.is_empty() {
            return false;
        }
        let now = Utc::now();
        let error = sanitize_context_field(error, 240);
        let mut visible_changed = false;
        for key in keys {
            let next = match self.entries.get(&key).cloned() {
                Some(mut entry) => {
                    entry.status = CapabilityReadinessStatus::Error;
                    entry.last_error = Some(error.clone());
                    entry.source = CapabilityReadinessSource::UseTimeFailure;
                    entry
                }
                None => CapabilityReadinessEntry {
                    key: key.clone(),
                    label: None,
                    status: CapabilityReadinessStatus::Error,
                    enabled: true,
                    last_verified: None,
                    expires_at: None,
                    stale_after_seconds: None,
                    last_error: Some(error.clone()),
                    source: CapabilityReadinessSource::UseTimeFailure,
                },
            };
            let changed = self
                .entries
                .get(&key)
                .map(|existing| readiness_entry_meaningfully_changed(existing, &next, now))
                .unwrap_or(true);
            visible_changed |= changed;
            self.entries.insert(key, next);
        }
        if visible_changed {
            self.generation = self.generation.saturating_add(1);
        }
        visible_changed
    }

    pub fn compact_snapshot(
        &self,
        now: DateTime<Utc>,
        max_entries: usize,
    ) -> CapabilityReadinessSnapshot {
        let entries = self
            .entries
            .values()
            .filter(|entry| entry.should_appear_in_compact_snapshot(now))
            .take(max_entries)
            .map(|entry| {
                let stale = entry.stale_at(now);
                CapabilityReadinessSnapshotEntry {
                    surface: entry.key.surface.as_str().to_string(),
                    id: sanitize_context_field(
                        &entry.key.id,
                        CAPABILITY_READINESS_CONTEXT_FIELD_MAX_CHARS,
                    ),
                    label: entry.label.as_ref().map(|value| {
                        sanitize_context_field(value, CAPABILITY_READINESS_CONTEXT_FIELD_MAX_CHARS)
                    }),
                    status: if stale && entry.status == CapabilityReadinessStatus::Connected {
                        "stale".to_string()
                    } else {
                        entry.status.as_str().to_string()
                    },
                    enabled: entry.enabled,
                    stale,
                    last_verified: entry.last_verified.map(|value| value.to_rfc3339()),
                    expires_at: entry.expires_at.map(|value| value.to_rfc3339()),
                    last_error: entry.last_error.as_ref().map(|value| {
                        value
                            .chars()
                            .filter(|ch| !ch.is_control())
                            .take(240)
                            .collect::<String>()
                    }),
                    source: entry.source.as_str().to_string(),
                }
            })
            .collect();
        CapabilityReadinessSnapshot {
            generation: self.generation,
            generated_at: now.to_rfc3339(),
            entries,
        }
    }

    pub fn system_context_message(&self, now: DateTime<Utc>, max_entries: usize) -> Option<String> {
        let snapshot = self.compact_snapshot(now, max_entries);
        if snapshot.entries.is_empty() {
            return None;
        }
        let json = serde_json::to_string(&snapshot).ok()?;
        Some(format!("Capability readiness context:\n{json}"))
    }
}

fn readiness_entry_meaningfully_changed(
    existing: &CapabilityReadinessEntry,
    next: &CapabilityReadinessEntry,
    now: DateTime<Utc>,
) -> bool {
    existing.key != next.key
        || existing.label != next.label
        || existing.status != next.status
        || existing.enabled != next.enabled
        || existing.expires_at != next.expires_at
        || existing.stale_after_seconds != next.stale_after_seconds
        || existing.last_error != next.last_error
        || existing.stale_at(now) != next.stale_at(now)
}

fn sanitize_context_field(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_chars)
        .collect()
}

fn capability_readiness_keys_for_action_scope(
    scope: &crate::runtime::ActionScopeHint,
) -> BTreeSet<CapabilityReadinessKey> {
    let mut keys = BTreeSet::new();
    for id in &scope.integration_ids {
        let id = id.trim();
        if !id.is_empty() {
            keys.insert(CapabilityReadinessKey::new(
                CapabilitySurface::Integration,
                id,
            ));
        }
    }
    if let Some(id) = scope
        .custom_api_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        keys.insert(CapabilityReadinessKey::new(
            CapabilitySurface::CustomApi,
            id,
        ));
    }
    if let Some(id) = scope
        .mcp_server_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        keys.insert(CapabilityReadinessKey::new(
            CapabilitySurface::McpServer,
            id,
        ));
    }
    if let Some(id) = scope
        .plugin_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        keys.insert(CapabilityReadinessKey::new(CapabilitySurface::Plugin, id));
    }
    for id in &scope.extension_pack_ids {
        let id = id.trim();
        if !id.is_empty() {
            keys.insert(CapabilityReadinessKey::new(
                CapabilitySurface::ExtensionPack,
                id,
            ));
        }
    }
    for target in &scope.channel_targets {
        let id = target.default_target.trim();
        if !id.is_empty() {
            keys.insert(CapabilityReadinessKey::new(
                CapabilitySurface::MessagingChannel,
                id,
            ));
        }
    }
    keys
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityReadinessEvent {
    pub generation: u64,
}

pub async fn load_capability_readiness_registry(storage: &Storage) -> CapabilityReadinessRegistry {
    match storage.get(CAPABILITY_READINESS_KV_KEY).await {
        Ok(Some(raw)) => match serde_json::from_slice::<CapabilityReadinessRegistry>(&raw) {
            Ok(registry) => registry,
            Err(error) => {
                tracing::warn!(
                    "Failed to parse persisted capability readiness registry; starting empty: {}",
                    error
                );
                CapabilityReadinessRegistry::default()
            }
        },
        Ok(None) => CapabilityReadinessRegistry::default(),
        Err(error) => {
            tracing::warn!(
                "Failed to load persisted capability readiness registry; starting empty: {}",
                error
            );
            CapabilityReadinessRegistry::default()
        }
    }
}

fn readiness_status_from_integration_status(
    status: &crate::integrations::IntegrationStatus,
) -> (CapabilityReadinessStatus, Option<String>) {
    match status {
        crate::integrations::IntegrationStatus::NotConfigured => {
            (CapabilityReadinessStatus::NotConfigured, None)
        }
        crate::integrations::IntegrationStatus::NeedsAuth => {
            (CapabilityReadinessStatus::NeedsAuth, None)
        }
        crate::integrations::IntegrationStatus::Connected => {
            (CapabilityReadinessStatus::Connected, None)
        }
        crate::integrations::IntegrationStatus::Error(error) => {
            (CapabilityReadinessStatus::Error, Some(error.clone()))
        }
    }
}

fn integration_readiness_entry(
    info: &crate::integrations::IntegrationInfo,
    enabled: bool,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    let (status, last_error) = readiness_status_from_integration_status(&info.status);
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(CapabilitySurface::Integration, info.id.clone()),
        label: Some(info.name.clone()),
        status,
        enabled,
        last_verified: Some(now),
        expires_at: None,
        stale_after_seconds: Some(INTEGRATION_READINESS_STALE_AFTER_SECONDS),
        last_error,
        source,
    }
}

fn parse_utc_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn custom_api_readiness_entry(
    view: &crate::custom_apis::CustomApiView,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    let auth_ready = view.secret_configured
        || matches!(
            view.config.auth_mode,
            crate::custom_apis::CustomApiAuthMode::None
        );
    let failed_probe = matches!(
        view.config.last_test_outcome.as_deref(),
        Some("failure" | "unavailable")
    );
    let status = if failed_probe {
        CapabilityReadinessStatus::Error
    } else if !auth_ready {
        CapabilityReadinessStatus::NeedsAuth
    } else {
        CapabilityReadinessStatus::Connected
    };
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(CapabilitySurface::CustomApi, view.config.id.clone()),
        label: Some(view.config.name.clone()),
        status,
        enabled: view.config.enabled,
        last_verified: parse_utc_rfc3339(view.config.last_tested_at.as_deref()).or(Some(now)),
        expires_at: None,
        stale_after_seconds: None,
        last_error: if failed_probe {
            view.config.last_test_message.clone()
        } else {
            None
        },
        source,
    }
}

fn mcp_server_readiness_entry(
    view: &crate::mcp::registry::McpServerView,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    let status = if view.last_error.is_some() {
        CapabilityReadinessStatus::Error
    } else if !view.auth.has_auth && !matches!(view.auth.auth_type.as_str(), "none") {
        CapabilityReadinessStatus::NeedsAuth
    } else {
        CapabilityReadinessStatus::Connected
    };
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(CapabilitySurface::McpServer, view.id.clone()),
        label: Some(view.name.clone()),
        status,
        enabled: view.enabled,
        last_verified: Some(now),
        expires_at: None,
        stale_after_seconds: None,
        last_error: view
            .last_error
            .clone()
            .or_else(|| view.warnings.first().cloned()),
        source,
    }
}

fn extension_pack_status(status: &str, needs_auth: bool) -> CapabilityReadinessStatus {
    match status {
        "connected" | "ready" => CapabilityReadinessStatus::Connected,
        "needs_auth" => CapabilityReadinessStatus::NeedsAuth,
        "disabled" if needs_auth => CapabilityReadinessStatus::NeedsAuth,
        "disabled" => CapabilityReadinessStatus::Connected,
        "available" | "draft" => CapabilityReadinessStatus::NotConfigured,
        _ => CapabilityReadinessStatus::Error,
    }
}

fn extension_pack_readiness_entry(
    view: &crate::extension_packs::ExtensionPackView,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(
            CapabilitySurface::ExtensionPack,
            view.manifest.id.clone(),
        ),
        label: Some(view.manifest.name.clone()),
        status: extension_pack_status(&view.status, view.needs_auth),
        enabled: view.enabled,
        last_verified: Some(now),
        expires_at: None,
        stale_after_seconds: None,
        last_error: view
            .status_detail
            .clone()
            .or_else(|| view.runtime_detail.clone())
            .or_else(|| view.verification_detail.clone()),
        source,
    }
}

fn messaging_channel_readiness_entry(
    descriptor: &crate::channels::messaging_registry::ChannelDescriptor,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(
            CapabilitySurface::MessagingChannel,
            descriptor.id.clone(),
        ),
        label: Some(descriptor.display_name.clone()),
        status: if descriptor.configured {
            CapabilityReadinessStatus::Connected
        } else if descriptor.requires_auth {
            CapabilityReadinessStatus::NeedsAuth
        } else {
            CapabilityReadinessStatus::NotConfigured
        },
        enabled: descriptor.configured,
        last_verified: Some(now),
        expires_at: None,
        stale_after_seconds: None,
        last_error: None,
        source,
    }
}

fn model_provider_readiness_entry(
    slot: &crate::core::runtime::config::ModelSlot,
    runtime_ready: bool,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(CapabilitySurface::ModelProvider, slot.id.clone()),
        label: Some(if slot.label.trim().is_empty() {
            slot.id.clone()
        } else {
            slot.label.clone()
        }),
        status: if runtime_ready {
            CapabilityReadinessStatus::Connected
        } else {
            CapabilityReadinessStatus::NeedsAuth
        },
        enabled: slot.enabled,
        last_verified: Some(now),
        expires_at: None,
        stale_after_seconds: None,
        last_error: None,
        source,
    }
}

fn plugin_readiness_entry(
    view: &crate::plugins::registry::PluginView,
    source: CapabilityReadinessSource,
    now: DateTime<Utc>,
) -> CapabilityReadinessEntry {
    let status = if view.plugin.last_error.is_some() {
        CapabilityReadinessStatus::Error
    } else if !view.token_configured
        && !matches!(
            view.plugin.auth_mode,
            crate::plugins::registry::PluginAuthMode::None
        )
    {
        CapabilityReadinessStatus::NeedsAuth
    } else {
        CapabilityReadinessStatus::Connected
    };
    CapabilityReadinessEntry {
        key: CapabilityReadinessKey::new(CapabilitySurface::Plugin, view.plugin.id.clone()),
        label: Some(view.plugin.name.clone()),
        status,
        enabled: view.plugin.enabled,
        last_verified: parse_utc_rfc3339(view.plugin.last_synced_at.as_deref()).or(Some(now)),
        expires_at: None,
        stale_after_seconds: None,
        last_error: view.plugin.last_error.clone(),
        source,
    }
}

impl Agent {
    async fn persist_capability_readiness_registry(&self, snapshot: CapabilityReadinessRegistry) {
        let storage = self.storage.clone();
        crate::spawn_logged!(
            "src/core/agent/capability_readiness.rs:persist_registry",
            async move {
                match serde_json::to_vec(&snapshot) {
                    Ok(raw) => {
                        if let Err(error) = storage.set(CAPABILITY_READINESS_KV_KEY, &raw).await {
                            tracing::warn!(
                                "Failed to persist capability readiness registry: {}",
                                error
                            );
                        }
                    }
                    Err(error) => tracing::warn!(
                        "Failed to serialize capability readiness registry: {}",
                        error
                    ),
                }
            }
        );
    }

    pub async fn replace_capability_readiness_surface_entries(
        &self,
        surface: CapabilitySurface,
        entries: Vec<CapabilityReadinessEntry>,
    ) -> bool {
        let (changed, generation, snapshot) = {
            let mut registry = self.capability_readiness.write().await;
            let changed = registry.replace_surface_entries(surface, entries);
            let generation = registry.generation();
            let snapshot = changed.then(|| registry.clone());
            (changed, generation, snapshot)
        };
        if !changed {
            return false;
        }
        let _ = self
            .capability_readiness_events
            .send(CapabilityReadinessEvent { generation });
        if let Some(snapshot) = snapshot {
            self.persist_capability_readiness_registry(snapshot).await;
        }
        true
    }

    pub async fn capability_readiness_generation(&self) -> u64 {
        self.capability_readiness.read().await.generation()
    }

    pub async fn capability_readiness_context_message(&self) -> Option<String> {
        self.capability_readiness
            .read()
            .await
            .system_context_message(chrono::Utc::now(), CAPABILITY_READINESS_CONTEXT_MAX_ENTRIES)
    }

    pub async fn mark_capability_readiness_failure_for_action_scope(
        &self,
        scope: Option<&crate::runtime::ActionScopeHint>,
        error: &str,
    ) -> bool {
        let Some(scope) = scope else {
            return false;
        };
        let (changed, generation, snapshot) = {
            let mut registry = self.capability_readiness.write().await;
            let changed = registry.mark_action_scope_failure(scope, error);
            let generation = registry.generation();
            let snapshot = changed.then(|| registry.clone());
            (changed, generation, snapshot)
        };
        if !changed {
            return false;
        }
        let _ = self
            .capability_readiness_events
            .send(CapabilityReadinessEvent { generation });
        if let Some(snapshot) = snapshot {
            self.persist_capability_readiness_registry(snapshot).await;
        }
        true
    }

    pub async fn refresh_integration_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        let entries = self
            .integrations
            .list()
            .await
            .into_iter()
            .map(|info| {
                let enabled = self.integrations.is_enabled(&info.id);
                integration_readiness_entry(&info, enabled, source, now)
            })
            .collect();
        self.replace_capability_readiness_surface_entries(CapabilitySurface::Integration, entries)
            .await;
    }

    pub async fn refresh_custom_api_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        match crate::custom_apis::list_custom_apis(&self.storage, &self.config_dir, &self.data_dir)
            .await
        {
            Ok(views) => {
                let entries = views
                    .iter()
                    .map(|view| custom_api_readiness_entry(view, source, now))
                    .collect();
                self.replace_capability_readiness_surface_entries(
                    CapabilitySurface::CustomApi,
                    entries,
                )
                .await;
            }
            Err(error) => tracing::warn!("Failed to refresh custom API readiness: {}", error),
        }
    }

    pub async fn refresh_mcp_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        match self.mcp.read().await.list_servers(false).await {
            Ok(views) => {
                let entries = views
                    .iter()
                    .map(|view| mcp_server_readiness_entry(view, source, now))
                    .collect();
                self.replace_capability_readiness_surface_entries(
                    CapabilitySurface::McpServer,
                    entries,
                )
                .await;
            }
            Err(error) => tracing::warn!("Failed to refresh MCP readiness: {}", error),
        }
    }

    pub async fn refresh_extension_pack_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        match self.extension_packs.read().await.list_installed(None).await {
            Ok(views) => {
                let entries = views
                    .iter()
                    .map(|view| extension_pack_readiness_entry(view, source, now))
                    .collect();
                self.replace_capability_readiness_surface_entries(
                    CapabilitySurface::ExtensionPack,
                    entries,
                )
                .await;
            }
            Err(error) => tracing::warn!("Failed to refresh extension pack readiness: {}", error),
        }
    }

    pub async fn refresh_messaging_channel_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )
        .ok();
        let packs_guard = self.extension_packs.read().await;
        struct AgentBundledCheck<'a>(&'a Agent);
        impl<'a> crate::channels::messaging_registry::BundledConfiguredCheck for AgentBundledCheck<'a> {
            fn is_configured(&self, channel_id: &str) -> bool {
                self.0.notification_channel_is_configured(channel_id)
            }
        }
        let bundled_check = AgentBundledCheck(self);
        let ctx = crate::channels::messaging_registry::ChannelQueryContext {
            bundled_configured: &bundled_check,
            extension_packs: &packs_guard,
            storage: &self.storage,
            config_dir: &self.config_dir,
            data_dir: &self.data_dir,
            config_manager: manager.as_ref(),
        };
        match crate::channels::messaging_registry::MessagingChannelRegistry::new()
            .list(&ctx)
            .await
        {
            Ok(descriptors) => {
                let entries = descriptors
                    .iter()
                    .map(|descriptor| messaging_channel_readiness_entry(descriptor, source, now))
                    .collect();
                self.replace_capability_readiness_surface_entries(
                    CapabilitySurface::MessagingChannel,
                    entries,
                )
                .await;
            }
            Err(error) => {
                tracing::warn!("Failed to refresh messaging channel readiness: {}", error)
            }
        }
    }

    pub async fn refresh_model_provider_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        let entries = self
            .config
            .model_pool
            .slots
            .iter()
            .map(|slot| {
                model_provider_readiness_entry(
                    slot,
                    self.model_pool.contains_key(&slot.id),
                    source,
                    now,
                )
            })
            .collect();
        self.replace_capability_readiness_surface_entries(
            CapabilitySurface::ModelProvider,
            entries,
        )
        .await;
    }

    pub async fn refresh_plugin_capability_readiness_snapshot(
        &self,
        source: CapabilityReadinessSource,
    ) {
        let now = chrono::Utc::now();
        match self.plugins.read().await.list_plugins().await {
            Ok(views) => {
                let entries = views
                    .iter()
                    .map(|view| plugin_readiness_entry(view, source, now))
                    .collect();
                self.replace_capability_readiness_surface_entries(
                    CapabilitySurface::Plugin,
                    entries,
                )
                .await;
            }
            Err(error) => tracing::warn!("Failed to refresh plugin readiness: {}", error),
        }
    }

    pub async fn refresh_capability_readiness_snapshot(&self, source: CapabilityReadinessSource) {
        self.refresh_integration_capability_readiness_snapshot(source)
            .await;
        self.refresh_custom_api_capability_readiness_snapshot(source)
            .await;
        self.refresh_mcp_capability_readiness_snapshot(source).await;
        self.refresh_extension_pack_capability_readiness_snapshot(source)
            .await;
        self.refresh_messaging_channel_capability_readiness_snapshot(source)
            .await;
        self.refresh_model_provider_capability_readiness_snapshot(source)
            .await;
        self.refresh_plugin_capability_readiness_snapshot(source)
            .await;
    }

    pub async fn refresh_capability_readiness_for_action_scope(
        &self,
        scope: Option<&crate::runtime::ActionScopeHint>,
        source: CapabilityReadinessSource,
    ) {
        let Some(scope) = scope else {
            return;
        };
        if !scope.integration_ids.is_empty() {
            self.refresh_integration_capability_readiness_snapshot(source)
                .await;
        }
        if scope.custom_api_id.is_some() {
            self.refresh_custom_api_capability_readiness_snapshot(source)
                .await;
        }
        if scope.mcp_server_id.is_some() {
            self.refresh_mcp_capability_readiness_snapshot(source).await;
        }
        if scope.plugin_id.is_some() {
            self.refresh_plugin_capability_readiness_snapshot(source)
                .await;
        }
        if !scope.extension_pack_ids.is_empty() {
            self.refresh_extension_pack_capability_readiness_snapshot(source)
                .await;
            self.refresh_messaging_channel_capability_readiness_snapshot(source)
                .await;
        }
        if !scope.channel_targets.is_empty() {
            self.refresh_messaging_channel_capability_readiness_snapshot(source)
                .await;
        }
    }

    pub fn spawn_capability_readiness_refresh_for_action_scope(
        &self,
        scope: Option<crate::runtime::ActionScopeHint>,
        source: CapabilityReadinessSource,
        error: Option<String>,
    ) {
        let Some(scope) = scope else {
            return;
        };
        let agent = self.clone();
        crate::spawn_logged!(
            "src/core/agent/capability_readiness.rs:action_scope_refresh",
            async move {
                if source == CapabilityReadinessSource::UseTimeFailure {
                    agent
                        .mark_capability_readiness_failure_for_action_scope(
                            Some(&scope),
                            error.as_deref().unwrap_or("Capability use failed"),
                        )
                        .await;
                } else {
                    agent
                        .refresh_capability_readiness_for_action_scope(Some(&scope), source)
                        .await;
                }
            }
        );
    }

    pub fn spawn_capability_readiness_boot_probe(&self) {
        let agent = self.clone();
        crate::spawn_logged!(
            "src/core/agent/capability_readiness.rs:boot_probe",
            async move {
                agent
                    .refresh_capability_readiness_snapshot(CapabilityReadinessSource::BootProbe)
                    .await;
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn ts(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).single().unwrap()
    }

    #[test]
    fn upsert_increments_generation_only_when_entry_changes() {
        let mut registry = CapabilityReadinessRegistry::default();
        let entry = CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "gmail"),
            label: Some("Gmail".to_string()),
            status: CapabilityReadinessStatus::Connected,
            enabled: true,
            last_verified: Some(ts(10)),
            expires_at: None,
            stale_after_seconds: Some(300),
            last_error: None,
            source: CapabilityReadinessSource::StatusProbe,
        };

        assert_eq!(registry.generation(), 0);
        assert!(registry.upsert(entry.clone()));
        assert_eq!(registry.generation(), 1);
        assert!(!registry.upsert(entry));
        assert_eq!(registry.generation(), 1);
    }

    #[test]
    fn compact_snapshot_excludes_unconfigured_but_keeps_blocked_and_stale_entries() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.upsert(CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "gmail"),
            label: Some("Gmail".to_string()),
            status: CapabilityReadinessStatus::Connected,
            enabled: true,
            last_verified: Some(ts(10)),
            expires_at: None,
            stale_after_seconds: Some(30),
            last_error: None,
            source: CapabilityReadinessSource::StatusProbe,
        });
        registry.upsert(CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "calendar"),
            label: Some("Calendar".to_string()),
            status: CapabilityReadinessStatus::NotConfigured,
            enabled: true,
            last_verified: Some(ts(20)),
            expires_at: None,
            stale_after_seconds: Some(300),
            last_error: None,
            source: CapabilityReadinessSource::BootProbe,
        });
        registry.upsert(CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::McpServer, "filesystem"),
            label: Some("Filesystem MCP".to_string()),
            status: CapabilityReadinessStatus::NeedsAuth,
            enabled: true,
            last_verified: Some(ts(25)),
            expires_at: None,
            stale_after_seconds: Some(300),
            last_error: None,
            source: CapabilityReadinessSource::RuntimeEvent,
        });

        let snapshot = registry.compact_snapshot(ts(60), 20);

        assert_eq!(snapshot.generation, 3);
        assert_eq!(snapshot.entries.len(), 2);
        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.id == "gmail" && entry.stale));
        assert!(snapshot
            .entries
            .iter()
            .any(|entry| entry.id == "filesystem" && entry.status == "needs_auth"));
        assert!(!snapshot.entries.iter().any(|entry| entry.id == "calendar"));
    }

    #[test]
    fn replace_surface_entries_removes_absent_capabilities() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.upsert(CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::CustomApi, "old_api"),
            label: Some("Old API".to_string()),
            status: CapabilityReadinessStatus::Connected,
            enabled: true,
            last_verified: Some(ts(100)),
            expires_at: None,
            stale_after_seconds: None,
            last_error: None,
            source: CapabilityReadinessSource::StatusProbe,
        });
        registry.upsert(CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "gmail"),
            label: Some("Gmail".to_string()),
            status: CapabilityReadinessStatus::Connected,
            enabled: true,
            last_verified: Some(ts(100)),
            expires_at: None,
            stale_after_seconds: None,
            last_error: None,
            source: CapabilityReadinessSource::StatusProbe,
        });

        assert!(registry.replace_surface_entries(CapabilitySurface::CustomApi, Vec::new()));

        assert!(registry
            .get(&CapabilityReadinessKey::new(
                CapabilitySurface::CustomApi,
                "old_api"
            ))
            .is_none());
        assert!(registry
            .get(&CapabilityReadinessKey::new(
                CapabilitySurface::Integration,
                "gmail"
            ))
            .is_some());
    }

    #[test]
    fn registry_json_persistence_round_trips_structural_keys() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.replace_surface_entries(
            CapabilitySurface::CustomApi,
            vec![CapabilityReadinessEntry {
                key: CapabilityReadinessKey::new(CapabilitySurface::CustomApi, "weather_api"),
                label: Some("Weather API".to_string()),
                status: CapabilityReadinessStatus::Connected,
                enabled: true,
                last_verified: Some(ts(100)),
                expires_at: None,
                stale_after_seconds: None,
                last_error: None,
                source: CapabilityReadinessSource::RuntimeEvent,
            }],
        );

        let raw =
            serde_json::to_vec(&registry).expect("registry must serialize for kv persistence");
        let restored: CapabilityReadinessRegistry =
            serde_json::from_slice(&raw).expect("registry must deserialize from kv persistence");

        assert_eq!(restored.generation(), registry.generation());
        assert!(restored
            .get(&CapabilityReadinessKey::new(
                CapabilitySurface::CustomApi,
                "weather_api"
            ))
            .is_some());
    }

    #[test]
    fn replace_surface_entries_does_not_bump_generation_for_timestamp_only_refresh() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.replace_surface_entries(
            CapabilitySurface::Integration,
            vec![CapabilityReadinessEntry {
                key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "github"),
                label: Some("GitHub".to_string()),
                status: CapabilityReadinessStatus::Connected,
                enabled: true,
                last_verified: Some(ts(100)),
                expires_at: None,
                stale_after_seconds: Some(300),
                last_error: None,
                source: CapabilityReadinessSource::RuntimeEvent,
            }],
        );

        let changed = registry.replace_surface_entries(
            CapabilitySurface::Integration,
            vec![CapabilityReadinessEntry {
                key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "github"),
                label: Some("GitHub".to_string()),
                status: CapabilityReadinessStatus::Connected,
                enabled: true,
                last_verified: Some(ts(120)),
                expires_at: None,
                stale_after_seconds: Some(300),
                last_error: None,
                source: CapabilityReadinessSource::RuntimeEvent,
            }],
        );

        assert!(!changed);
        assert_eq!(registry.generation(), 1);
        assert_eq!(
            registry
                .get(&CapabilityReadinessKey::new(
                    CapabilitySurface::Integration,
                    "github"
                ))
                .and_then(|entry| entry.last_verified),
            Some(ts(120))
        );
    }

    #[test]
    fn mark_action_scope_failure_records_error_without_resetting_last_verified() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.replace_surface_entries(
            CapabilitySurface::Integration,
            vec![CapabilityReadinessEntry {
                key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "github"),
                label: Some("GitHub".to_string()),
                status: CapabilityReadinessStatus::Connected,
                enabled: true,
                last_verified: Some(ts(100)),
                expires_at: None,
                stale_after_seconds: Some(300),
                last_error: None,
                source: CapabilityReadinessSource::RuntimeEvent,
            }],
        );
        let scope = crate::runtime::ActionScopeHint {
            integration_ids: vec!["github".to_string()],
            ..Default::default()
        };

        assert!(registry.mark_action_scope_failure(&scope, "401 token revoked"));

        let entry = registry
            .get(&CapabilityReadinessKey::new(
                CapabilitySurface::Integration,
                "github",
            ))
            .expect("entry should still exist");
        assert_eq!(entry.status, CapabilityReadinessStatus::Error);
        assert_eq!(entry.last_verified, Some(ts(100)));
        assert_eq!(entry.last_error.as_deref(), Some("401 token revoked"));
        assert_eq!(entry.source, CapabilityReadinessSource::UseTimeFailure);
    }

    #[test]
    fn compact_snapshot_caps_id_and_label_lengths() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.replace_surface_entries(
            CapabilitySurface::Plugin,
            vec![CapabilityReadinessEntry {
                key: CapabilityReadinessKey::new(CapabilitySurface::Plugin, "p".repeat(300)),
                label: Some("label".repeat(80)),
                status: CapabilityReadinessStatus::Connected,
                enabled: true,
                last_verified: Some(ts(100)),
                expires_at: None,
                stale_after_seconds: None,
                last_error: None,
                source: CapabilityReadinessSource::RuntimeEvent,
            }],
        );

        let snapshot = registry.compact_snapshot(ts(120), 20);
        let entry = snapshot.entries.first().expect("snapshot entry");

        assert!(entry.id.len() <= 120);
        assert!(entry.label.as_ref().is_some_and(|label| label.len() <= 120));
    }

    #[test]
    fn system_context_is_structured_and_non_secret() {
        let mut registry = CapabilityReadinessRegistry::default();
        registry.upsert(CapabilityReadinessEntry {
            key: CapabilityReadinessKey::new(CapabilitySurface::Integration, "github"),
            label: Some("GitHub".to_string()),
            status: CapabilityReadinessStatus::Error,
            enabled: true,
            last_verified: Some(ts(100)),
            expires_at: None,
            stale_after_seconds: Some(300),
            last_error: Some("API returned 401".to_string()),
            source: CapabilityReadinessSource::UseTimeFailure,
        });

        let context = registry.system_context_message(ts(120), 20).unwrap();

        assert!(context.starts_with("Capability readiness context:\n"));
        assert!(context.contains("\"generation\":1"));
        assert!(context.contains("\"status\":\"error\""));
        assert!(context.contains("\"last_error\":\"API returned 401\""));
        assert!(!context.contains("token"));
        assert!(!context.contains("secret"));
    }
}
