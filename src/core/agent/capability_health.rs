use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapabilityReadiness {
    Ready,
    AuthRequired,
    SetupRequired,
    Busy,
    Degraded,
    RateLimited,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CapabilityHealthEntry {
    pub action_name: String,
    pub readiness: CapabilityReadiness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated: Option<bool>,
    #[serde(default)]
    pub rate_limited: bool,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub missing_setup: Vec<String>,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub configured_notification_channels: Vec<String>,
    #[serde(default)]
    pub contract: Option<super::tool_contracts::ToolContractSummary>,
}

#[derive(Debug, Clone)]
pub(super) struct CapabilityHealthSnapshot {
    pub entries: Arc<BTreeMap<String, CapabilityHealthEntry>>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub generation: usize,
    pub capability_fingerprint: String,
    pub health_fingerprint: String,
    pub cache_hit: bool,
}

impl CapabilityHealthSnapshot {
    fn fresh_clone(&self, cache_hit: bool) -> Self {
        Self {
            entries: Arc::clone(&self.entries),
            generated_at: self.generated_at,
            generation: self.generation,
            capability_fingerprint: self.capability_fingerprint.clone(),
            health_fingerprint: self.health_fingerprint.clone(),
            cache_hit,
        }
    }

    pub(super) fn entry(&self, action_name: &str) -> Option<&CapabilityHealthEntry> {
        self.entries.get(action_name)
    }

    pub(super) fn compact_entries_for_actions(
        &self,
        action_names: impl IntoIterator<Item = String>,
    ) -> Vec<serde_json::Value> {
        let mut entries = action_names
            .into_iter()
            .filter_map(|name| self.entry(&name))
            .map(|entry| {
                serde_json::json!({
                    "action": entry.action_name,
                    "readiness": &entry.readiness,
                    "authenticated": entry.authenticated,
                    "busy": entry.busy,
                    "degraded": entry.degraded,
                    "missing_setup": entry.missing_setup.iter().take(4).collect::<Vec<_>>(),
                    "reasons": entry.reasons.iter().take(4).collect::<Vec<_>>(),
                    "configured_notification_channels": entry.configured_notification_channels.iter().take(6).collect::<Vec<_>>(),
                    "contract": entry.contract.as_ref().map(|contract| serde_json::json!({
                        "required_input": contract.required_input.iter().take(8).collect::<Vec<_>>(),
                        "input_completeness": &contract.input_completeness,
                        "side_effect_level": &contract.side_effect_level,
                        "delivery_mode": &contract.delivery_mode,
                        "auth_required": contract.auth_required,
                        "idempotency": &contract.idempotency,
                        "cost": &contract.cost,
                        "output_shape": &contract.output_shape,
                    })),
                })
            })
            .collect::<Vec<_>>();
        entries.truncate(48);
        entries
    }

    pub(super) fn summary_for_prompt(&self) -> serde_json::Value {
        let mut by_readiness = BTreeMap::<String, usize>::new();
        let mut configured_channels = BTreeSet::new();
        for entry in self.entries.values() {
            *by_readiness
                .entry(format!("{:?}", &entry.readiness).to_ascii_lowercase())
                .or_default() += 1;
            configured_channels.extend(entry.configured_notification_channels.iter().cloned());
        }
        serde_json::json!({
            "generated_at": self.generated_at.to_rfc3339(),
            "cache_hit": self.cache_hit,
            "actions": self.entries.len(),
            "readiness_counts": by_readiness,
            "configured_notification_channels": configured_channels.into_iter().take(8).collect::<Vec<_>>(),
        })
    }

    pub(super) fn trace_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "generated_at": self.generated_at.to_rfc3339(),
            "generation": self.generation,
            "capability_fingerprint": &self.capability_fingerprint,
            "health_fingerprint": &self.health_fingerprint,
            "cache_hit": self.cache_hit,
            "entries": self.entries.len(),
            "summary": self.summary_for_prompt(),
        })
    }
}

