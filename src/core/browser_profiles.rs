//! Browser profile control plane foundation.
//!
//! This module keeps profile state in the existing encrypted KV store so we can
//! add browser profile management without introducing schema churn.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::storage::Storage;

const BROWSER_PROFILES_KEY: &str = "browser:profiles:v1";
const MAX_RECENT_SESSIONS: usize = 20;
const PROFILE_RESOLVE_MIN_SCORE: f64 = 0.34;
const PROFILE_RESOLVE_AMBIGUITY_MARGIN: f64 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserProfileTargetKind {
    Sandbox,
    Host,
    RemoteCdp,
}

impl BrowserProfileTargetKind {
    fn default_value() -> Self {
        Self::Sandbox
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLoginState {
    Unknown,
    LoggedOut,
    LoggedIn,
    NeedsMfa,
    Expired,
    Error,
}

impl BrowserLoginState {
    fn default_value() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserProfileSummary {
    pub total: usize,
    pub sandbox: usize,
    pub host: usize,
    pub remote_cdp: usize,
    pub locked: usize,
    pub logged_in: usize,
    pub needs_attention: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfileLockInfo {
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub locked_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSessionHistoryEntry {
    pub id: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfileRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "BrowserProfileTargetKind::default_value")]
    pub target_kind: BrowserProfileTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_profile_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_workspace: Option<String>,
    #[serde(default = "BrowserLoginState::default_value")]
    pub login_state: BrowserLoginState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<BrowserProfileLockInfo>,
    #[serde(default)]
    pub recent_sessions: Vec<BrowserSessionHistoryEntry>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserProfileListResponse {
    pub summary: BrowserProfileSummary,
    pub profiles: Vec<BrowserProfileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfileResolveCandidate {
    pub profile: BrowserProfileRecord,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrowserProfileResolveOutcome {
    Resolved {
        profile: BrowserProfileRecord,
        candidates: Vec<BrowserProfileResolveCandidate>,
    },
    Ambiguous {
        candidates: Vec<BrowserProfileResolveCandidate>,
    },
    NotFound {
        candidates: Vec<BrowserProfileResolveCandidate>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserProfileUpsert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<BrowserProfileTargetKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_profile_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_state: Option<BrowserLoginState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_sessions: Option<Vec<BrowserSessionHistoryEntry>>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserProfileLockRequest {
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserProfileSessionRecord {
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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
        .with_context(|| format!("failed to decode browser profile payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode browser profile payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

fn sanitize_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sanitize_tags(tags: Vec<String>) -> Vec<String> {
    let mut result = tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    result
}

fn profile_match_normalized(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn profile_match_tokens(value: &str) -> Vec<String> {
    let mut tokens = profile_match_normalized(value)
        .split_whitespace()
        .map(str::to_string)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn profile_match_ngrams(value: &str) -> std::collections::BTreeSet<String> {
    let compact = profile_match_normalized(value).replace(' ', "");
    let chars = compact.chars().collect::<Vec<_>>();
    let mut out = std::collections::BTreeSet::new();
    if chars.is_empty() {
        return out;
    }
    if chars.len() <= 3 {
        out.insert(compact);
        return out;
    }
    for window in chars.windows(3) {
        out.insert(window.iter().collect::<String>());
    }
    out
}

fn profile_match_dice(left: &str, right: &str) -> f64 {
    let left = profile_match_ngrams(left);
    let right = profile_match_ngrams(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let overlap = left.intersection(&right).count() as f64;
    (2.0 * overlap) / (left.len() as f64 + right.len() as f64)
}

fn profile_match_token_score(query_tokens: &[String], field_tokens: &[String]) -> f64 {
    if query_tokens.is_empty() || field_tokens.is_empty() {
        return 0.0;
    }
    let query = query_tokens
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let field = field_tokens
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let overlap = query.intersection(&field).count() as f64;
    let coverage = overlap / query.len() as f64;
    let precision = overlap / field.len() as f64;
    (coverage * 0.72) + (precision * 0.28)
}

fn profile_metadata_match_text(metadata: &Option<serde_json::Value>) -> String {
    let Some(metadata) = metadata else {
        return String::new();
    };
    match metadata {
        serde_json::Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| {
                if key.to_ascii_lowercase().contains("secret") {
                    return None;
                }
                match value {
                    serde_json::Value::String(text) => Some(format!("{key} {text}")),
                    serde_json::Value::Bool(flag) => Some(format!("{key} {flag}")),
                    serde_json::Value::Number(number) => Some(format!("{key} {number}")),
                    _ => Some(key.clone()),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn profile_resolve_score(query: &str, profile: &BrowserProfileRecord) -> f64 {
    let query_norm = profile_match_normalized(query);
    if query_norm.is_empty() {
        return 0.0;
    }
    let query_tokens = profile_match_tokens(query);
    let id_norm = profile_match_normalized(&profile.id);
    let name_norm = profile_match_normalized(&profile.name);
    let target_kind = format!("{:?}", profile.target_kind);
    let fields = [
        profile.name.clone(),
        profile.description.clone().unwrap_or_default(),
        profile.tags.join(" "),
        profile.target_endpoint.clone().unwrap_or_default(),
        profile.target_profile_path.clone().unwrap_or_default(),
        profile.target_workspace.clone().unwrap_or_default(),
        target_kind,
        profile_metadata_match_text(&profile.metadata),
    ];
    let combined = fields.join(" ");
    let combined_norm = profile_match_normalized(&combined);
    let field_tokens = profile_match_tokens(&combined);

    let mut score = profile_match_token_score(&query_tokens, &field_tokens);
    score = score.max(profile_match_dice(&query_norm, &combined_norm) * 0.82);

    if !id_norm.is_empty() && id_norm == query_norm {
        score = score.max(1.0);
    }
    if !name_norm.is_empty() {
        if name_norm == query_norm {
            score = score.max(0.98);
        } else if query_norm.contains(&name_norm) || name_norm.contains(&query_norm) {
            score = score.max(0.86);
        } else {
            score = score.max(profile_match_dice(&query_norm, &name_norm) * 0.94);
        }
    }

    score.clamp(0.0, 1.0)
}

fn normalize_profile(mut profile: BrowserProfileRecord) -> BrowserProfileRecord {
    profile.name = profile.name.trim().to_string();
    profile.description = sanitize_text(profile.description);
    profile.target_endpoint = sanitize_text(profile.target_endpoint);
    profile.target_profile_path = sanitize_text(profile.target_profile_path);
    profile.target_workspace = sanitize_text(profile.target_workspace);
    profile.login_note = sanitize_text(profile.login_note);
    profile.last_error = sanitize_text(profile.last_error);
    profile.tags = sanitize_tags(profile.tags);
    profile.recent_sessions = normalize_sessions(profile.recent_sessions);
    profile
}

fn normalize_sessions(
    mut sessions: Vec<BrowserSessionHistoryEntry>,
) -> Vec<BrowserSessionHistoryEntry> {
    sessions.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    if sessions.len() > MAX_RECENT_SESSIONS {
        sessions.split_off(sessions.len() - MAX_RECENT_SESSIONS)
    } else {
        sessions
    }
}

fn build_summary(profiles: &[BrowserProfileRecord]) -> BrowserProfileSummary {
    BrowserProfileSummary {
        total: profiles.len(),
        sandbox: profiles
            .iter()
            .filter(|profile| matches!(profile.target_kind, BrowserProfileTargetKind::Sandbox))
            .count(),
        host: profiles
            .iter()
            .filter(|profile| matches!(profile.target_kind, BrowserProfileTargetKind::Host))
            .count(),
        remote_cdp: profiles
            .iter()
            .filter(|profile| matches!(profile.target_kind, BrowserProfileTargetKind::RemoteCdp))
            .count(),
        locked: profiles
            .iter()
            .filter(|profile| profile.lock.is_some())
            .count(),
        logged_in: profiles
            .iter()
            .filter(|profile| matches!(profile.login_state, BrowserLoginState::LoggedIn))
            .count(),
        needs_attention: profiles
            .iter()
            .filter(|profile| {
                matches!(
                    profile.login_state,
                    BrowserLoginState::LoggedOut
                        | BrowserLoginState::NeedsMfa
                        | BrowserLoginState::Expired
                        | BrowserLoginState::Error
                ) || profile.lock.is_some()
            })
            .count(),
    }
}

fn is_lock_expired(lock: &BrowserProfileLockInfo) -> bool {
    let Some(expires_at) = lock.expires_at.as_deref() else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(expires_at) {
        Ok(expires_at) => expires_at.with_timezone(&chrono::Utc) <= chrono::Utc::now(),
        Err(_) => false,
    }
}

fn lock_is_active(lock: &BrowserProfileLockInfo) -> bool {
    !is_lock_expired(lock)
}

fn prune_stale_lock(profile: &mut BrowserProfileRecord) {
    if profile
        .lock
        .as_ref()
        .is_some_and(|lock| !lock_is_active(lock))
    {
        profile.lock = None;
    }
}

fn ensure_owner(value: &str) -> Result<String> {
    let owner = value.trim();
    if owner.is_empty() {
        bail!("lock owner is required");
    }
    Ok(owner.to_string())
}

fn merge_upsert(
    resolved_id: String,
    existing: Option<BrowserProfileRecord>,
    input: BrowserProfileUpsert,
) -> BrowserProfileRecord {
    let now = now_rfc3339();
    let mut profile = existing.unwrap_or_else(|| BrowserProfileRecord {
        id: resolved_id.clone(),
        name: String::new(),
        description: None,
        target_kind: BrowserProfileTargetKind::Sandbox,
        target_endpoint: None,
        target_profile_path: None,
        target_workspace: None,
        login_state: BrowserLoginState::Unknown,
        login_checked_at: None,
        login_note: None,
        lock: None,
        recent_sessions: Vec::new(),
        tags: Vec::new(),
        enabled: true,
        last_used_at: None,
        last_error: None,
        metadata: None,
    });

    if let Some(name) = input.name {
        profile.name = name;
    }
    if let Some(description) = input.description {
        profile.description = Some(description);
    }
    if let Some(target_kind) = input.target_kind {
        profile.target_kind = target_kind;
    }
    if input.target_endpoint.is_some() {
        profile.target_endpoint = input.target_endpoint;
    }
    if input.target_profile_path.is_some() {
        profile.target_profile_path = input.target_profile_path;
    }
    if input.target_workspace.is_some() {
        profile.target_workspace = input.target_workspace;
    }
    if let Some(login_state) = input.login_state {
        profile.login_state = login_state;
    }
    if input.login_checked_at.is_some() {
        profile.login_checked_at = input.login_checked_at;
    }
    if input.login_note.is_some() {
        profile.login_note = input.login_note;
    }
    if input.recent_sessions.is_some() {
        profile.recent_sessions = input.recent_sessions.unwrap_or_default();
    }
    if !input.tags.is_empty() {
        profile.tags = input.tags;
    }
    if let Some(enabled) = input.enabled {
        profile.enabled = enabled;
    }
    if input.last_used_at.is_some() {
        profile.last_used_at = input.last_used_at;
    }
    if input.last_error.is_some() {
        profile.last_error = input.last_error;
    }
    if input.metadata.is_some() {
        profile.metadata = input.metadata;
    }

    profile.id = resolved_id;
    if profile.name.trim().is_empty() {
        profile.name = "Browser profile".to_string();
    }
    profile.last_used_at.get_or_insert(now);
    normalize_profile(profile)
}

async fn load_profiles(storage: &Storage) -> Result<Vec<BrowserProfileRecord>> {
    let profiles: Vec<BrowserProfileRecord> = load_json(storage, BROWSER_PROFILES_KEY).await?;
    Ok(profiles.into_iter().map(normalize_profile).collect())
}

async fn save_profiles(storage: &Storage, profiles: &[BrowserProfileRecord]) -> Result<()> {
    save_json(storage, BROWSER_PROFILES_KEY, profiles).await
}

async fn mutate_profile<F, T>(storage: &Storage, id: &str, mut mutate: F) -> Result<T>
where
    F: FnMut(&mut BrowserProfileRecord) -> Result<T>,
{
    let mut profiles = load_profiles(storage).await?;
    let Some(profile) = profiles.iter_mut().find(|profile| profile.id == id) else {
        bail!("browser profile not found");
    };

    prune_stale_lock(profile);
    let output = mutate(profile)?;
    save_profiles(storage, &profiles).await?;
    Ok(output)
}

pub struct BrowserProfileControlPlane;

impl BrowserProfileControlPlane {
    pub async fn list(storage: &Storage) -> Result<BrowserProfileListResponse> {
        let profiles = load_profiles(storage).await?;
        Ok(BrowserProfileListResponse {
            summary: build_summary(&profiles),
            profiles,
        })
    }

    pub async fn resolve(
        storage: &Storage,
        selector: &str,
    ) -> Result<BrowserProfileResolveOutcome> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Ok(BrowserProfileResolveOutcome::NotFound {
                candidates: Vec::new(),
            });
        }

        let mut candidates = load_profiles(storage)
            .await?
            .into_iter()
            .filter(|profile| profile.enabled)
            .map(|profile| BrowserProfileResolveCandidate {
                score: profile_resolve_score(selector, &profile),
                profile,
            })
            .filter(|candidate| candidate.score > 0.0)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.profile.name.cmp(&right.profile.name))
        });
        candidates.truncate(5);

        let Some(best) = candidates.first().cloned() else {
            return Ok(BrowserProfileResolveOutcome::NotFound { candidates });
        };
        if best.score < PROFILE_RESOLVE_MIN_SCORE {
            return Ok(BrowserProfileResolveOutcome::NotFound { candidates });
        }
        if candidates
            .get(1)
            .is_some_and(|next| best.score - next.score <= PROFILE_RESOLVE_AMBIGUITY_MARGIN)
        {
            return Ok(BrowserProfileResolveOutcome::Ambiguous { candidates });
        }
        Ok(BrowserProfileResolveOutcome::Resolved {
            profile: best.profile.clone(),
            candidates,
        })
    }

    pub async fn upsert(
        storage: &Storage,
        input: BrowserProfileUpsert,
    ) -> Result<BrowserProfileRecord> {
        let mut profiles = load_profiles(storage).await?;
        let id = input
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let existing = profiles.iter().position(|profile| profile.id == id);
        let profile = merge_upsert(id, existing.map(|idx| profiles[idx].clone()), input);

        if let Some(idx) = existing {
            profiles[idx] = profile.clone();
        } else {
            profiles.push(profile.clone());
        }

        save_profiles(storage, &profiles).await?;
        Ok(profile)
    }

    pub async fn delete(storage: &Storage, id: &str) -> Result<bool> {
        let mut profiles = load_profiles(storage).await?;
        let before = profiles.len();
        profiles.retain(|profile| profile.id != id);
        if before == profiles.len() {
            return Ok(false);
        }
        save_profiles(storage, &profiles).await?;
        Ok(true)
    }

    pub async fn lock(
        storage: &Storage,
        profile_id: &str,
        request: BrowserProfileLockRequest,
    ) -> Result<BrowserProfileRecord> {
        let owner = ensure_owner(&request.owner)?;
        mutate_profile(storage, profile_id, |profile| {
            if let Some(existing_lock) = profile.lock.as_ref() {
                if lock_is_active(existing_lock) && existing_lock.owner != owner {
                    bail!("browser profile is locked by another owner");
                }
            }

            profile.lock = Some(BrowserProfileLockInfo {
                owner: owner.clone(),
                reason: sanitize_text(request.reason.clone()),
                locked_at: now_rfc3339(),
                expires_at: request.expires_at.clone(),
            });
            profile.enabled = true;
            Ok(profile.clone())
        })
        .await
    }

    pub async fn unlock(
        storage: &Storage,
        profile_id: &str,
        owner: Option<&str>,
    ) -> Result<BrowserProfileRecord> {
        let owner = owner.map(ensure_owner).transpose()?;
        mutate_profile(storage, profile_id, |profile| {
            if let Some(existing_lock) = profile.lock.as_ref() {
                if let Some(owner) = owner.as_deref() {
                    if existing_lock.owner != owner {
                        bail!("browser profile is locked by another owner");
                    }
                }
            }
            profile.lock = None;
            Ok(profile.clone())
        })
        .await
    }

    pub async fn record_session(
        storage: &Storage,
        entry: BrowserProfileSessionRecord,
    ) -> Result<BrowserProfileRecord> {
        let profile_id = entry.profile_id.clone();
        mutate_profile(storage, &profile_id, |profile| {
            let started_at = entry.started_at.clone();
            let history_entry = BrowserSessionHistoryEntry {
                id: entry
                    .session_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                started_at: started_at.clone(),
                ended_at: entry.ended_at.clone(),
                duration_secs: entry.duration_secs,
                outcome: entry.outcome.clone(),
                title: entry.title.clone(),
                url: entry.url.clone(),
                channel: entry.channel.clone(),
                note: entry.note.clone(),
            };

            profile.recent_sessions.push(history_entry);
            profile.recent_sessions = normalize_sessions(profile.recent_sessions.clone());
            profile.last_used_at = Some(entry.ended_at.clone().unwrap_or(started_at));
            Ok(profile.clone())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> BrowserProfileRecord {
        BrowserProfileRecord {
            id: "profile-1".to_string(),
            name: "Default".to_string(),
            description: Some("Main browser profile".to_string()),
            target_kind: BrowserProfileTargetKind::Sandbox,
            target_endpoint: None,
            target_profile_path: Some("C:/profiles/default".to_string()),
            target_workspace: None,
            login_state: BrowserLoginState::LoggedOut,
            login_checked_at: None,
            login_note: None,
            lock: None,
            recent_sessions: Vec::new(),
            tags: vec!["primary".to_string(), "primary".to_string()],
            enabled: true,
            last_used_at: None,
            last_error: None,
            metadata: None,
        }
    }

    #[test]
    fn normalizes_profile_fields() {
        let profile = normalize_profile(sample_profile());
        assert_eq!(profile.tags, vec!["primary"]);
        assert_eq!(profile.description.as_deref(), Some("Main browser profile"));
    }

    #[test]
    fn prunes_recent_sessions() {
        let mut sessions = (0..25)
            .map(|idx| BrowserSessionHistoryEntry {
                id: idx.to_string(),
                started_at: format!("2026-03-20T{:02}:00:00Z", idx % 24),
                ended_at: None,
                duration_secs: None,
                outcome: "success".to_string(),
                title: None,
                url: None,
                channel: None,
                note: None,
            })
            .collect::<Vec<_>>();
        sessions.reverse();
        let pruned = normalize_sessions(sessions);
        assert_eq!(pruned.len(), MAX_RECENT_SESSIONS);
    }
}
