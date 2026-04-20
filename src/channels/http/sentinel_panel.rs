use super::*;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

const SENTINEL_SCAN_STATE_KEY: &str = "sentinel_scan_state_v1";
const SENTINEL_OBSERVATIONS_KEY: &str = "sentinel_observations_v1";
const SENTINEL_PROPOSALS_KEY: &str = "sentinel_proposals_v1";
const SENTINEL_DAILY_AUTO_RUNS_KEY: &str = "sentinel_daily_auto_runs_v1";
const BACKGROUND_LEARNING_STATE_KEY: &str = "sentinel_background_learning_state_v1";
const BACKGROUND_LEARNING_STATE_LEASE_KEY: &str = "sentinel_background_learning_state_lease_v1";
const BACKGROUND_LEARNING_STATE_LEASE_TTL_SECS: i64 = 15;
const SENTINEL_SCAN_COOLDOWN_SECS: i64 = 30 * 60;
const MAX_SENTINEL_OBSERVATIONS: usize = 120;
const MAX_SENTINEL_PROPOSALS: usize = 96;
const SENTINEL_RETENTION_DAYS: i64 = 30;
const SENTINEL_PROPOSAL_RECREATE_HOURS: i64 = 24;
const BACKGROUND_LEARNING_JOB_KEYS: [(&str, &str); 4] = [
    ("reflection_pass", "Reflection pass"),
    ("experience_consolidation", "Experience consolidation"),
    ("pattern_induction", "Pattern induction"),
    ("candidate_generation", "Candidate generation"),
];
static BACKGROUND_LEARNING_STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct SentinelScanState {
    #[serde(default)]
    pub last_started_at: Option<String>,
    #[serde(default)]
    pub last_completed_at: Option<String>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_trigger: Option<String>,
    #[serde(default)]
    pub last_created_observations: usize,
    #[serde(default)]
    pub last_created_proposals: usize,
    #[serde(default)]
    pub last_auto_executed: usize,
    #[serde(default)]
    pub open_proposals: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SentinelObservation {
    pub id: String,
    pub fingerprint: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_label: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub priority: u8,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SentinelProposal {
    pub id: String,
    pub fingerprint: String,
    pub proposal_kind: String,
    pub status: String,
    pub title: String,
    pub detail: String,
    pub rationale: String,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_label: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub priority: u8,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub snoozed_until: Option<String>,
    #[serde(default)]
    pub approved_at: Option<String>,
    #[serde(default)]
    pub dismissed_at: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub run_status: Option<String>,
    #[serde(default)]
    pub last_run_summary: Option<String>,
    #[serde(default)]
    pub action: Option<RecommendedAction>,
    #[serde(default)]
    pub chat_suggestion_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SentinelFeedStats {
    open_proposals: usize,
    completed_recently: usize,
    connected_services: usize,
    important_service_events: usize,
    recent_runs: usize,
    auto_mode_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SentinelDailyAutoRuns {
    day: String,
    count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BackgroundLearningJobRecord {
    pub key: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub last_started_at: Option<String>,
    #[serde(default)]
    pub last_completed_at: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub changed: bool,
    #[serde(default)]
    pub runs: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub stats: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BackgroundLearningStore {
    #[serde(default)]
    jobs: Vec<BackgroundLearningJobRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BackgroundLearningFeed {
    pub status: String,
    #[serde(default)]
    pub last_started_at: Option<String>,
    #[serde(default)]
    pub last_completed_at: Option<String>,
    pub summary: String,
    pub changed: bool,
    pub jobs: std::collections::BTreeMap<String, BackgroundLearningJobRecord>,
}

async fn load_scan_state(storage: &crate::storage::Storage) -> SentinelScanState {
    match storage.get(SENTINEL_SCAN_STATE_KEY).await {
        Ok(Some(raw)) => serde_json::from_slice::<SentinelScanState>(&raw).unwrap_or_default(),
        _ => SentinelScanState::default(),
    }
}

async fn save_scan_state(storage: &crate::storage::Storage, state: &SentinelScanState) {
    if let Ok(raw) = serde_json::to_vec(state) {
        let _ = storage.set(SENTINEL_SCAN_STATE_KEY, &raw).await;
    }
}

async fn load_observations(storage: &crate::storage::Storage) -> Vec<SentinelObservation> {
    match storage.get(SENTINEL_OBSERVATIONS_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<Vec<SentinelObservation>>(&raw).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn save_observations(
    storage: &crate::storage::Storage,
    observations: &[SentinelObservation],
) {
    if let Ok(raw) = serde_json::to_vec(observations) {
        let _ = storage.set(SENTINEL_OBSERVATIONS_KEY, &raw).await;
    }
}

async fn load_proposals(storage: &crate::storage::Storage) -> Vec<SentinelProposal> {
    match storage.get(SENTINEL_PROPOSALS_KEY).await {
        Ok(Some(raw)) => serde_json::from_slice::<Vec<SentinelProposal>>(&raw).unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn save_proposals(storage: &crate::storage::Storage, proposals: &[SentinelProposal]) {
    if let Ok(raw) = serde_json::to_vec(proposals) {
        let _ = storage.set(SENTINEL_PROPOSALS_KEY, &raw).await;
    }
}

async fn load_daily_auto_runs(storage: &crate::storage::Storage) -> SentinelDailyAutoRuns {
    match storage.get(SENTINEL_DAILY_AUTO_RUNS_KEY).await {
        Ok(Some(raw)) => serde_json::from_slice::<SentinelDailyAutoRuns>(&raw).unwrap_or_default(),
        _ => SentinelDailyAutoRuns::default(),
    }
}

async fn save_daily_auto_runs(storage: &crate::storage::Storage, counter: &SentinelDailyAutoRuns) {
    if let Ok(raw) = serde_json::to_vec(counter) {
        let _ = storage.set(SENTINEL_DAILY_AUTO_RUNS_KEY, &raw).await;
    }
}

fn background_learning_label(key: &str) -> String {
    match key {
        "reflection_pass" => "Reflection pass",
        "experience_consolidation" => "Experience consolidation",
        "pattern_induction" => "Pattern induction",
        "candidate_generation" => "Candidate generation",
        other => other,
    }
    .to_string()
}

fn default_background_learning_jobs() -> Vec<BackgroundLearningJobRecord> {
    BACKGROUND_LEARNING_JOB_KEYS
        .iter()
        .map(|(key, label)| BackgroundLearningJobRecord {
            key: (*key).to_string(),
            label: (*label).to_string(),
            status: "idle".to_string(),
            last_started_at: None,
            last_completed_at: None,
            summary: None,
            changed: false,
            runs: 0,
            last_error: None,
            stats: serde_json::json!({}),
        })
        .collect()
}

fn normalize_background_learning_jobs(
    jobs: Vec<BackgroundLearningJobRecord>,
) -> Vec<BackgroundLearningJobRecord> {
    let mut normalized = default_background_learning_jobs();
    for default_job in normalized.iter_mut() {
        if let Some(existing) = jobs.iter().find(|job| job.key == default_job.key) {
            *default_job = existing.clone();
            if default_job.label.trim().is_empty() {
                default_job.label = background_learning_label(&default_job.key);
            }
            continue;
        }
    }
    normalized
}

fn background_learning_summary_text(
    status: &str,
    changed: bool,
    changed_jobs: &[&BackgroundLearningJobRecord],
) -> String {
    match status {
        "disabled" => "Background learning is disabled.".to_string(),
        "paused" => "Background learning is paused.".to_string(),
        "running" => "Background learning is running.".to_string(),
        "failed" => "Background learning reported an error.".to_string(),
        _ if changed => {
            let labels = changed_jobs
                .iter()
                .take(2)
                .map(|job| job.label.clone())
                .collect::<Vec<_>>();
            if labels.is_empty() {
                "Background learning updated recent memory and patterns.".to_string()
            } else {
                format!("Background learning updated {}.", labels.join(", "))
            }
        }
        _ => "Background learning reviewed recent activity and found no new changes.".to_string(),
    }
}

async fn load_background_learning_store(
    storage: &crate::storage::Storage,
) -> BackgroundLearningStore {
    match storage.get(BACKGROUND_LEARNING_STATE_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<BackgroundLearningStore>(&raw).unwrap_or_default()
        }
        _ => BackgroundLearningStore::default(),
    }
}

async fn save_background_learning_store(
    storage: &crate::storage::Storage,
    store: &BackgroundLearningStore,
) {
    if let Ok(raw) = serde_json::to_vec(store) {
        if let Err(error) = storage.set(BACKGROUND_LEARNING_STATE_KEY, &raw).await {
            tracing::warn!("Failed to save background learning store: {}", error);
        }
    }
}

pub(crate) async fn load_background_learning_feed(
    storage: &crate::storage::Storage,
    settings: &AutonomySettings,
) -> BackgroundLearningFeed {
    let store = load_background_learning_store(storage).await;
    let learning_enabled = crate::core::learning::load_learning_enabled(storage).await;
    build_background_learning_feed(settings, learning_enabled, &store.jobs)
}

pub(crate) struct BackgroundLearningJobUpdate {
    pub key: String,
    pub status: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub summary: String,
    pub changed: bool,
    pub stats: serde_json::Value,
}

pub(crate) async fn record_background_learning_job_result(
    storage: &crate::storage::Storage,
    update: &BackgroundLearningJobUpdate,
) {
    let _guard = BACKGROUND_LEARNING_STORE_LOCK.lock().await;
    let lease_owner = uuid::Uuid::new_v4().to_string();
    let mut lease_acquired = false;
    for _ in 0..20 {
        match storage
            .acquire_kv_lease(
                BACKGROUND_LEARNING_STATE_LEASE_KEY,
                &lease_owner,
                BACKGROUND_LEARNING_STATE_LEASE_TTL_SECS,
            )
            .await
        {
            Ok(true) => {
                lease_acquired = true;
                break;
            }
            Ok(false) => {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to acquire background learning state lease: {}",
                    error
                );
                return;
            }
        }
    }
    if !lease_acquired {
        tracing::warn!("Timed out waiting for background learning state lease");
        return;
    }

    let mut store = load_background_learning_store(storage).await;
    let label = background_learning_label(&update.key);
    let mut jobs = normalize_background_learning_jobs(store.jobs);
    let last_error = if update.status.eq_ignore_ascii_case("failed") {
        update
            .stats
            .get("error")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| Some(update.summary.clone()))
    } else {
        None
    };
    if let Some(job) = jobs.iter_mut().find(|job| job.key == update.key) {
        job.label = label;
        job.status = update.status.clone();
        job.last_started_at = update.started_at.clone();
        job.last_completed_at = update.completed_at.clone();
        job.summary = Some(update.summary.clone());
        job.changed = update.changed;
        job.runs = job.runs.saturating_add(1);
        job.last_error = last_error;
        job.stats = update.stats.clone();
    } else {
        jobs.push(BackgroundLearningJobRecord {
            key: update.key.clone(),
            label,
            status: update.status.clone(),
            last_started_at: update.started_at.clone(),
            last_completed_at: update.completed_at.clone(),
            summary: Some(update.summary.clone()),
            changed: update.changed,
            runs: 1,
            last_error,
            stats: update.stats.clone(),
        });
    }
    store.jobs = jobs;
    save_background_learning_store(storage, &store).await;
    if let Err(error) = storage
        .release_kv_lease(BACKGROUND_LEARNING_STATE_LEASE_KEY, &lease_owner)
        .await
    {
        tracing::warn!(
            "Failed to release background learning state lease: {}",
            error
        );
    }
}

fn build_background_learning_feed(
    settings: &AutonomySettings,
    learning_enabled: bool,
    jobs: &[BackgroundLearningJobRecord],
) -> BackgroundLearningFeed {
    let jobs = normalize_background_learning_jobs(jobs.to_vec());
    let effective_status = if !learning_enabled {
        "disabled"
    } else if settings.autonomy_mode.eq_ignore_ascii_case("off") {
        "disabled"
    } else if settings.agent_paused {
        "paused"
    } else if jobs.iter().any(|job| job.status == "running") {
        "running"
    } else if jobs.iter().any(|job| job.status == "failed") {
        "failed"
    } else if jobs.iter().any(|job| job.changed) {
        "completed"
    } else {
        "idle"
    };

    let mut started_at = None::<String>;
    let mut completed_at = None::<String>;
    let mut changed_jobs = Vec::new();
    let mut changed = false;
    for job in &jobs {
        if let Some(value) = job.last_started_at.as_ref() {
            if started_at
                .as_deref()
                .and_then(|current| parse_optional_utc(Some(current)))
                .map(|current| {
                    parse_optional_utc(Some(value))
                        .map(|next| next > current)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            {
                started_at = Some(value.clone());
            }
        }
        if let Some(value) = job.last_completed_at.as_ref() {
            if completed_at
                .as_deref()
                .and_then(|current| parse_optional_utc(Some(current)))
                .map(|current| {
                    parse_optional_utc(Some(value))
                        .map(|next| next > current)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            {
                completed_at = Some(value.clone());
            }
        }
        if job.changed {
            changed = true;
            changed_jobs.push(job);
        }
    }

    let summary = background_learning_summary_text(effective_status, changed, &changed_jobs);
    let mut response_jobs = std::collections::BTreeMap::new();
    for job in jobs {
        response_jobs.insert(job.key.clone(), job);
    }

    BackgroundLearningFeed {
        status: effective_status.to_string(),
        last_started_at: started_at,
        last_completed_at: completed_at,
        summary,
        changed,
        jobs: response_jobs,
    }
}

fn normalize_fingerprint(parts: &[&str]) -> String {
    let mut out = String::new();
    for ch in parts
        .iter()
        .map(|part| part.trim().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(":")
        .chars()
    {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn parse_optional_utc(value: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    value.and_then(super::parse_rfc3339_utc)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn is_quiet_hours_active(settings: &AutonomySettings) -> bool {
    let Some(start_raw) = settings.quiet_hours_start.as_deref() else {
        return false;
    };
    let Some(end_raw) = settings.quiet_hours_end.as_deref() else {
        return false;
    };
    let Ok(start) = chrono::NaiveTime::parse_from_str(start_raw.trim(), "%H:%M") else {
        return false;
    };
    let Ok(end) = chrono::NaiveTime::parse_from_str(end_raw.trim(), "%H:%M") else {
        return false;
    };
    let now = chrono::Local::now().time();
    if start == end {
        return false;
    }
    if start < end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

fn should_run_scan(
    state: &SentinelScanState,
    _trigger: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    match parse_optional_utc(state.last_completed_at.as_deref()) {
        Some(last) => (now - last).num_seconds() >= SENTINEL_SCAN_COOLDOWN_SECS,
        None => true,
    }
}

fn retention_cutoff(now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    now - chrono::Duration::days(SENTINEL_RETENTION_DAYS)
}

fn recent_proposal_blocks_recreation(
    proposal: &SentinelProposal,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if matches!(
        proposal.status.as_str(),
        "open" | "running" | "queued_for_approval"
    ) {
        return true;
    }

    if proposal.status == "snoozed" {
        return parse_optional_utc(proposal.snoozed_until.as_deref())
            .map(|until| until > now)
            .unwrap_or(false);
    }

    parse_optional_utc(Some(&proposal.updated_at))
        .map(|updated| (now - updated).num_hours() < SENTINEL_PROPOSAL_RECREATE_HOURS)
        .unwrap_or(false)
}

fn refresh_snoozed_proposals(
    proposals: &mut [SentinelProposal],
    now: chrono::DateTime<chrono::Utc>,
) {
    for proposal in proposals.iter_mut() {
        if proposal.status != "snoozed" {
            continue;
        }
        let expired = parse_optional_utc(proposal.snoozed_until.as_deref())
            .map(|until| until <= now)
            .unwrap_or(true);
        if expired {
            proposal.status = "open".to_string();
            proposal.snoozed_until = None;
            proposal.updated_at = now.to_rfc3339();
        }
    }
}

fn prune_observations(
    observations: Vec<SentinelObservation>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<SentinelObservation> {
    let cutoff = retention_cutoff(now);
    let mut retained = observations
        .into_iter()
        .filter(|item| {
            parse_optional_utc(Some(&item.updated_at))
                .map(|updated| updated >= cutoff)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    retained.sort_by(|a, b| {
        parse_optional_utc(Some(&b.updated_at))
            .cmp(&parse_optional_utc(Some(&a.updated_at)))
            .then_with(|| a.id.cmp(&b.id))
    });
    if retained.len() > MAX_SENTINEL_OBSERVATIONS {
        retained.truncate(MAX_SENTINEL_OBSERVATIONS);
    }
    retained
}

fn prune_proposals(
    proposals: Vec<SentinelProposal>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<SentinelProposal> {
    let cutoff = retention_cutoff(now);
    let mut retained = proposals
        .into_iter()
        .filter(|item| {
            parse_optional_utc(Some(&item.updated_at))
                .map(|updated| updated >= cutoff)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    retained.sort_by(|a, b| {
        let rank = |status: &str| match status {
            "open" | "running" | "queued_for_approval" => 0u8,
            "snoozed" => 1u8,
            "failed" => 2u8,
            "completed" => 3u8,
            _ => 4u8,
        };
        rank(&a.status)
            .cmp(&rank(&b.status))
            .then_with(|| {
                parse_optional_utc(Some(&b.updated_at))
                    .cmp(&parse_optional_utc(Some(&a.updated_at)))
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    if retained.len() > MAX_SENTINEL_PROPOSALS {
        retained.truncate(MAX_SENTINEL_PROPOSALS);
    }
    retained
}

fn open_proposal_count(proposals: &[SentinelProposal]) -> usize {
    proposals
        .iter()
        .filter(|proposal| {
            matches!(
                proposal.status.as_str(),
                "open" | "running" | "queued_for_approval"
            )
        })
        .count()
}

fn sentinel_channel_for_action(action_kind: &str) -> bool {
    matches!(action_kind, "chat_prompt" | "create_task")
}

fn decorate_action_for_sentinel(
    action: &RecommendedAction,
    proposal_id: &str,
    source_kind: &str,
    source_id: Option<&str>,
) -> RecommendedAction {
    let mut next = action.clone();
    let mut payload_map = match next.payload.clone() {
        serde_json::Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), other);
            map
        }
    };
    payload_map.insert(
        "_sentinel_origin".to_string(),
        serde_json::json!({
            "origin_type": "sentinel",
            "proposal_id": proposal_id,
            "source_kind": source_kind,
            "source_id": source_id,
        }),
    );
    if sentinel_channel_for_action(&next.action_kind)
        && !payload_map
            .get("channel")
            .and_then(|value| value.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    {
        payload_map.insert(
            "channel".to_string(),
            serde_json::Value::String("sentinel".to_string()),
        );
    }
    next.payload = serde_json::Value::Object(payload_map);
    next
}

fn build_integration_prompt(
    item: &crate::core::integration_sync::IntegrationSyncFeedItem,
) -> String {
    format!(
        "An important connected-service event was detected.\n\
Integration: {}\n\
Kind: {}\n\
Title: {}\n\
Summary: {}\n\
URL: {}\n\
Detected at: {}\n\n\
Take the safest concrete next action now. Prefer summarizing, drafting a reply, creating a follow-up task, or setting a watcher if that is the correct move. Do not only explain what could be done.",
        item.integration_name.trim(),
        item.kind.trim(),
        item.title.trim(),
        item.summary.trim(),
        item.url.clone().unwrap_or_default(),
        item.detected_at.trim(),
    )
}

async fn build_candidates(
    integration_ctx: &crate::core::integration_sync::IntegrationSyncContext,
    settings: &AutonomySettings,
) -> (
    Vec<SentinelObservation>,
    Vec<SentinelProposal>,
    usize,
    usize,
) {
    let now = now_rfc3339();
    let mut observations = Vec::new();
    let mut proposals = Vec::new();

    let mut connected_services = 0usize;
    let mut important_service_events = 0usize;
    if settings.sentinel.watch_connected_services {
        let statuses = crate::core::integration_sync::list_statuses(integration_ctx).await;
        connected_services = statuses
            .iter()
            .filter(|status| status.connected && status.supported)
            .count();
        let feed_items =
            crate::core::integration_sync::list_feed_items(integration_ctx, None, 18).await;
        for item in feed_items.into_iter().filter(|entry| {
            entry.important || entry.importance >= settings.sentinel.confidence_threshold
        }) {
            important_service_events += 1;
            let fingerprint = normalize_fingerprint(&[
                "integration_feed",
                item.integration_id.as_str(),
                item.id.as_str(),
            ]);
            observations.push(SentinelObservation {
                id: String::new(),
                fingerprint: fingerprint.clone(),
                kind: "integration_signal".to_string(),
                title: format!("{}: {}", item.integration_name, item.title),
                detail: item.summary.clone(),
                source_kind: "integration".to_string(),
                source_id: Some(item.id.clone()),
                source_label: Some(item.integration_name.clone()),
                confidence: item.importance,
                priority: if item.important { 5 } else { 3 },
                created_at: now.clone(),
                updated_at: now.clone(),
                metadata: serde_json::json!({
                    "integration_id": item.integration_id,
                    "kind": item.kind,
                    "url": item.url,
                    "occurred_at": item.occurred_at,
                    "detected_at": item.detected_at,
                }),
            });

            if item.importance >= settings.sentinel.confidence_threshold {
                let action = super::recommendation(
                    &format!("Triage {}", item.integration_name),
                    &format!(
                        "Review the latest important {} event and take the safest concrete next action.",
                        item.integration_name
                    ),
                    "chat_prompt",
                    serde_json::json!({
                        "prompt": build_integration_prompt(&item),
                    }),
                    &settings.trust_policy,
                );
                let proposal_id = uuid::Uuid::new_v4().to_string();
                proposals.push(SentinelProposal {
                    id: proposal_id.clone(),
                    fingerprint: normalize_fingerprint(&[
                        "proposal",
                        "integration_feed",
                        item.integration_id.as_str(),
                        item.id.as_str(),
                    ]),
                    proposal_kind: "recommended_action".to_string(),
                    status: "open".to_string(),
                    title: action.title.clone(),
                    detail: item.summary.clone(),
                    rationale: format!(
                        "{} flagged a new high-signal external event: {}",
                        item.integration_name, item.title
                    ),
                    source_kind: "integration".to_string(),
                    source_id: Some(item.id.clone()),
                    source_label: Some(item.integration_name.clone()),
                    confidence: item.importance,
                    priority: if item.important { 5 } else { 3 },
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    snoozed_until: None,
                    approved_at: None,
                    dismissed_at: None,
                    trace_id: None,
                    run_status: None,
                    last_run_summary: None,
                    action: Some(decorate_action_for_sentinel(
                        &action,
                        &proposal_id,
                        "integration",
                        Some(&item.id),
                    )),
                    chat_suggestion_id: None,
                });
            }
        }
    }

    observations.sort_by(|a, b| {
        b.priority.cmp(&a.priority).then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    proposals.sort_by(|a, b| {
        b.priority.cmp(&a.priority).then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    proposals.truncate(settings.sentinel.max_proposals_per_scan.max(1) as usize);
    (
        observations,
        proposals,
        connected_services,
        important_service_events,
    )
}

async fn persist_sentinel_trace_from_agent(
    agent: &Agent,
    proposal: &SentinelProposal,
    action: &RecommendedAction,
    status: &str,
    summary: &str,
    detail_payload: serde_json::Value,
) -> Option<String> {
    let started_at = chrono::Utc::now();
    let trace_id = uuid::Uuid::new_v4().to_string();
    let status_normalized = status.trim().to_ascii_lowercase();
    let (step_type, icon, title) = if status_normalized == "error" {
        ("error", "[err]", "Sentinel Proposal Failed")
    } else if status_normalized == "queued_for_approval" {
        ("warning", "[wait]", "Sentinel Proposal Queued")
    } else {
        ("success", "[ok]", "Sentinel Proposal Completed")
    };
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
        id: trace_id.clone(),
        message: format!("Sentinel proposal: {}", proposal.title),
        channel: "sentinel".to_string(),
        started_at: Some(started_at),
        completed_at: Some(started_at),
        steps: vec![
            crate::core::ExecutionStep {
                icon: "[sentinel]".to_string(),
                title: "Sentinel Proposal Approved".to_string(),
                detail: format!(
                    "{} ({})",
                    proposal.title.trim(),
                    proposal.proposal_kind.trim()
                ),
                step_type: "info".to_string(),
                data: serde_json::to_string_pretty(&serde_json::json!({
                    "trace_kind": "sentinel.proposal.request",
                    "proposal_id": proposal.id,
                    "proposal_kind": proposal.proposal_kind,
                    "source_kind": proposal.source_kind,
                    "source_id": proposal.source_id,
                    "action": action,
                }))
                .ok(),
                timestamp: started_at,
                duration_ms: Some(0),
            },
            crate::core::ExecutionStep {
                icon: icon.to_string(),
                title: title.to_string(),
                detail: summary.to_string(),
                step_type: step_type.to_string(),
                data: serde_json::to_string_pretty(&detail_payload).ok(),
                timestamp: started_at,
                duration_ms: Some(0),
            },
        ],
        proof_id: None,
        response: Some(summary.to_string()),
        model: Some("internal:sentinel".to_string()),
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost_usd: 0.0,
        complexity: Some("sentinel".to_string()),
        plan: None,
    }));
    agent.persist_completed_trace(&trace_ref).await;
    Some(trace_id)
}

async fn execute_action_proposal(
    agent: &Agent,
    settings: &mut AutonomySettings,
    proposal: &SentinelProposal,
) -> std::result::Result<(String, Option<String>, String), String> {
    let action = proposal
        .action
        .clone()
        .ok_or_else(|| "Proposal is missing an action payload".to_string())?;
    let result = super::run_recommended_action(agent, settings, &action, false).await?;
    let summary = super::summarize_autonomy_action_result(&action, &result);
    let result_status = result
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("executed")
        .to_string();
    let trace_id = if let Some(existing) = result
        .get("trace_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
    {
        Some(existing)
    } else {
        persist_sentinel_trace_from_agent(
            agent,
            proposal,
            &action,
            &result_status,
            &summary,
            serde_json::json!({
                "proposal": proposal,
                "action": action.clone(),
                "result": result,
            }),
        )
        .await
    };
    Ok((summary, trace_id, result_status))
}

async fn try_auto_execute_new_proposals(
    agent: &Agent,
    settings: &mut AutonomySettings,
    storage: &crate::storage::Storage,
    proposals: &mut [SentinelProposal],
    new_ids: &[String],
) -> usize {
    if !settings.autonomy_mode.eq_ignore_ascii_case("auto") {
        return 0;
    }
    if is_quiet_hours_active(settings) {
        return 0;
    }
    let mut counter = load_daily_auto_runs(storage).await;
    let today = chrono::Local::now().date_naive().to_string();
    if counter.day != today {
        counter.day = today;
        counter.count = 0;
    }
    let remaining = settings
        .daily_run_limit
        .unwrap_or(40)
        .saturating_sub(counter.count);
    if remaining == 0 {
        save_daily_auto_runs(storage, &counter).await;
        return 0;
    }

    let mut executed = 0usize;
    for proposal_id in new_ids {
        if executed as u32 >= remaining {
            break;
        }
        let Some(proposal) = proposals.iter_mut().find(|item| item.id == *proposal_id) else {
            continue;
        };
        if proposal.proposal_kind != "recommended_action" || proposal.status != "open" {
            continue;
        }
        let Some(action) = proposal.action.clone() else {
            continue;
        };
        let risk = score_action_risk(&action.action_kind, &action.payload, &settings.trust_policy);
        if risk.requires_approval || risk.score > settings.trust_policy.auto_execute_max_score {
            continue;
        }

        proposal.status = "running".to_string();
        proposal.updated_at = now_rfc3339();
        match execute_action_proposal(agent, settings, proposal).await {
            Ok((summary, trace_id, run_status)) => {
                proposal.status = if run_status == "queued_for_approval" {
                    "queued_for_approval".to_string()
                } else {
                    "completed".to_string()
                };
                proposal.approved_at = Some(now_rfc3339());
                proposal.updated_at = now_rfc3339();
                proposal.trace_id = trace_id;
                proposal.run_status = Some(run_status);
                proposal.last_run_summary = Some(summary);
                executed += 1;
                counter.count = counter.count.saturating_add(1);
            }
            Err(error) => {
                proposal.status = "failed".to_string();
                proposal.updated_at = now_rfc3339();
                proposal.run_status = Some("failed".to_string());
                proposal.last_run_summary = Some(error);
            }
        }
    }

    save_daily_auto_runs(storage, &counter).await;
    executed
}

pub(crate) async fn run_sentinel_scan_tick(
    shared: Arc<RwLock<Agent>>,
    trigger: &str,
) -> serde_json::Value {
    let now = chrono::Utc::now();
    let (storage, integration_ctx) = {
        let agent = shared.read().await;
        (
            agent.storage.clone(),
            crate::core::integration_sync::context_from_agent(&agent, None),
        )
    };
    let mut settings = super::load_autonomy_settings_from_storage(&storage).await;

    if settings.agent_paused
        || settings.autonomy_mode.eq_ignore_ascii_case("off")
        || !settings.sentinel.enabled
    {
        return serde_json::json!({
            "status": "disabled",
            "trigger": trigger,
            "generated_at": now.to_rfc3339(),
        });
    }

    let mut scan_state = load_scan_state(&storage).await;
    if !should_run_scan(&scan_state, trigger, now) {
        return serde_json::json!({
            "status": "ok",
            "trigger": trigger,
            "generated_at": now.to_rfc3339(),
            "skipped": true,
            "reason": "cooldown",
        });
    }

    scan_state.last_started_at = Some(now.to_rfc3339());
    scan_state.last_trigger = Some(trigger.to_string());
    scan_state.last_status = Some("running".to_string());
    scan_state.last_error = None;
    save_scan_state(&storage, &scan_state).await;

    let mut observations = load_observations(&storage).await;
    let mut proposals = load_proposals(&storage).await;
    observations.retain(|item| item.kind != "chat_suggestion");
    proposals.retain(|item| {
        item.proposal_kind != "chat_suggestion_accept" && item.chat_suggestion_id.is_none()
    });
    refresh_snoozed_proposals(&mut proposals, now);

    let (candidate_observations, candidate_proposals, connected_services, important_service_events) =
        build_candidates(&integration_ctx, &settings).await;
    let mut created_observations = 0usize;
    let mut created_proposals = 0usize;
    let mut new_proposal_ids = Vec::new();

    for candidate in candidate_observations {
        if let Some(existing) = observations
            .iter_mut()
            .find(|item| item.fingerprint == candidate.fingerprint)
        {
            existing.kind = candidate.kind;
            existing.title = candidate.title;
            existing.detail = candidate.detail;
            existing.source_kind = candidate.source_kind;
            existing.source_id = candidate.source_id;
            existing.source_label = candidate.source_label;
            existing.confidence = candidate.confidence;
            existing.priority = candidate.priority;
            existing.updated_at = now.to_rfc3339();
            existing.metadata = candidate.metadata;
        } else {
            observations.push(SentinelObservation {
                id: uuid::Uuid::new_v4().to_string(),
                ..candidate
            });
            created_observations += 1;
        }
    }

    for candidate in candidate_proposals {
        if let Some(existing) = proposals
            .iter_mut()
            .find(|item| item.fingerprint == candidate.fingerprint)
        {
            if recent_proposal_blocks_recreation(existing, now) {
                existing.title = candidate.title;
                existing.detail = candidate.detail;
                existing.rationale = candidate.rationale;
                existing.source_kind = candidate.source_kind;
                existing.source_id = candidate.source_id;
                existing.source_label = candidate.source_label;
                existing.confidence = candidate.confidence;
                existing.priority = candidate.priority;
                existing.action = candidate.action;
                existing.chat_suggestion_id = candidate.chat_suggestion_id;
                existing.updated_at = now.to_rfc3339();
                continue;
            }
        }
        new_proposal_ids.push(candidate.id.clone());
        proposals.push(candidate);
        created_proposals += 1;
    }

    let mut agent_snapshot: Option<Agent> = None;
    let auto_executed = if new_proposal_ids.is_empty() {
        0
    } else {
        if agent_snapshot.is_none() {
            agent_snapshot = Some(Agent::snapshot(&shared).await);
        }
        try_auto_execute_new_proposals(
            agent_snapshot.as_ref().expect("agent snapshot"),
            &mut settings,
            &storage,
            &mut proposals,
            &new_proposal_ids,
        )
        .await
    };
    let _ = super::save_autonomy_settings_to_storage(&storage, &settings).await;

    observations = prune_observations(observations, now);
    proposals = prune_proposals(proposals, now);
    save_observations(&storage, &observations).await;
    save_proposals(&storage, &proposals).await;

    scan_state.last_completed_at = Some(now_rfc3339());
    scan_state.last_status = Some("completed".to_string());
    scan_state.last_created_observations = created_observations;
    scan_state.last_created_proposals = created_proposals;
    scan_state.last_auto_executed = auto_executed;
    scan_state.open_proposals = open_proposal_count(&proposals);
    save_scan_state(&storage, &scan_state).await;

    if created_proposals > 0 {
        let body = format!(
            "Sentinel prepared {} new proposal{} from recent signals.",
            created_proposals,
            if created_proposals == 1 { "" } else { "s" }
        );
        if agent_snapshot.is_none() {
            agent_snapshot = Some(Agent::snapshot(&shared).await);
        }
        agent_snapshot
            .as_ref()
            .expect("agent snapshot")
            .emit_notification("Sentinel queued new work", &body, "info", "sentinel")
            .await;
    }

    serde_json::json!({
        "status": "ok",
        "trigger": trigger,
        "generated_at": now.to_rfc3339(),
        "created_observations": created_observations,
        "created_proposals": created_proposals,
        "auto_executed": auto_executed,
        "connected_services": connected_services,
        "important_service_events": important_service_events,
        "open_proposals": scan_state.open_proposals,
    })
}

pub(super) async fn get_sentinel_settings(State(state): State<AppState>) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    let settings = super::load_autonomy_settings_from_storage(&storage).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "settings": settings.sentinel,
            "autonomy_mode": settings.autonomy_mode,
            "daily_run_limit": settings.daily_run_limit,
            "quiet_hours_start": settings.quiet_hours_start,
            "quiet_hours_end": settings.quiet_hours_end,
            "agent_paused": settings.agent_paused,
        })),
    )
        .into_response()
}

pub(super) async fn update_sentinel_settings(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    let mut settings = super::load_autonomy_settings_from_storage(&storage).await;

    if let Some(enabled) = request.get("enabled").and_then(|value| value.as_bool()) {
        settings.sentinel.enabled = enabled;
    }
    if let Some(watch_in_app) = request
        .get("watch_in_app")
        .and_then(|value| value.as_bool())
    {
        settings.sentinel.watch_in_app = watch_in_app;
    }
    if let Some(watch_connected_services) = request
        .get("watch_connected_services")
        .and_then(|value| value.as_bool())
    {
        settings.sentinel.watch_connected_services = watch_connected_services;
    }
    if let Some(infer_new_automations) = request
        .get("infer_new_automations")
        .and_then(|value| value.as_bool())
    {
        settings.sentinel.infer_new_automations = infer_new_automations;
    }
    if let Some(confidence_threshold) = request
        .get("confidence_threshold")
        .and_then(|value| value.as_f64())
    {
        settings.sentinel.confidence_threshold = confidence_threshold.clamp(0.1, 1.0) as f32;
    }
    if let Some(max_proposals) = request
        .get("max_proposals_per_scan")
        .and_then(|value| value.as_u64())
    {
        settings.sentinel.max_proposals_per_scan = max_proposals.clamp(1, 20) as u32;
    }
    if let Some(mode) = request
        .get("autonomy_mode")
        .and_then(|value| value.as_str())
    {
        let normalized = mode.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "off" | "assist" | "auto") {
            settings.autonomy_mode = normalized;
        }
    }

    match super::save_autonomy_settings_to_storage(&storage, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "settings": settings.sentinel,
                "autonomy_mode": settings.autonomy_mode,
                "daily_run_limit": settings.daily_run_limit,
                "quiet_hours_start": settings.quiet_hours_start,
                "quiet_hours_end": settings.quiet_hours_end,
                "agent_paused": settings.agent_paused,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error }),
        )
            .into_response(),
    }
}

pub(super) async fn get_sentinel_feed(State(state): State<AppState>) -> Response {
    let (storage, ctx, trace_history) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            crate::core::integration_sync::context_from_agent(&agent, None),
            agent.trace_history.clone(),
        )
    };
    let settings = super::load_autonomy_settings_from_storage(&storage).await;
    let mut observations = load_observations(&storage).await;
    let mut proposals = load_proposals(&storage).await;
    let scan = load_scan_state(&storage).await;
    let statuses = crate::core::integration_sync::list_statuses(&ctx).await;
    let feed_items = crate::core::integration_sync::list_feed_items(&ctx, None, 18).await;
    let recent_runs = trace_history
        .read()
        .await
        .iter()
        .take(40)
        .filter(|trace| {
            let channel = trace.channel.to_ascii_lowercase();
            channel == "sentinel" || channel == "autonomy"
        })
        .count();

    let now = chrono::Utc::now();
    refresh_snoozed_proposals(&mut proposals, now);
    observations = prune_observations(observations, now);
    proposals = prune_proposals(proposals, now);
    save_observations(&storage, &observations).await;
    save_proposals(&storage, &proposals).await;

    let stats = SentinelFeedStats {
        open_proposals: open_proposal_count(&proposals),
        completed_recently: proposals
            .iter()
            .filter(|proposal| proposal.status == "completed")
            .take(12)
            .count(),
        connected_services: statuses
            .iter()
            .filter(|status| status.connected && status.supported)
            .count(),
        important_service_events: feed_items.iter().filter(|item| item.important).count(),
        recent_runs,
        auto_mode_enabled: settings.autonomy_mode.eq_ignore_ascii_case("auto"),
    };
    let background_learning = load_background_learning_feed(&storage, &settings).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "scan": scan,
            "background_learning": background_learning,
            "observations": observations,
            "proposals": proposals,
            "stats": stats,
        })),
    )
        .into_response()
}

pub(super) async fn approve_sentinel_proposal(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let proposal_id = id.trim();
    if proposal_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Proposal id is required".to_string(),
            }),
        )
            .into_response();
    }

    let storage = { state.agent.read().await.storage.clone() };
    let mut proposals = load_proposals(&storage).await;
    let Some(index) = proposals
        .iter()
        .position(|proposal| proposal.id == proposal_id)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Proposal not found".to_string(),
            }),
        )
            .into_response();
    };

    if !matches!(
        proposals[index].status.as_str(),
        "open" | "snoozed" | "failed"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Proposal is not in a runnable state".to_string(),
            }),
        )
            .into_response();
    }

    proposals[index].status = "running".to_string();
    proposals[index].updated_at = now_rfc3339();
    proposals[index].snoozed_until = None;
    let proposal = proposals[index].clone();
    save_proposals(&storage, &proposals).await;

    let result = if proposal.proposal_kind == "chat_suggestion_accept" {
        if let Some(suggestion_id) = proposal.chat_suggestion_id.clone() {
            super::accept_chat_suggestion(&state, &suggestion_id)
                .await
                .map(|payload| {
                    let trace_id = payload
                        .get("trace_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string());
                    let summary = payload
                        .get("run")
                        .and_then(|value| value.get("summary"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("Launched suggestion execution.")
                        .to_string();
                    (summary, trace_id, "running".to_string())
                })
        } else {
            Err("Proposal is missing the chat suggestion id".to_string())
        }
    } else {
        let agent = Agent::snapshot(&state.agent).await;
        let mut settings = super::load_autonomy_settings_from_storage(&storage).await;
        let outcome = execute_action_proposal(&agent, &mut settings, &proposal).await;
        let _ = super::save_autonomy_settings_to_storage(&storage, &settings).await;
        outcome
    };

    let mut proposals = load_proposals(&storage).await;
    if let Some(current) = proposals.iter_mut().find(|item| item.id == proposal.id) {
        match result {
            Ok((summary, trace_id, run_status)) => {
                current.status = if run_status == "queued_for_approval" {
                    "queued_for_approval".to_string()
                } else if proposal.proposal_kind == "chat_suggestion_accept" {
                    "running".to_string()
                } else {
                    "completed".to_string()
                };
                current.approved_at = Some(now_rfc3339());
                current.updated_at = now_rfc3339();
                current.trace_id = trace_id.clone();
                current.run_status = Some(run_status);
                current.last_run_summary = Some(summary.clone());
                let response_proposal = current.clone();
                save_proposals(&storage, &proposals).await;
                super::spawn_autonomy_analysis_tick(state.agent.clone(), "sentinel_proposal");
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "message": summary,
                        "trace_id": trace_id,
                        "proposal": response_proposal,
                    })),
                )
                    .into_response();
            }
            Err(error) => {
                current.status = "failed".to_string();
                current.updated_at = now_rfc3339();
                current.run_status = Some("failed".to_string());
                current.last_run_summary = Some(error.clone());
                save_proposals(&storage, &proposals).await;
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Proposal state disappeared during execution".to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn dismiss_sentinel_proposal(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    let mut proposals = load_proposals(&storage).await;
    let Some(proposal) = proposals.iter_mut().find(|item| item.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Proposal not found".to_string(),
            }),
        )
            .into_response();
    };
    proposal.status = "dismissed".to_string();
    proposal.dismissed_at = Some(now_rfc3339());
    proposal.updated_at = now_rfc3339();
    proposal.snoozed_until = None;
    save_proposals(&storage, &proposals).await;
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response()
}

pub(super) async fn snooze_sentinel_proposal(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    let mut proposals = load_proposals(&storage).await;
    let Some(proposal) = proposals.iter_mut().find(|item| item.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Proposal not found".to_string(),
            }),
        )
            .into_response();
    };
    let until = (chrono::Utc::now() + chrono::Duration::hours(6)).to_rfc3339();
    proposal.status = "snoozed".to_string();
    proposal.updated_at = now_rfc3339();
    proposal.snoozed_until = Some(until.clone());
    save_proposals(&storage, &proposals).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "snoozed_until": until })),
    )
        .into_response()
}