fn capability_health_ttl_ms() -> i64 {
    std::env::var("AGENTARK_CAPABILITY_HEALTH_TTL_MS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(120_000)
        .clamp(1_000, 3_600_000)
}

fn health_snapshot_is_fresh(
    snapshot: &CapabilityHealthSnapshot,
    generation: usize,
    capability_fingerprint: &str,
) -> bool {
    let age_ms = chrono::Utc::now()
        .signed_duration_since(snapshot.generated_at)
        .num_milliseconds();
    snapshot.generation == generation
        && snapshot.capability_fingerprint == capability_fingerprint
        && age_ms >= 0
        && age_ms <= capability_health_ttl_ms()
}

impl Agent {
    pub(super) async fn invalidate_capability_health_snapshot(&self, reason: &'static str) {
        self.capability_health_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        *self.capability_health_snapshot.write().await = None;
        tracing::debug!(reason, "Capability health snapshot invalidated");
    }

    pub(super) async fn load_capability_health_snapshot(
        &self,
        capability_snapshot: &super::semantic_turn::CapabilitySnapshot,
    ) -> anyhow::Result<CapabilityHealthSnapshot> {
        let generation = self
            .capability_health_generation
            .load(std::sync::atomic::Ordering::Acquire);
        if let Some(snapshot) = self
            .capability_health_snapshot
            .read()
            .await
            .as_ref()
            .filter(|snapshot| {
                health_snapshot_is_fresh(snapshot, generation, &capability_snapshot.fingerprint)
            })
            .map(|snapshot| snapshot.fresh_clone(true))
        {
            return Ok(snapshot);
        }

        let _refresh_guard = self.capability_health_refresh.lock().await;
        if let Some(snapshot) = self
            .capability_health_snapshot
            .read()
            .await
            .as_ref()
            .filter(|snapshot| {
                health_snapshot_is_fresh(snapshot, generation, &capability_snapshot.fingerprint)
            })
            .map(|snapshot| snapshot.fresh_clone(true))
        {
            return Ok(snapshot);
        }

        let configured_channels = self.configured_notification_channels().await;
        let configured_channel_set = configured_channels
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let integration_ids = self.integrations.ids().into_iter().collect::<BTreeSet<_>>();
        let mut entries = BTreeMap::new();

        for action in capability_snapshot.actions.iter() {
            let review = self.runtime.get_action_review(&action.name).await;
            let entry = health_entry_for_action(
                self,
                action,
                review.as_ref(),
                &configured_channels,
                &configured_channel_set,
                &integration_ids,
            );
            entries.insert(action.name.clone(), entry);
        }

        let health_fingerprint = health_entries_fingerprint(&entries);
        let snapshot = CapabilityHealthSnapshot {
            entries: Arc::new(entries),
            generated_at: chrono::Utc::now(),
            generation,
            capability_fingerprint: capability_snapshot.fingerprint.clone(),
            health_fingerprint,
            cache_hit: false,
        };
        *self.capability_health_snapshot.write().await = Some(snapshot.fresh_clone(false));
        Ok(snapshot)
    }
}

fn health_entry_for_action(
    agent: &Agent,
    action: &crate::actions::ActionDef,
    review: Option<&crate::runtime::ActionReviewSnapshot>,
    configured_channels: &[String],
    configured_channel_set: &BTreeSet<String>,
    integration_ids: &BTreeSet<String>,
) -> CapabilityHealthEntry {
    let metadata = action.action_metadata();
    let mut readiness = CapabilityReadiness::Ready;
    let mut authenticated = None;
    let mut missing_setup = Vec::new();
    let mut reasons = Vec::new();
    let mut degraded = false;
    let mut busy = false;

    if let Some(review) = review {
        if !review.allow_execute
            || matches!(
                &review.status,
                crate::runtime::ActionReviewStatus::Blocked
            )
        {
            readiness = CapabilityReadiness::SetupRequired;
            if let Some(reason) = review.blocked_reason.as_ref() {
                missing_setup.push(safe_truncate(reason, 180));
            }
        } else if matches!(
            &review.status,
            crate::runtime::ActionReviewStatus::NeedsSecrets
        ) {
            readiness = CapabilityReadiness::AuthRequired;
            missing_setup.extend(review.missing_env.iter().take(4).cloned());
        } else if matches!(
            &review.status,
            crate::runtime::ActionReviewStatus::Warning
        ) {
            readiness = CapabilityReadiness::Degraded;
            degraded = true;
        }
        authenticated = Some(!review.requires_auth || review.auth_configured);
        reasons.extend(review.notes.iter().take(4).map(|note| safe_truncate(note, 180)));
        reasons.extend(
            review
                .warnings
                .iter()
                .take(4)
                .map(|warning| safe_truncate(warning, 180)),
        );
    }

    for integration_id in &action.authorization.access.integration_ids {
        if !integration_ids.contains(integration_id)
            && !crate::integrations::effective_integration_enabled(&agent.config_dir, integration_id)
        {
            readiness = stronger_readiness(readiness, CapabilityReadiness::SetupRequired);
            missing_setup.push(format!("integration:{integration_id}"));
            authenticated = Some(false);
            continue;
        }
        if !crate::integrations::effective_integration_enabled(&agent.config_dir, integration_id) {
            readiness = stronger_readiness(readiness, CapabilityReadiness::SetupRequired);
            missing_setup.push(format!("integration_disabled:{integration_id}"));
            authenticated = Some(false);
        } else {
            authenticated.get_or_insert(true);
        }
    }

    for (integration_id, features) in &action.authorization.access.integration_features {
        let status = integration_feature_status(agent, integration_id, features);
        match status {
            IntegrationFeatureStatus::Ready => {
                authenticated.get_or_insert(true);
            }
            IntegrationFeatureStatus::NeedsAuth(missing) => {
                readiness = stronger_readiness(readiness, CapabilityReadiness::AuthRequired);
                authenticated = Some(false);
                missing_setup.extend(missing);
            }
            IntegrationFeatureStatus::Unknown => {
                readiness = stronger_readiness(readiness, CapabilityReadiness::Unknown);
                reasons.push(format!("integration_feature_status_unknown:{integration_id}"));
            }
        }
    }

    for target in &action.authorization.access.channel_targets {
        let target_ready =
            channel_target_is_ready(&target.default_target, configured_channel_set);
        if !target_ready {
            degraded = true;
            readiness = stronger_readiness(readiness, CapabilityReadiness::Degraded);
            missing_setup.push(format!("notification_channel:{}", target.default_target));
        }
    }

    if matches!(
        metadata.integration_class,
        crate::actions::ActionIntegrationClass::Browser
    ) && agent.browser_sessions.active_count() >= 2
    {
        busy = true;
        readiness = stronger_readiness(readiness, CapabilityReadiness::Busy);
        reasons.push("browser_session_limit_reached".to_string());
    }

    if action.authorization.rate_limit.is_some() {
        reasons.push("rate_limit_policy_present".to_string());
    }

    CapabilityHealthEntry {
        action_name: action.name.clone(),
        readiness,
        authenticated,
        rate_limited: false,
        stale: false,
        degraded,
        busy,
        missing_setup: dedup_truncated(missing_setup, 8),
        reasons: dedup_truncated(reasons, 8),
        configured_notification_channels: configured_channels.iter().take(8).cloned().collect(),
        contract: Some(super::tool_contracts::contract_summary_for_action(action, None)),
    }
}

enum IntegrationFeatureStatus {
    Ready,
    NeedsAuth(Vec<String>),
    Unknown,
}

fn integration_feature_status(
    agent: &Agent,
    integration_id: &str,
    features: &[String],
) -> IntegrationFeatureStatus {
    if integration_id == "google_workspace" {
        let granted =
            crate::actions::google_workspace::granted_bundles(&agent.config_dir).unwrap_or_default();
        if granted.is_empty() {
            return IntegrationFeatureStatus::NeedsAuth(vec![
                "integration_feature:google_workspace".to_string(),
            ]);
        }
        let missing = features
            .iter()
            .filter(|feature| {
                !granted
                    .iter()
                    .any(|granted_feature| granted_feature.eq_ignore_ascii_case(feature))
            })
            .map(|feature| format!("integration_feature:{integration_id}/{feature}"))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            IntegrationFeatureStatus::Ready
        } else {
            IntegrationFeatureStatus::NeedsAuth(missing)
        }
    } else if features.is_empty() {
        IntegrationFeatureStatus::Ready
    } else {
        IntegrationFeatureStatus::Unknown
    }
}

