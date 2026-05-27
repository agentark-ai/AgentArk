//! Model failover control plane.
//!
//! KV-backed state for auth profiles, provider health, and fallback chains.

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::storage::Storage;

const AUTH_PROFILES_KEY: &str = "model_failover:auth_profiles:v1";
const PROVIDER_HEALTH_KEY: &str = "model_failover:provider_health:v1";
const FALLBACK_CHAINS_KEY: &str = "model_failover:fallback_chains:v1";

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
        .with_context(|| format!("failed to decode model failover payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode model failover payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

fn sanitize_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sanitize_list(values: Vec<String>) -> Vec<String> {
    let mut out = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn is_cooling(record: &ProviderHealthRecord) -> bool {
    let Some(until) = record.cooldown_until.as_deref() else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(until) {
        Ok(dt) => dt > chrono::Utc::now().with_timezone(dt.offset()),
        Err(_) => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelFailoverSummary {
    pub auth_profiles: usize,
    pub providers: usize,
    pub disabled_providers: usize,
    pub cooling_providers: usize,
    pub chains: usize,
    pub session_pins: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSessionPin {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfileRecord {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_pin: Option<ModelSessionPin>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthProfileUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_pin: Option<ModelSessionPin>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthRecord {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<String>,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub failure_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_note: Option<String>,
    #[serde(default)]
    pub session_pin_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderHealthUpsert {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<String>,
    #[serde(default)]
    pub success_count: Option<u64>,
    #[serde(default)]
    pub failure_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthEvent {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<String>,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackCandidate {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackChainRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ordered_candidates: Vec<FallbackCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_pin: Option<ModelSessionPin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FallbackChainUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub ordered_candidates: Vec<FallbackCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_pin: Option<ModelSessionPin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelFailoverListResponse {
    pub summary: ModelFailoverSummary,
    pub auth_profiles: Vec<AuthProfileRecord>,
    pub provider_health: Vec<ProviderHealthRecord>,
    pub fallback_chains: Vec<FallbackChainRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelFailoverSelectionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default)]
    pub allow_disabled: bool,
    #[serde(default)]
    pub allow_cooling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelFailoverSelectionResult {
    pub matched_chain_id: Option<String>,
    pub selected_provider_id: Option<String>,
    pub selected_auth_profile_id: Option<String>,
    #[serde(default)]
    pub ordered_candidates: Vec<FallbackCandidate>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_health: Option<ProviderHealthRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile: Option<AuthProfileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CooldownClearResult {
    pub cleared: usize,
}

fn normalize_profile(mut profile: AuthProfileRecord) -> AuthProfileRecord {
    profile.id = profile.id.trim().to_string();
    profile.name = profile.name.trim().to_string();
    profile.provider_id = profile.provider_id.trim().to_string();
    profile.provider_kind = sanitize_text(profile.provider_kind);
    profile.base_url = sanitize_text(profile.base_url);
    profile.model_id = sanitize_text(profile.model_id);
    profile.credential_ref = sanitize_text(profile.credential_ref);
    profile.last_used_at = sanitize_text(profile.last_used_at);
    profile.last_error = sanitize_text(profile.last_error);
    profile.tags = sanitize_list(profile.tags);
    profile
}

fn normalize_health(mut health: ProviderHealthRecord) -> ProviderHealthRecord {
    health.provider_id = health.provider_id.trim().to_string();
    health.provider_kind = sanitize_text(health.provider_kind);
    health.cooldown_until = sanitize_text(health.cooldown_until);
    health.last_success_at = sanitize_text(health.last_success_at);
    health.last_failure_at = sanitize_text(health.last_failure_at);
    health.last_error = sanitize_text(health.last_error);
    health.health_note = sanitize_text(health.health_note);
    health
}

fn normalize_chain(mut chain: FallbackChainRecord) -> FallbackChainRecord {
    chain.id = chain.id.trim().to_string();
    chain.name = chain.name.trim().to_string();
    chain.notes = sanitize_text(chain.notes);
    chain.ordered_candidates = chain
        .ordered_candidates
        .into_iter()
        .filter_map(|mut candidate| {
            candidate.provider_id = candidate.provider_id.trim().to_string();
            candidate.auth_profile_id = sanitize_text(candidate.auth_profile_id);
            candidate.reason = sanitize_text(candidate.reason);
            if candidate.provider_id.is_empty() {
                return None;
            }
            Some(candidate)
        })
        .collect();
    chain
}

fn profile_pins_count(profiles: &[AuthProfileRecord]) -> usize {
    profiles
        .iter()
        .filter(|profile| profile.session_pin.is_some())
        .count()
}

fn candidate_rank(
    candidate: &FallbackCandidate,
    provider_map: &std::collections::BTreeMap<String, ProviderHealthRecord>,
) -> (i32, bool, bool) {
    let provider_state = provider_map.get(&candidate.provider_id);
    let disabled = provider_state
        .map(|state| state.disabled || !state.enabled)
        .unwrap_or(false);
    let cooling = provider_state.map(is_cooling).unwrap_or(false);
    (candidate.priority, disabled, cooling)
}

fn chain_candidates(chain: &FallbackChainRecord) -> Vec<FallbackCandidate> {
    let mut candidates = chain.ordered_candidates.clone();
    candidates.sort_by_key(|candidate| candidate.priority);
    candidates
}

fn pick_chain<'a>(
    chains: &'a [FallbackChainRecord],
    request: &ModelFailoverSelectionRequest,
) -> Option<&'a FallbackChainRecord> {
    if let Some(chain_id) = request.chain_id.as_deref() {
        return chains.iter().find(|chain| chain.id == chain_id);
    }
    chains.iter().find(|chain| chain.enabled)
}

fn pick_candidate<'a>(
    candidates: &'a [FallbackCandidate],
    profiles: &'a [AuthProfileRecord],
    provider_map: &std::collections::BTreeMap<String, ProviderHealthRecord>,
    request: &ModelFailoverSelectionRequest,
) -> (Option<&'a FallbackCandidate>, Option<String>) {
    let mut ordered: Vec<&FallbackCandidate> = candidates.iter().collect();
    ordered.sort_by_key(|candidate| candidate_rank(candidate, provider_map));

    if let Some(session_id) = request.session_id.as_deref() {
        if let Some(candidate) = ordered.iter().copied().find(|candidate| {
            candidate
                .auth_profile_id
                .as_deref()
                .and_then(|auth_profile_id| {
                    profiles
                        .iter()
                        .find(|profile| profile.id == auth_profile_id)
                        .and_then(|profile| profile.session_pin.as_ref())
                        .map(|pin| pin.session_id.as_str() == session_id)
                })
                == Some(true)
        }) {
            return (
                Some(candidate),
                Some("matched session pin on auth profile".to_string()),
            );
        }
    }

    if let Some(provider_id) = request.provider_id.as_deref() {
        if let Some(candidate) = ordered
            .iter()
            .copied()
            .find(|candidate| candidate.provider_id == provider_id)
        {
            return (
                Some(candidate),
                Some("matched requested provider".to_string()),
            );
        }
    }

    if let Some(auth_profile_id) = request.auth_profile_id.as_deref() {
        if let Some(candidate) = ordered
            .iter()
            .copied()
            .find(|candidate| candidate.auth_profile_id.as_deref() == Some(auth_profile_id))
        {
            return (
                Some(candidate),
                Some("matched requested auth profile".to_string()),
            );
        }
    }

    for candidate in ordered {
        let state = provider_map.get(&candidate.provider_id);
        if !request.allow_disabled
            && state
                .map(|state| state.disabled || !state.enabled)
                .unwrap_or(false)
        {
            continue;
        }
        if !request.allow_cooling && state.map(is_cooling).unwrap_or(false) {
            continue;
        }
        return (
            Some(candidate),
            Some("selected by fallback order".to_string()),
        );
    }

    (None, Some("no candidate was eligible".to_string()))
}

fn merge_provider_record(
    existing: Option<ProviderHealthRecord>,
    input: ProviderHealthUpsert,
    pin_count: usize,
) -> ProviderHealthRecord {
    let now = now_rfc3339();
    let mut record = existing.unwrap_or_else(|| ProviderHealthRecord {
        provider_id: input.provider_id.clone(),
        provider_kind: input.provider_kind.clone(),
        enabled: true,
        disabled: false,
        cooldown_until: None,
        last_success_at: None,
        last_failure_at: None,
        success_count: 0,
        failure_count: 0,
        last_error: None,
        health_note: None,
        session_pin_count: pin_count,
        metadata: input.metadata.clone(),
    });

    record.provider_id = input.provider_id;
    if input.provider_kind.is_some() {
        record.provider_kind = input.provider_kind;
    }
    if let Some(enabled) = input.enabled {
        record.enabled = enabled;
    }
    if let Some(disabled) = input.disabled {
        record.disabled = disabled;
    }
    if input.cooldown_until.is_some() {
        record.cooldown_until = input.cooldown_until;
    }
    if input.last_success_at.is_some() {
        record.last_success_at = input.last_success_at;
    } else if record.last_success_at.is_none() && input.success_count.unwrap_or(0) > 0 {
        record.last_success_at = Some(now.clone());
    }
    if input.last_failure_at.is_some() {
        record.last_failure_at = input.last_failure_at;
    }
    if let Some(success_count) = input.success_count {
        record.success_count = success_count;
    }
    if let Some(failure_count) = input.failure_count {
        record.failure_count = failure_count;
    }
    if input.last_error.is_some() {
        record.last_error = input.last_error;
    }
    if input.health_note.is_some() {
        record.health_note = input.health_note;
    }
    if input.metadata.is_some() {
        record.metadata = input.metadata;
    }
    record.session_pin_count = pin_count;
    normalize_health(record)
}

pub struct ModelFailoverControlPlane;

impl ModelFailoverControlPlane {
    pub async fn list(storage: &Storage) -> Result<ModelFailoverListResponse> {
        let auth_profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let provider_health = load_json::<Vec<ProviderHealthRecord>>(storage, PROVIDER_HEALTH_KEY)
            .await?
            .into_iter()
            .map(normalize_health)
            .collect::<Vec<_>>();
        let fallback_chains = load_json::<Vec<FallbackChainRecord>>(storage, FALLBACK_CHAINS_KEY)
            .await?
            .into_iter()
            .map(normalize_chain)
            .collect::<Vec<_>>();

        Ok(ModelFailoverListResponse {
            summary: ModelFailoverSummary {
                auth_profiles: auth_profiles.len(),
                providers: provider_health.len(),
                disabled_providers: provider_health
                    .iter()
                    .filter(|record| record.disabled)
                    .count(),
                cooling_providers: provider_health
                    .iter()
                    .filter(|record| is_cooling(record))
                    .count(),
                chains: fallback_chains.len(),
                session_pins: profile_pins_count(&auth_profiles),
            },
            auth_profiles,
            provider_health,
            fallback_chains,
        })
    }

    pub async fn upsert_auth_profile(
        storage: &Storage,
        input: AuthProfileUpsert,
    ) -> Result<AuthProfileRecord> {
        let mut profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let id = input
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing = profiles.iter().position(|profile| profile.id == id);
        let now = now_rfc3339();
        let profile = if let Some(index) = existing {
            let mut profile = profiles[index].clone();
            if let Some(name) = input.name {
                profile.name = name;
            }
            profile.provider_id = input.provider_id;
            if input.provider_kind.is_some() {
                profile.provider_kind = input.provider_kind;
            }
            if input.base_url.is_some() {
                profile.base_url = input.base_url;
            }
            if input.model_id.is_some() {
                profile.model_id = input.model_id;
            }
            if input.credential_ref.is_some() {
                profile.credential_ref = input.credential_ref;
            }
            if let Some(enabled) = input.enabled {
                profile.enabled = enabled;
            }
            if let Some(priority) = input.priority {
                profile.priority = priority;
            }
            if input.last_used_at.is_some() {
                profile.last_used_at = input.last_used_at;
            } else if profile.last_used_at.is_none() && profile.enabled {
                profile.last_used_at = Some(now.clone());
            }
            if input.last_error.is_some() {
                profile.last_error = input.last_error;
            }
            if input.session_pin.is_some() {
                profile.session_pin = input.session_pin;
            }
            if !input.tags.is_empty() {
                profile.tags = input.tags;
            }
            if input.metadata.is_some() {
                profile.metadata = input.metadata;
            }
            normalize_profile(profile)
        } else {
            normalize_profile(AuthProfileRecord {
                id: id.clone(),
                name: input.name.unwrap_or_else(|| "Auth profile".to_string()),
                provider_id: input.provider_id,
                provider_kind: input.provider_kind,
                base_url: input.base_url,
                model_id: input.model_id,
                credential_ref: input.credential_ref,
                enabled: input.enabled.unwrap_or(true),
                priority: input.priority.unwrap_or(100),
                last_used_at: input.last_used_at.or(Some(now)),
                last_error: input.last_error,
                session_pin: input.session_pin,
                tags: input.tags,
                metadata: input.metadata,
            })
        };

        if let Some(index) = existing {
            profiles[index] = profile.clone();
        } else {
            profiles.push(profile.clone());
        }
        save_json(storage, AUTH_PROFILES_KEY, &profiles).await?;
        Ok(profile)
    }

    pub async fn disable_auth_profile(
        storage: &Storage,
        profile_id: &str,
        disabled: bool,
    ) -> Result<Option<AuthProfileRecord>> {
        let mut profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let Some(index) = profiles.iter().position(|profile| profile.id == profile_id) else {
            return Ok(None);
        };
        profiles[index].enabled = !disabled;
        if disabled {
            profiles[index].last_error = Some("disabled".to_string());
        }
        let profile = normalize_profile(profiles[index].clone());
        profiles[index] = profile.clone();
        save_json(storage, AUTH_PROFILES_KEY, &profiles).await?;
        Ok(Some(profile))
    }

    pub async fn upsert_provider_health(
        storage: &Storage,
        input: ProviderHealthUpsert,
    ) -> Result<ProviderHealthRecord> {
        let profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let mut health = load_json::<Vec<ProviderHealthRecord>>(storage, PROVIDER_HEALTH_KEY)
            .await?
            .into_iter()
            .map(normalize_health)
            .collect::<Vec<_>>();
        let existing = health
            .iter()
            .position(|record| record.provider_id == input.provider_id);
        let record = merge_provider_record(
            existing.map(|index| health[index].clone()),
            input,
            profile_pins_count(&profiles),
        );
        if let Some(index) = existing {
            health[index] = record.clone();
        } else {
            health.push(record.clone());
        }
        save_json(storage, PROVIDER_HEALTH_KEY, &health).await?;
        Ok(record)
    }

    pub async fn disable_provider(
        storage: &Storage,
        provider_id: &str,
        disabled: bool,
    ) -> Result<Option<ProviderHealthRecord>> {
        let mut health = load_json::<Vec<ProviderHealthRecord>>(storage, PROVIDER_HEALTH_KEY)
            .await?
            .into_iter()
            .map(normalize_health)
            .collect::<Vec<_>>();
        let Some(index) = health
            .iter()
            .position(|record| record.provider_id == provider_id)
        else {
            return Ok(None);
        };
        health[index].disabled = disabled;
        health[index].enabled = !disabled;
        if disabled {
            health[index].last_error = Some("disabled".to_string());
        } else {
            health[index].last_error = None;
        }
        let record = normalize_health(health[index].clone());
        health[index] = record.clone();
        save_json(storage, PROVIDER_HEALTH_KEY, &health).await?;
        Ok(Some(record))
    }

    pub async fn record_health(
        storage: &Storage,
        event: ProviderHealthEvent,
    ) -> Result<ProviderHealthRecord> {
        let profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let mut health = load_json::<Vec<ProviderHealthRecord>>(storage, PROVIDER_HEALTH_KEY)
            .await?
            .into_iter()
            .map(normalize_health)
            .collect::<Vec<_>>();
        let now = now_rfc3339();
        let existing = health
            .iter()
            .position(|record| record.provider_id == event.provider_id);
        let mut record = existing
            .map(|index| health[index].clone())
            .unwrap_or_else(|| ProviderHealthRecord {
                provider_id: event.provider_id.clone(),
                provider_kind: event.provider_kind.clone(),
                enabled: true,
                disabled: false,
                cooldown_until: None,
                last_success_at: None,
                last_failure_at: None,
                success_count: 0,
                failure_count: 0,
                last_error: None,
                health_note: None,
                session_pin_count: profile_pins_count(&profiles),
                metadata: event.metadata.clone(),
            });

        if event.provider_kind.is_some() {
            record.provider_kind = event.provider_kind;
        }
        if let Some(disabled) = event.disabled {
            record.disabled = disabled;
            record.enabled = !disabled;
        }
        if event.success {
            record.success_count = record.success_count.saturating_add(1);
            record.last_success_at = Some(now.clone());
            record.last_error = None;
            record.cooldown_until = None;
        } else {
            record.failure_count = record.failure_count.saturating_add(1);
            record.last_failure_at = Some(now.clone());
            record.last_error = event
                .error
                .clone()
                .or_else(|| Some("provider failure".to_string()));
            if let Some(cooldown_secs) = event.cooldown_secs {
                if cooldown_secs > 0 {
                    record.cooldown_until = Some(
                        (chrono::Utc::now() + chrono::Duration::seconds(cooldown_secs))
                            .to_rfc3339(),
                    );
                }
            }
        }
        if let Some(note) = event.note {
            record.health_note = sanitize_text(Some(note));
        }
        if event.metadata.is_some() {
            record.metadata = event.metadata;
        }
        record.session_pin_count = profile_pins_count(&profiles);
        record = normalize_health(record);

        if let Some(index) = existing {
            health[index] = record.clone();
        } else {
            health.push(record.clone());
        }
        save_json(storage, PROVIDER_HEALTH_KEY, &health).await?;
        Ok(record)
    }

    pub async fn upsert_fallback_chain(
        storage: &Storage,
        input: FallbackChainUpsert,
    ) -> Result<FallbackChainRecord> {
        let mut chains = load_json::<Vec<FallbackChainRecord>>(storage, FALLBACK_CHAINS_KEY)
            .await?
            .into_iter()
            .map(normalize_chain)
            .collect::<Vec<_>>();
        let id = input
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing = chains.iter().position(|chain| chain.id == id);
        let chain = if let Some(index) = existing {
            let mut chain = chains[index].clone();
            if let Some(name) = input.name {
                chain.name = name;
            }
            if let Some(enabled) = input.enabled {
                chain.enabled = enabled;
            }
            if !input.ordered_candidates.is_empty() {
                chain.ordered_candidates = input.ordered_candidates;
            }
            if input.session_pin.is_some() {
                chain.session_pin = input.session_pin;
            }
            if input.notes.is_some() {
                chain.notes = input.notes;
            }
            if input.metadata.is_some() {
                chain.metadata = input.metadata;
            }
            normalize_chain(chain)
        } else {
            normalize_chain(FallbackChainRecord {
                id: id.clone(),
                name: input.name.unwrap_or_else(|| "Fallback chain".to_string()),
                enabled: input.enabled.unwrap_or(true),
                ordered_candidates: input.ordered_candidates,
                session_pin: input.session_pin,
                notes: input.notes,
                metadata: input.metadata,
            })
        };

        if let Some(index) = existing {
            chains[index] = chain.clone();
        } else {
            chains.push(chain.clone());
        }
        save_json(storage, FALLBACK_CHAINS_KEY, &chains).await?;
        Ok(chain)
    }

    pub async fn select_candidate(
        storage: &Storage,
        request: ModelFailoverSelectionRequest,
    ) -> Result<ModelFailoverSelectionResult> {
        let chains = load_json::<Vec<FallbackChainRecord>>(storage, FALLBACK_CHAINS_KEY)
            .await?
            .into_iter()
            .map(normalize_chain)
            .collect::<Vec<_>>();
        let profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let health = load_json::<Vec<ProviderHealthRecord>>(storage, PROVIDER_HEALTH_KEY)
            .await?
            .into_iter()
            .map(normalize_health)
            .collect::<Vec<_>>();
        let provider_map = health
            .into_iter()
            .map(|record| (record.provider_id.clone(), record))
            .collect::<std::collections::BTreeMap<_, _>>();

        let Some(chain) = pick_chain(&chains, &request) else {
            return Ok(ModelFailoverSelectionResult {
                blocked: true,
                blocked_reason: Some("No enabled fallback chain found.".to_string()),
                ..Default::default()
            });
        };

        let candidates = chain_candidates(chain);
        let (selected, reason) = pick_candidate(&candidates, &profiles, &provider_map, &request);
        let Some(candidate) = selected else {
            return Ok(ModelFailoverSelectionResult {
                matched_chain_id: Some(chain.id.clone()),
                ordered_candidates: candidates,
                blocked: true,
                blocked_reason: Some("No eligible provider candidate was available.".to_string()),
                reason,
                ..Default::default()
            });
        };

        let provider_health = provider_map.get(&candidate.provider_id).cloned();
        let auth_profile = candidate
            .auth_profile_id
            .as_deref()
            .and_then(|auth_profile_id| {
                profiles
                    .iter()
                    .find(|profile| profile.id == auth_profile_id)
                    .cloned()
            });

        Ok(ModelFailoverSelectionResult {
            matched_chain_id: Some(chain.id.clone()),
            selected_provider_id: Some(candidate.provider_id.clone()),
            selected_auth_profile_id: candidate.auth_profile_id.clone(),
            ordered_candidates: candidates,
            blocked: false,
            blocked_reason: None,
            reason,
            provider_health,
            auth_profile,
        })
    }

    pub async fn clear_cooldowns(
        storage: &Storage,
        provider_id: Option<&str>,
    ) -> Result<CooldownClearResult> {
        let mut health = load_json::<Vec<ProviderHealthRecord>>(storage, PROVIDER_HEALTH_KEY)
            .await?
            .into_iter()
            .map(normalize_health)
            .collect::<Vec<_>>();
        let mut cleared = 0usize;
        for record in &mut health {
            if let Some(target) = provider_id {
                if record.provider_id != target {
                    continue;
                }
            }
            if record.cooldown_until.is_some() {
                record.cooldown_until = None;
                cleared += 1;
            }
        }
        save_json(storage, PROVIDER_HEALTH_KEY, &health).await?;
        Ok(CooldownClearResult { cleared })
    }

    pub async fn set_default_auth_profile(
        storage: &Storage,
        profile_id: &str,
    ) -> Result<Option<AuthProfileRecord>> {
        let mut profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let Some(index) = profiles.iter().position(|profile| profile.id == profile_id) else {
            return Ok(None);
        };
        let provider_id = profiles[index].provider_id.clone();
        for profile in &mut profiles {
            if profile.provider_id == provider_id {
                profile.priority = profile.priority.max(100);
                if let Some(metadata) = profile.metadata.as_mut() {
                    if let Some(object) = metadata.as_object_mut() {
                        object.insert("is_default".to_string(), serde_json::Value::Bool(false));
                    }
                }
            }
        }
        profiles[index].priority = 0;
        let mut metadata = profiles[index]
            .metadata
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(object) = metadata.as_object_mut() {
            object.insert("is_default".to_string(), serde_json::Value::Bool(true));
        }
        profiles[index].metadata = Some(metadata);
        let selected = normalize_profile(profiles[index].clone());
        profiles[index] = selected.clone();
        save_json(storage, AUTH_PROFILES_KEY, &profiles).await?;
        Ok(Some(selected))
    }

    pub async fn rotate_auth_profile(
        storage: &Storage,
        profile_id: &str,
    ) -> Result<ModelFailoverSelectionResult> {
        let profiles = load_json::<Vec<AuthProfileRecord>>(storage, AUTH_PROFILES_KEY)
            .await?
            .into_iter()
            .map(normalize_profile)
            .collect::<Vec<_>>();
        let Some(current) = profiles.iter().find(|profile| profile.id == profile_id) else {
            return Ok(ModelFailoverSelectionResult {
                blocked: true,
                blocked_reason: Some("Auth profile not found.".to_string()),
                ..Default::default()
            });
        };

        let mut candidates = profiles
            .iter()
            .filter(|profile| profile.provider_id == current.provider_id && profile.enabled)
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|profile| profile.priority);
        if let Some(next) = candidates
            .into_iter()
            .find(|profile| profile.id != current.id)
        {
            return Ok(ModelFailoverSelectionResult {
                matched_chain_id: None,
                selected_provider_id: Some(next.provider_id.clone()),
                selected_auth_profile_id: Some(next.id.clone()),
                ordered_candidates: vec![FallbackCandidate {
                    provider_id: next.provider_id.clone(),
                    auth_profile_id: Some(next.id.clone()),
                    priority: next.priority,
                    reason: Some(
                        "Rotated to the next enabled profile for the provider.".to_string(),
                    ),
                }],
                blocked: false,
                blocked_reason: None,
                reason: Some("Rotated to the next available auth profile.".to_string()),
                provider_health: None,
                auth_profile: Some(next),
            });
        }

        Ok(ModelFailoverSelectionResult {
            matched_chain_id: None,
            selected_provider_id: Some(current.provider_id.clone()),
            selected_auth_profile_id: Some(current.id.clone()),
            ordered_candidates: vec![FallbackCandidate {
                provider_id: current.provider_id.clone(),
                auth_profile_id: Some(current.id.clone()),
                priority: current.priority,
                reason: Some("No alternative enabled profile was available.".to_string()),
            }],
            blocked: true,
            blocked_reason: Some("No alternative enabled profile was available.".to_string()),
            reason: Some("Rotation could not find a different profile.".to_string()),
            provider_health: None,
            auth_profile: Some(current.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_chain_removes_empty_candidates() {
        let chain = normalize_chain(FallbackChainRecord {
            id: "chain".to_string(),
            name: "Chain".to_string(),
            enabled: true,
            ordered_candidates: vec![
                FallbackCandidate {
                    provider_id: "openai".to_string(),
                    auth_profile_id: Some("auth".to_string()),
                    priority: 1,
                    reason: None,
                },
                FallbackCandidate {
                    provider_id: " ".to_string(),
                    auth_profile_id: None,
                    priority: 2,
                    reason: None,
                },
            ],
            session_pin: None,
            notes: None,
            metadata: None,
        });
        assert_eq!(chain.ordered_candidates.len(), 1);
    }

    #[test]
    fn candidate_rank_prefers_lower_priority() {
        let provider_map = std::collections::BTreeMap::<String, ProviderHealthRecord>::new();
        let a = FallbackCandidate {
            provider_id: "a".to_string(),
            auth_profile_id: None,
            priority: 1,
            reason: None,
        };
        let b = FallbackCandidate {
            provider_id: "b".to_string(),
            auth_profile_id: None,
            priority: 10,
            reason: None,
        };
        assert!(candidate_rank(&a, &provider_map) < candidate_rank(&b, &provider_map));
    }
}