fn channel_target_is_ready(default_target: &str, configured_channel_set: &BTreeSet<String>) -> bool {
    let target = default_target.trim().to_ascii_lowercase();
    target.is_empty()
        || target == "preferred"
        || target == "in_app"
        || target == "web"
        || configured_channel_set.contains(&target)
}

fn stronger_readiness(
    current: CapabilityReadiness,
    next: CapabilityReadiness,
) -> CapabilityReadiness {
    if readiness_weight(&next) > readiness_weight(&current) {
        next
    } else {
        current
    }
}

fn readiness_weight(readiness: &CapabilityReadiness) -> u8 {
    match readiness {
        CapabilityReadiness::Ready => 0,
        CapabilityReadiness::Degraded => 1,
        CapabilityReadiness::Unknown => 2,
        CapabilityReadiness::AuthRequired => 3,
        CapabilityReadiness::SetupRequired => 4,
        CapabilityReadiness::Busy => 5,
        CapabilityReadiness::RateLimited => 6,
    }
}

fn dedup_truncated(values: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| safe_truncate(value.trim(), 220))
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .take(limit)
        .collect()
}

fn health_entries_fingerprint(entries: &BTreeMap<String, CapabilityHealthEntry>) -> String {
    let mut summaries = entries
        .iter()
        .map(|(name, entry)| {
            format!(
                "{}\u{1f}{:?}\u{1f}{:?}\u{1f}{}\u{1f}{}",
                name,
                entry.readiness,
                entry.authenticated,
                entry.missing_setup.join("\u{1e}"),
                entry.reasons.join("\u{1e}")
            )
        })
        .collect::<Vec<_>>();
    summaries.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    summaries.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_notification_channel_does_not_block_action_health() {
        let configured = BTreeSet::new();
        assert!(channel_target_is_ready("preferred", &configured));
        assert!(channel_target_is_ready("in_app", &configured));
        assert!(!channel_target_is_ready("telegram", &configured));
    }
}
