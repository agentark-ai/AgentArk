use super::*;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

const SENTINEL_SCAN_STATE_KEY: &str = "sentinel_scan_state_v1";
const SENTINEL_OBSERVATIONS_KEY: &str = "sentinel_observations_v1";
const SENTINEL_PROPOSALS_KEY: &str = "sentinel_proposals_v1";
const SENTINEL_DAILY_AUTO_RUNS_KEY: &str = "sentinel_daily_auto_runs_v1";
const SENTINEL_DAILY_REVIEW_NOTICE_KEY: &str = "sentinel_daily_review_notice_v1";
const BACKGROUND_LEARNING_STATE_KEY: &str = "sentinel_background_learning_state_v1";
const BACKGROUND_LEARNING_STATE_LEASE_KEY: &str = "sentinel_background_learning_state_lease_v1";
const BACKGROUND_LEARNING_STATE_LEASE_TTL_SECS: i64 = 15;
const SENTINEL_SCAN_COOLDOWN_SECS: i64 = 30 * 60;
const MAX_SENTINEL_OBSERVATIONS: usize = 120;
const MAX_SENTINEL_PROPOSALS: usize = 96;
const SENTINEL_RETENTION_DAYS: i64 = 30;
const SENTINEL_PROPOSAL_RECREATE_HOURS: i64 = 24;
const IN_APP_EXECUTION_SCAN_LIMIT: u64 = 48;
const IN_APP_STALE_RUN_MINUTES: i64 = 15;
const SENTINEL_PROPOSAL_SEMANTIC_DEDUP_TIMEOUT_SECS: u64 = 8;
const CHAT_INTENT_SOURCE_KIND: &str = "chat_intent";
const CHAT_INTENT_DERIVED_FROM: &str = "autonomy_chat_suggestion";
const BACKGROUND_LEARNING_JOB_KEYS: [(&str, &str); 5] = [
    ("reflection_pass", "Reflection pass"),
    ("experience_consolidation", "Experience consolidation"),
    ("pattern_induction", "Pattern induction"),
    ("candidate_generation", "Candidate generation"),
    ("gepa_optimizer", "Prompt tuning"),
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
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct SentinelFeedStats {
    open_proposals: usize,
    completed_recently: usize,
    connected_services: usize,
    important_service_events: usize,
    in_app_events: usize,
    chat_suggestions: usize,
    recent_runs: usize,
    auto_mode_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SentinelDailyAutoRuns {
    day: String,
    count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SentinelDailyReviewNoticeState {
    #[serde(default)]
    last_day: Option<String>,
    #[serde(default)]
    last_sent_at: Option<String>,
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

pub(super) async fn load_scan_state(storage: &crate::storage::Storage) -> SentinelScanState {
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

pub(super) async fn load_observations(
    storage: &crate::storage::Storage,
) -> Vec<SentinelObservation> {
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

pub(super) async fn load_proposals(storage: &crate::storage::Storage) -> Vec<SentinelProposal> {
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

async fn load_daily_review_notice_state(
    storage: &crate::storage::Storage,
) -> SentinelDailyReviewNoticeState {
    match storage.get(SENTINEL_DAILY_REVIEW_NOTICE_KEY).await {
        Ok(Some(raw)) => serde_json::from_slice::<SentinelDailyReviewNoticeState>(&raw)
            .unwrap_or_else(|_| SentinelDailyReviewNoticeState {
                last_day: String::from_utf8(raw).ok(),
                ..SentinelDailyReviewNoticeState::default()
            }),
        _ => SentinelDailyReviewNoticeState::default(),
    }
}

async fn save_daily_review_notice_state(
    storage: &crate::storage::Storage,
    state: &SentinelDailyReviewNoticeState,
) -> Result<(), anyhow::Error> {
    let raw = serde_json::to_vec(state)?;
    storage.set(SENTINEL_DAILY_REVIEW_NOTICE_KEY, &raw).await
}

fn background_learning_label(key: &str) -> String {
    match key {
        "reflection_pass" => "Reflection pass",
        "experience_consolidation" => "Experience consolidation",
        "pattern_induction" => "Pattern induction",
        "candidate_generation" => "Candidate generation",
        "gepa_optimizer" => "Prompt tuning",
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
    let effective_status =
        if !learning_enabled || settings.autonomy_mode.eq_ignore_ascii_case("off") {
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

fn text_contains_direct_chat_approval_submit_text(text: &str) -> bool {
    text.split_whitespace().any(|part| {
        let candidate = part.trim_matches(|ch: char| {
            !(ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-'))
        });
        crate::core::parse_direct_chat_approval_submit_text(candidate).is_some()
    })
}

fn json_contains_direct_chat_approval_control(value: &serde_json::Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        serde_json::Value::String(text) => text_contains_direct_chat_approval_submit_text(text),
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_direct_chat_approval_control(item, depth + 1)),
        serde_json::Value::Object(map) => {
            let kind_is_direct_chat_approval = map
                .get("kind")
                .and_then(|value| value.as_str())
                .map(|value| {
                    let normalized = value.to_ascii_lowercase();
                    normalized.contains("direct_chat") && normalized.contains("approval")
                })
                .unwrap_or(false);
            let submit_text_is_direct_chat_approval = map
                .get("submit_text")
                .or_else(|| map.get("submitText"))
                .and_then(|value| value.as_str())
                .map(text_contains_direct_chat_approval_submit_text)
                .unwrap_or(false);
            if kind_is_direct_chat_approval || submit_text_is_direct_chat_approval {
                return true;
            }
            map.values()
                .any(|item| json_contains_direct_chat_approval_control(item, depth + 1))
        }
        _ => false,
    }
}

fn sentinel_observation_is_control_plane_artifact(observation: &SentinelObservation) -> bool {
    if observation.kind == "chat_suggestion" || observation.source_kind == "chat_suggestion" {
        return true;
    }

    matches!(
        observation.source_kind.as_str(),
        "execution_run" | "chat_suggestion"
    ) && (json_contains_direct_chat_approval_control(&observation.metadata, 0)
        || text_contains_direct_chat_approval_submit_text(&observation.title)
        || text_contains_direct_chat_approval_submit_text(&observation.detail))
}

fn sentinel_proposal_is_derived_chat_intent(proposal: &SentinelProposal) -> bool {
    proposal.source_kind == CHAT_INTENT_SOURCE_KIND
        && proposal.proposal_kind == "chat_suggestion_accept"
        && proposal.chat_suggestion_id.is_some()
        && proposal
            .metadata
            .get("derived_from")
            .and_then(|value| value.as_str())
            .map(|value| value == CHAT_INTENT_DERIVED_FROM)
            .unwrap_or(false)
}

fn sentinel_proposal_is_chat_continuation_artifact(proposal: &SentinelProposal) -> bool {
    if sentinel_proposal_is_derived_chat_intent(proposal) {
        return false;
    }
    proposal.proposal_kind == "chat_suggestion_accept"
        || proposal.source_kind == "chat_suggestion"
        || proposal.chat_suggestion_id.is_some()
}

fn sentinel_proposal_is_control_plane_artifact(proposal: &SentinelProposal) -> bool {
    if sentinel_proposal_is_chat_continuation_artifact(proposal) {
        return true;
    }

    matches!(
        proposal.source_kind.as_str(),
        "execution_run" | "chat_suggestion"
    ) && (json_contains_direct_chat_approval_control(&proposal.metadata, 0)
        || text_contains_direct_chat_approval_submit_text(&proposal.title)
        || text_contains_direct_chat_approval_submit_text(&proposal.detail)
        || text_contains_direct_chat_approval_submit_text(&proposal.rationale)
        || proposal
            .action
            .as_ref()
            .map(|action| json_contains_direct_chat_approval_control(&action.payload, 0))
            .unwrap_or(false))
}

fn sentinel_metadata_background_signal(metadata: &serde_json::Value) -> bool {
    metadata
        .get("background_signal")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn sentinel_observation_is_background_signal(observation: &SentinelObservation) -> bool {
    if observation.kind == "chat_suggestion" || observation.source_kind == "chat_suggestion" {
        return false;
    }
    if observation.source_kind == "execution_run" {
        return sentinel_metadata_background_signal(&observation.metadata);
    }
    true
}

fn sentinel_proposal_is_background_signal(proposal: &SentinelProposal) -> bool {
    if sentinel_proposal_is_chat_continuation_artifact(proposal) {
        return false;
    }
    if proposal.source_kind == "execution_run" {
        return sentinel_metadata_background_signal(&proposal.metadata);
    }
    true
}

fn prune_observations(
    observations: Vec<SentinelObservation>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<SentinelObservation> {
    let cutoff = retention_cutoff(now);
    let mut retained = observations
        .into_iter()
        .filter(|item| !sentinel_observation_is_control_plane_artifact(item))
        .filter(sentinel_observation_is_background_signal)
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
        .filter(|item| !sentinel_proposal_is_control_plane_artifact(item))
        .filter(sentinel_proposal_is_background_signal)
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
        .filter(|proposal| sentinel_proposal_is_background_signal(proposal))
        .filter(|proposal| {
            matches!(
                proposal.status.as_str(),
                "open" | "running" | "queued_for_approval"
            )
        })
        .count()
}

fn sentinel_daily_review_local_day(
    now: chrono::DateTime<chrono::Utc>,
    timezone: Option<&str>,
) -> String {
    timezone
        .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
        .map(|tz| now.with_timezone(&tz).date_naive().to_string())
        .unwrap_or_else(|| now.with_timezone(&chrono::Local).date_naive().to_string())
}

async fn agent_sentinel_daily_review_local_day(
    agent: &Agent,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let timezone = agent.user_profile.read().await.timezone.clone();
    sentinel_daily_review_local_day(now, timezone.as_deref())
}

fn sentinel_daily_review_body() -> &'static str {
    "Sentinel has background signals ready. Open Mission Control to review connected-source events or detached background work."
}

async fn maybe_send_sentinel_daily_review_notice(
    storage: &crate::storage::Storage,
    agent: &Agent,
    settings: &AutonomySettings,
    now: chrono::DateTime<chrono::Utc>,
    open_proposals: usize,
) -> bool {
    if open_proposals == 0
        || !settings.sentinel.enabled
        || settings.agent_paused
        || settings.autonomy_mode.eq_ignore_ascii_case("off")
        || is_quiet_hours_active(settings)
    {
        return false;
    }

    let day = agent_sentinel_daily_review_local_day(agent, now).await;
    let mut state = load_daily_review_notice_state(storage).await;
    if state.last_day.as_deref() == Some(day.as_str()) {
        return false;
    }

    state.last_day = Some(day);
    state.last_sent_at = Some(now.to_rfc3339());
    if let Err(error) = save_daily_review_notice_state(storage, &state).await {
        tracing::warn!("Failed to record Sentinel daily review notice: {}", error);
        return false;
    }

    agent
        .notify_preferred_channel(sentinel_daily_review_body())
        .await;
    true
}

fn sentinel_channel_for_action(action_kind: &str) -> bool {
    matches!(action_kind, "chat_prompt" | "create_task" | "watch")
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

#[derive(Default)]
struct SentinelCandidateBatch {
    observations: Vec<SentinelObservation>,
    proposals: Vec<SentinelProposal>,
    connected_services: usize,
    important_service_events: usize,
    in_app_events: usize,
    chat_suggestions: usize,
}

impl SentinelCandidateBatch {
    fn extend(&mut self, other: SentinelCandidateBatch) {
        self.connected_services = self.connected_services.max(other.connected_services);
        self.important_service_events += other.important_service_events;
        self.in_app_events += other.in_app_events;
        self.chat_suggestions += other.chat_suggestions;
        self.observations.extend(other.observations);
        self.proposals.extend(other.proposals);
    }
}

fn is_current_in_app_observation(observation: &SentinelObservation) -> bool {
    observation.source_kind == "execution_run"
}

fn is_current_in_app_proposal(proposal: &SentinelProposal) -> bool {
    proposal.source_kind == "execution_run"
}

fn is_current_chat_intent_observation(observation: &SentinelObservation) -> bool {
    observation.source_kind == CHAT_INTENT_SOURCE_KIND
}

fn is_current_chat_intent_proposal(proposal: &SentinelProposal) -> bool {
    sentinel_proposal_is_derived_chat_intent(proposal)
}

fn is_current_reconciled_observation(observation: &SentinelObservation) -> bool {
    is_current_in_app_observation(observation) || is_current_chat_intent_observation(observation)
}

fn is_current_reconciled_proposal(proposal: &SentinelProposal) -> bool {
    is_current_in_app_proposal(proposal) || is_current_chat_intent_proposal(proposal)
}

fn reconcile_sentinel_candidates(
    observations: &mut Vec<SentinelObservation>,
    proposals: &mut Vec<SentinelProposal>,
    candidate_batch: SentinelCandidateBatch,
    now: &chrono::DateTime<chrono::Utc>,
    prune_missing_in_app: bool,
) -> (usize, usize, Vec<String>) {
    if prune_missing_in_app {
        let current_observation_fingerprints = candidate_batch
            .observations
            .iter()
            .filter(|observation| is_current_reconciled_observation(observation))
            .map(|observation| observation.fingerprint.clone())
            .collect::<std::collections::HashSet<_>>();
        let current_proposal_fingerprints = candidate_batch
            .proposals
            .iter()
            .filter(|proposal| is_current_reconciled_proposal(proposal))
            .map(|proposal| proposal.fingerprint.clone())
            .collect::<std::collections::HashSet<_>>();

        observations.retain(|observation| {
            if !is_current_reconciled_observation(observation) {
                return true;
            }
            current_observation_fingerprints.contains(&observation.fingerprint)
        });
        proposals.retain(|proposal| {
            if !is_current_reconciled_proposal(proposal) {
                return true;
            }
            if !matches!(proposal.status.as_str(), "open" | "snoozed") {
                return true;
            }
            current_proposal_fingerprints.contains(&proposal.fingerprint)
        });
    }

    let mut created_observations = 0usize;
    let mut created_proposals = 0usize;
    let mut new_proposal_ids = Vec::new();

    for candidate in candidate_batch.observations {
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

    for candidate in candidate_batch.proposals {
        if let Some(existing) = proposals
            .iter_mut()
            .find(|item| item.fingerprint == candidate.fingerprint)
        {
            if recent_proposal_blocks_recreation(existing, *now) {
                existing.proposal_kind = candidate.proposal_kind;
                existing.title = candidate.title;
                existing.detail = candidate.detail;
                existing.rationale = candidate.rationale;
                existing.source_kind = candidate.source_kind;
                existing.source_id = candidate.source_id;
                existing.source_label = candidate.source_label;
                existing.confidence = candidate.confidence;
                existing.priority = candidate.priority;
                existing.trace_id = candidate.trace_id;
                existing.run_status = candidate.run_status;
                existing.last_run_summary = candidate.last_run_summary;
                existing.action = candidate.action;
                existing.chat_suggestion_id = candidate.chat_suggestion_id;
                existing.updated_at = now.to_rfc3339();
                existing.metadata = candidate.metadata;
                continue;
            }
        }
        new_proposal_ids.push(candidate.id.clone());
        proposals.push(candidate);
        created_proposals += 1;
    }

    (created_observations, created_proposals, new_proposal_ids)
}

struct InAppRunAttention {
    title: &'static str,
    proposal_title: &'static str,
    default_detail: &'static str,
    rationale: &'static str,
    priority: u8,
    confidence: f32,
    user_actionable: bool,
}

fn run_trace_key(run: &crate::core::ExecutionRun) -> Option<String> {
    run.trace_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn execution_run_is_detached_background_signal(run: &crate::core::ExecutionRun) -> bool {
    let kind = run.kind.trim().to_ascii_lowercase();
    let channel = run
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    if channel.as_deref() == Some("sentinel") {
        return true;
    }

    kind != "chat"
}

fn run_request_title(run: &crate::core::ExecutionRun, fallback: &str) -> String {
    let request = run
        .request_message
        .as_deref()
        .map(|value| compact_for_prompt(value, 72))
        .filter(|value| !value.is_empty());
    match request {
        Some(request) => format!("{fallback}: {request}"),
        None => fallback.to_string(),
    }
}

async fn load_run_clarification_choices(
    storage: &crate::storage::Storage,
    runs: &[crate::core::ExecutionRun],
) -> std::collections::HashMap<String, Vec<crate::core::ClarificationChoice>> {
    let trace_ids = runs.iter().filter_map(run_trace_key).collect::<Vec<_>>();
    if trace_ids.is_empty() {
        return std::collections::HashMap::new();
    }

    let rows = match storage
        .list_operational_logs_for_trace_ids_by_event(
            &trace_ids,
            "action_selection",
            (trace_ids.len().saturating_mul(4).max(32)) as u64,
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                "Failed to load Sentinel clarification choices from operational logs: {}",
                error
            );
            return std::collections::HashMap::new();
        }
    };

    let mut by_trace_id = std::collections::HashMap::new();
    for row in rows {
        let Some(trace_id) = row
            .trace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if by_trace_id.contains_key(trace_id) {
            continue;
        }
        let choices = super::clarification_choices_from_operational_payload(row.payload.as_deref());
        if choices.is_empty() {
            continue;
        }
        by_trace_id.insert(trace_id.to_string(), choices);
    }
    by_trace_id
}

fn compact_for_prompt(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect()
}

fn run_nonempty_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| compact_for_prompt(value, 480))
}

fn run_degradation_summary(run: &crate::core::ExecutionRun) -> Option<String> {
    let parts = run
        .degradation
        .iter()
        .take(2)
        .filter_map(|note| {
            let summary = note.summary.trim();
            if !summary.is_empty() {
                Some(summary.to_string())
            } else {
                note.detail
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
            }
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(compact_for_prompt(&parts.join(" | "), 480))
    }
}

fn clarification_choice_is_direct_chat_approval(choice: &crate::core::ClarificationChoice) -> bool {
    if choice.approval.is_some() {
        return true;
    }
    if crate::core::parse_direct_chat_approval_submit_text(&choice.submit_text).is_some() {
        return true;
    }
    choice
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let normalized = value.to_ascii_lowercase();
            normalized.contains("direct_chat") && normalized.contains("approval")
        })
        .unwrap_or(false)
}

fn clarification_choice_is_sentinel_reviewable(choice: &crate::core::ClarificationChoice) -> bool {
    !clarification_choice_is_direct_chat_approval(choice)
        && !choice.label.trim().is_empty()
        && !choice.submit_text.trim().is_empty()
}

fn run_is_direct_chat_approval_control_flow(
    run: &crate::core::ExecutionRun,
    choices: &[crate::core::ClarificationChoice],
) -> bool {
    run.request_message
        .as_deref()
        .and_then(crate::core::parse_direct_chat_approval_submit_text)
        .is_some()
        || choices
            .iter()
            .any(clarification_choice_is_direct_chat_approval)
}

fn run_attention_detail(run: &crate::core::ExecutionRun, default_detail: &str) -> String {
    run_nonempty_field(run.result_summary.as_deref())
        .or_else(|| run_nonempty_field(run.last_error.as_deref()))
        .or_else(|| run_degradation_summary(run))
        .unwrap_or_else(|| default_detail.to_string())
}

fn run_is_transient_router_failure(run: &crate::core::ExecutionRun, detail: &str) -> bool {
    let mut haystack = detail.to_ascii_lowercase();
    haystack.push('\n');
    haystack.push_str(
        &run.result_summary
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase(),
    );
    haystack.push('\n');
    haystack.push_str(&run.last_error.as_deref().unwrap_or("").to_ascii_lowercase());
    haystack.contains("semantic router")
        || haystack.contains("could not route this request")
        || haystack.contains("router model call failed")
        || haystack.contains("unified semantic router failed")
}

fn should_create_in_app_execution_proposal(
    run: &crate::core::ExecutionRun,
    detail: &str,
    choices: &[crate::core::ClarificationChoice],
    detached_background_signal: bool,
) -> bool {
    use crate::core::ExecutionRunStatus;

    if run_is_transient_router_failure(run, detail) {
        return false;
    }
    if run_is_direct_chat_approval_control_flow(run, choices) {
        return false;
    }

    match run.status {
        ExecutionRunStatus::NeedsInput => {
            choices
                .iter()
                .any(clarification_choice_is_sentinel_reviewable)
                || detached_background_signal
        }
        ExecutionRunStatus::NeedsStrongerModel => true,
        ExecutionRunStatus::Blocked
        | ExecutionRunStatus::PlatformFailed
        | ExecutionRunStatus::Degraded
        | ExecutionRunStatus::Accepted
        | ExecutionRunStatus::Routing
        | ExecutionRunStatus::ModelSelection
        | ExecutionRunStatus::Planning
        | ExecutionRunStatus::ToolDispatch
        | ExecutionRunStatus::Synthesis => detached_background_signal,
        _ => false,
    }
}

fn in_app_execution_run_is_sentinel_material(
    run: &crate::core::ExecutionRun,
    attention: &InAppRunAttention,
    choices: &[crate::core::ClarificationChoice],
) -> bool {
    let detached_background_signal = execution_run_is_detached_background_signal(run);
    if !detached_background_signal {
        return false;
    }
    let detail = run_attention_detail(run, attention.default_detail);
    if run_is_transient_router_failure(run, &detail) {
        return false;
    }
    if run_is_direct_chat_approval_control_flow(run, choices) {
        return false;
    }
    should_create_in_app_execution_proposal(run, &detail, choices, detached_background_signal)
}

fn execution_run_is_stale(
    run: &crate::core::ExecutionRun,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if parse_optional_utc(run.deadline_at.as_deref())
        .map(|deadline| deadline <= now)
        .unwrap_or(false)
    {
        return true;
    }
    parse_optional_utc(Some(&run.updated_at))
        .map(|updated| (now - updated).num_minutes() >= IN_APP_STALE_RUN_MINUTES)
        .unwrap_or(false)
}

fn in_app_attention_for_run(
    run: &crate::core::ExecutionRun,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<InAppRunAttention> {
    use crate::core::ExecutionRunStatus;

    match &run.status {
        ExecutionRunStatus::NeedsInput => Some(InAppRunAttention {
            title: "Decision needed",
            proposal_title: "Choose how to continue",
            default_detail: "A detached AgentArk run stopped because it needs more information.",
            rationale: "This background run is not attached to an active chat and needs a deliberate next step.",
            priority: 5,
            confidence: 0.92,
            user_actionable: true,
        }),
        ExecutionRunStatus::Blocked => Some(InAppRunAttention {
            title: "Request is blocked",
            proposal_title: "Review what blocked the request",
            default_detail: "A detached AgentArk run could not continue without an external requirement.",
            rationale: "This background run is blocked outside an active chat and needs triage.",
            priority: 5,
            confidence: 0.9,
            user_actionable: false,
        }),
        ExecutionRunStatus::PlatformFailed => Some(InAppRunAttention {
            title: "Request failed",
            proposal_title: "Review the failed request",
            default_detail: "A detached AgentArk run failed before it could complete.",
            rationale: "This background run failed outside an active chat and needs triage.",
            priority: 5,
            confidence: 0.95,
            user_actionable: false,
        }),
        ExecutionRunStatus::NeedsStrongerModel => Some(InAppRunAttention {
            title: "Stronger model needed",
            proposal_title: "Review the model escalation",
            default_detail: "A detached AgentArk run could not complete with the selected model.",
            rationale: "This background run needs a model decision before it continues.",
            priority: 4,
            confidence: 0.86,
            user_actionable: true,
        }),
        ExecutionRunStatus::Degraded => Some(InAppRunAttention {
            title: "Completed with caveats",
            proposal_title: "Review the caveat",
            default_detail: "A detached AgentArk run completed with degraded execution quality.",
            rationale: "This background run completed with caveats that may need a follow-up.",
            priority: 3,
            confidence: 0.78,
            user_actionable: false,
        }),
        ExecutionRunStatus::Accepted
        | ExecutionRunStatus::Routing
        | ExecutionRunStatus::ModelSelection
        | ExecutionRunStatus::Planning
        | ExecutionRunStatus::ToolDispatch
        | ExecutionRunStatus::Synthesis
            if execution_run_is_stale(run, now) =>
        {
            Some(InAppRunAttention {
                title: "Request appears stalled",
                proposal_title: "Check the stalled request",
                default_detail: "A detached AgentArk run did not reach a terminal state in time.",
                rationale: "This background run is still in progress after its expected window.",
                priority: 4,
                confidence: 0.74,
                user_actionable: false,
            })
        }
        _ => None,
    }
}

fn run_source_label(run: &crate::core::ExecutionRun) -> String {
    match run
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some("web") => "Chat".to_string(),
        Some("telegram") => "Telegram".to_string(),
        Some("whatsapp") => "WhatsApp".to_string(),
        Some("slack") => "Slack".to_string(),
        Some("discord") => "Discord".to_string(),
        Some(value) => format!("{} request", value),
        None => "AgentArk request".to_string(),
    }
}

fn build_in_app_execution_prompt(
    run: &crate::core::ExecutionRun,
    attention: &InAppRunAttention,
    detail: &str,
) -> String {
    let request_preview = run
        .request_message
        .as_deref()
        .map(|value| compact_for_prompt(value, 700))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Not available".to_string());
    format!(
        "A detached or background AgentArk execution run needs triage.\n\
Run ID: {}\n\
Trace ID: {}\n\
Conversation ID: {}\n\
Channel: {}\n\
Status: {}\n\
Signal: {}\n\
Summary: {}\n\
Original request preview: {}\n\n\
Take the safest concrete next step. If the run needs input, ask for the missing input clearly. If it failed or degraded, diagnose the failure and retry only when safe. If the best next step is a task, watcher, or reminder, create that concrete follow-up instead of only explaining options.",
        run.id,
        run.trace_id.as_deref().unwrap_or(""),
        run.conversation_id.as_deref().unwrap_or(""),
        run.channel.as_deref().unwrap_or(""),
        run.status.as_str(),
        attention.title,
        detail,
        request_preview,
    )
}

fn build_in_app_execution_candidates(
    run: &crate::core::ExecutionRun,
    attention: &InAppRunAttention,
    settings: &AutonomySettings,
    readiness_policy: &crate::core::ReadinessPolicy,
    now: &str,
    choices: &[crate::core::ClarificationChoice],
) -> (SentinelObservation, Option<SentinelProposal>) {
    let detail = run_attention_detail(run, attention.default_detail);
    let detached_background_signal = execution_run_is_detached_background_signal(run);
    let source_label = run_source_label(run);
    let fingerprint =
        normalize_fingerprint(&["in_app_execution", run.id.as_str(), run.status.as_str()]);
    let observation = SentinelObservation {
        id: String::new(),
        fingerprint: fingerprint.clone(),
        kind: "in_app_run_attention".to_string(),
        title: attention.title.to_string(),
        detail: detail.clone(),
        source_kind: "execution_run".to_string(),
        source_id: Some(run.id.clone()),
        source_label: Some(source_label.clone()),
        confidence: attention.confidence,
        priority: attention.priority,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        metadata: serde_json::json!({
            "run_id": run.id,
            "trace_id": run.trace_id,
            "conversation_id": run.conversation_id,
            "channel": run.channel,
            "status": run.status.as_str(),
            "current_stage": run.current_stage,
            "deadline_at": run.deadline_at,
            "user_actionable": attention.user_actionable,
            "background_signal": detached_background_signal,
        }),
    };

    let proposal = {
        let should_propose = should_create_in_app_execution_proposal(
            run,
            &detail,
            choices,
            detached_background_signal,
        );
        if !should_propose {
            return (observation, None);
        }
        let proposal_title = run_request_title(run, attention.proposal_title);
        let action = super::recommendation(
            &proposal_title,
            &detail,
            "chat_prompt",
            serde_json::json!({
                "prompt": build_in_app_execution_prompt(run, attention, &detail),
            }),
            &settings.trust_policy,
            readiness_policy,
        );
        let proposal_id = uuid::Uuid::new_v4().to_string();
        Some(SentinelProposal {
            id: proposal_id.clone(),
            fingerprint: normalize_fingerprint(&[
                "proposal",
                "in_app_execution",
                run.id.as_str(),
                run.status.as_str(),
            ]),
            proposal_kind: "recommended_action".to_string(),
            status: "open".to_string(),
            title: action.title.clone(),
            detail,
            rationale: attention.rationale.to_string(),
            source_kind: "execution_run".to_string(),
            source_id: Some(run.id.clone()),
            source_label: Some(source_label),
            confidence: attention.confidence,
            priority: attention.priority,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            snoozed_until: None,
            approved_at: None,
            dismissed_at: None,
            trace_id: run.trace_id.clone(),
            run_status: Some(run.status.as_str().to_string()),
            last_run_summary: run.result_summary.clone(),
            action: Some(decorate_action_for_sentinel(
                &action,
                &proposal_id,
                "execution_run",
                Some(&run.id),
            )),
            chat_suggestion_id: None,
            metadata: serde_json::json!({
                "run_id": run.id,
                "trace_id": run.trace_id,
                "conversation_id": run.conversation_id,
                "channel": run.channel,
                "status": run.status.as_str(),
                "current_stage": run.current_stage,
                "user_actionable": true,
                "background_signal": detached_background_signal,
                "choices": choices,
            }),
        })
    };
    (observation, proposal)
}

fn sentinel_proposal_can_block_duplicate(proposal: &SentinelProposal) -> bool {
    matches!(
        proposal.status.as_str(),
        "open"
            | "running"
            | "queued_for_approval"
            | "snoozed"
            | "completed"
            | "dismissed"
            | "failed"
    )
}

fn sentinel_proposal_is_removable_duplicate(proposal: &SentinelProposal) -> bool {
    matches!(proposal.status.as_str(), "open" | "snoozed")
}

fn sentinel_proposal_representative_rank(proposal: &SentinelProposal) -> (u8, u8, f32, i64) {
    let status_rank = if sentinel_proposal_is_removable_duplicate(proposal) {
        1
    } else {
        0
    };
    let updated_at = parse_optional_utc(Some(&proposal.updated_at))
        .map(|value| value.timestamp())
        .unwrap_or(0);
    (
        status_rank,
        proposal.priority,
        proposal.confidence,
        updated_at,
    )
}

fn compact_json_for_semantic_payload(value: &serde_json::Value, max_chars: usize) -> String {
    serde_json::to_string(value)
        .map(|raw| compact_for_prompt(&raw, max_chars))
        .unwrap_or_default()
}

fn sentinel_proposal_semantic_payload(proposal: &SentinelProposal) -> serde_json::Value {
    let action_payload = proposal.action.as_ref().map(|action| {
        serde_json::json!({
            "action_kind": action.action_kind,
            "title": compact_for_prompt(&action.title, 240),
            "description": compact_for_prompt(&action.description, 480),
            "payload": compact_json_for_semantic_payload(&action.payload, 1200),
        })
    });

    serde_json::json!({
        "id": proposal.id,
        "status": proposal.status,
        "kind": proposal.proposal_kind,
        "title": compact_for_prompt(&proposal.title, 240),
        "detail": compact_for_prompt(&proposal.detail, 700),
        "rationale": compact_for_prompt(&proposal.rationale, 700),
        "source_kind": proposal.source_kind,
        "source_label": proposal.source_label,
        "trace_id": proposal.trace_id,
        "run_status": proposal.run_status,
        "last_run_summary": proposal
            .last_run_summary
            .as_deref()
            .map(|value| compact_for_prompt(value, 700)),
        "chat_suggestion_id": proposal.chat_suggestion_id,
        "action": action_payload,
        "metadata": compact_json_for_semantic_payload(&proposal.metadata, 1200),
    })
}

fn extract_json_value(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]).ok()
}

fn parse_sentinel_proposal_duplicate_groups(
    text: &str,
    valid_ids: &std::collections::HashSet<String>,
) -> Vec<Vec<String>> {
    let Some(payload) = extract_json_value(text) else {
        return Vec::new();
    };
    let groups = payload
        .get("duplicate_groups")
        .or_else(|| payload.get("groups"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut parsed = Vec::new();
    let mut seen_groups = std::collections::HashSet::new();
    for group in groups {
        let Some(items) = group.as_array() else {
            continue;
        };
        let mut ids = items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::trim)
            .filter(|id| valid_ids.contains(*id))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        if ids.len() < 2 {
            continue;
        }
        let signature = ids.join("\u{1f}");
        if seen_groups.insert(signature) {
            parsed.push(ids);
        }
    }
    parsed
}

fn apply_sentinel_proposal_duplicate_groups(
    proposals: &mut Vec<SentinelProposal>,
    groups: &[Vec<String>],
    now: &str,
) -> usize {
    if proposals.len() < 2 || groups.is_empty() {
        return 0;
    }

    let mut index_by_id = std::collections::HashMap::new();
    for (idx, proposal) in proposals.iter().enumerate() {
        index_by_id.insert(proposal.id.clone(), idx);
    }

    let mut remove_ids = std::collections::HashSet::new();
    for group in groups {
        let indices = group
            .iter()
            .filter_map(|id| index_by_id.get(id).copied())
            .collect::<Vec<_>>();
        if indices.len() < 2 {
            continue;
        }

        let Some(representative_idx) = indices.iter().copied().min_by(|left, right| {
            let left_rank = sentinel_proposal_representative_rank(&proposals[*left]);
            let right_rank = sentinel_proposal_representative_rank(&proposals[*right]);
            left_rank
                .0
                .cmp(&right_rank.0)
                .then_with(|| right_rank.1.cmp(&left_rank.1))
                .then_with(|| right_rank.2.total_cmp(&left_rank.2))
                .then_with(|| right_rank.3.cmp(&left_rank.3))
                .then_with(|| proposals[*left].id.cmp(&proposals[*right].id))
        }) else {
            continue;
        };

        for duplicate_idx in indices {
            if duplicate_idx == representative_idx {
                continue;
            }
            if !sentinel_proposal_is_removable_duplicate(&proposals[duplicate_idx]) {
                continue;
            }
            remove_ids.insert(proposals[duplicate_idx].id.clone());
        }

        if let Some(representative) = proposals.get_mut(representative_idx) {
            representative.updated_at = now.to_string();
        }
    }

    if remove_ids.is_empty() {
        return 0;
    }
    let before = proposals.len();
    proposals.retain(|proposal| !remove_ids.contains(&proposal.id));
    before.saturating_sub(proposals.len())
}

fn build_sentinel_proposal_dedup_prompt(proposals: &[SentinelProposal]) -> Option<String> {
    let items = proposals
        .iter()
        .filter(|proposal| sentinel_proposal_can_block_duplicate(proposal))
        .map(sentinel_proposal_semantic_payload)
        .collect::<Vec<_>>();
    if items.len() < 2 {
        return None;
    }

    Some(format!(
        "You are deduplicating Sentinel proposals before they are shown to the user.\n\
         Cluster proposals only when they represent the same underlying user intent or same useful next action, and resolving one would make the others redundant.\n\
         Judge by intended outcome, target, constraints, schedule, recipient, external system, and required user decision. \
         Wording, order, casing, punctuation, grammar, abbreviations, typos, and source type are irrelevant. \
         Do not use keyword matching or phrase templates; decide from meaning.\n\
         Keep proposals separate when they require materially different decisions, deliverables, targets, constraints, schedules, recipients, or external systems.\n\n\
         Proposals:\n{}\n\n\
         Return strict JSON only with this shape: {{\"duplicate_groups\":[[\"id-a\",\"id-b\"]]}}. \
         Include only groups with two or more ids. Return an empty array when nothing is redundant.",
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
    ))
}

async fn collapse_semantically_equivalent_sentinel_proposals(
    llm: &crate::core::LlmClient,
    proposals: &mut Vec<SentinelProposal>,
    now: &str,
) -> usize {
    let valid_ids = proposals
        .iter()
        .filter(|proposal| sentinel_proposal_can_block_duplicate(proposal))
        .map(|proposal| proposal.id.clone())
        .collect::<std::collections::HashSet<_>>();
    if valid_ids.len() < 2 {
        return 0;
    }
    let Some(prompt) = build_sentinel_proposal_dedup_prompt(proposals) else {
        return 0;
    };
    let system =
        "You are a strict semantic deduplication checker for Sentinel proposals. Return JSON only.";
    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(SENTINEL_PROPOSAL_SEMANTIC_DEDUP_TIMEOUT_SECS),
        llm.chat_with_system_bounded(system, &prompt, 420),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::debug!(
                "sentinel_proposal_dedup: LLM clustering unavailable: {}",
                error
            );
            return 0;
        }
        Err(_) => {
            tracing::debug!(
                "sentinel_proposal_dedup: LLM clustering timed out after {}s",
                SENTINEL_PROPOSAL_SEMANTIC_DEDUP_TIMEOUT_SECS
            );
            return 0;
        }
    };
    let groups = parse_sentinel_proposal_duplicate_groups(&response.content, &valid_ids);
    apply_sentinel_proposal_duplicate_groups(proposals, &groups, now)
}

async fn build_in_app_candidates(
    storage: &crate::storage::Storage,
    settings: &AutonomySettings,
    now: chrono::DateTime<chrono::Utc>,
) -> SentinelCandidateBatch {
    let now_text = now.to_rfc3339();
    let readiness_policy = crate::core::readiness::load_readiness_policy(storage).await;
    let mut batch = SentinelCandidateBatch::default();
    let recent_runs = storage
        .list_recent_execution_runs(IN_APP_EXECUTION_SCAN_LIMIT)
        .await
        .unwrap_or_default();
    let mut attention_runs = Vec::new();
    for run in recent_runs {
        if !execution_run_is_detached_background_signal(&run) {
            continue;
        }
        let Some(attention) = in_app_attention_for_run(&run, now) else {
            continue;
        };
        attention_runs.push((run, attention));
    }

    let runs_for_choices = attention_runs
        .iter()
        .map(|(run, _)| run.clone())
        .collect::<Vec<_>>();
    let choices_by_trace_id = load_run_clarification_choices(storage, &runs_for_choices).await;

    for (run, attention) in attention_runs {
        let choices = run_trace_key(&run)
            .and_then(|trace_id| choices_by_trace_id.get(&trace_id))
            .map(|choices| choices.as_slice())
            .unwrap_or(&[]);
        if !in_app_execution_run_is_sentinel_material(&run, &attention, choices) {
            continue;
        }
        let (observation, proposal) = build_in_app_execution_candidates(
            &run,
            &attention,
            settings,
            &readiness_policy,
            &now_text,
            choices,
        );
        batch.observations.push(observation);
        if let Some(proposal) = proposal {
            batch.proposals.push(proposal);
        }
        batch.in_app_events += 1;
    }

    batch
}

fn chat_suggestion_is_open(suggestion: &super::autonomy_support::ChatAutomationSuggestion) -> bool {
    suggestion.status.trim().eq_ignore_ascii_case("open")
}

fn chat_suggestion_tags(
    suggestion: &super::autonomy_support::ChatAutomationSuggestion,
) -> Vec<String> {
    let mut tags = vec![
        "derived_from_chat".to_string(),
        "operational_intent".to_string(),
    ];
    let intent_kind = suggestion.kind.trim().to_ascii_lowercase();
    if !intent_kind.is_empty() {
        tags.push(intent_kind);
    }
    tags
}

fn chat_suggestion_priority(confidence: f32, threshold: f32) -> u8 {
    let confidence = confidence.clamp(0.0, 1.0);
    let threshold = threshold.clamp(0.0, 1.0);
    if confidence >= threshold.max(0.9) {
        5
    } else if confidence >= threshold {
        4
    } else {
        3
    }
}

fn build_chat_suggestion_sentinel_launch_prompt(
    suggestion: &super::autonomy_support::ChatAutomationSuggestion,
) -> String {
    let focus = suggestion
        .goal_detail
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| suggestion.goal_title.trim());
    format!(
        "A Sentinel-derived operational intent was accepted.\n\
Intent kind: {}\n\
Title: {}\n\
Detail: {}\n\
Rationale: {}\n\
Requested outcome: {}\n\n\
Execute the concrete durable outcome represented by this intent. If required inputs are missing, \
ask for the missing input clearly. Return a concise final outcome after actual work is done.",
        compact_for_prompt(&suggestion.kind, 80),
        compact_for_prompt(&suggestion.title, 180),
        compact_for_prompt(&suggestion.detail, 420),
        compact_for_prompt(&suggestion.rationale, 360),
        compact_for_prompt(focus, 520),
    )
}

fn chat_suggestion_metadata(
    suggestion: &super::autonomy_support::ChatAutomationSuggestion,
    tags: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "derived_from": CHAT_INTENT_DERIVED_FROM,
        "raw_chat_visible": false,
        "suggestion_id": suggestion.id,
        "intent_kind": suggestion.kind,
        "tags": tags,
        "conversation_id": suggestion.conversation_id,
        "conversation_channel": suggestion.conversation_channel,
        "source_message_id": suggestion.source_message_id,
        "goal_title": suggestion.goal_title,
        "goal_detail": suggestion.goal_detail,
    })
}

fn build_chat_suggestion_candidates_from_suggestions(
    suggestions: &[super::autonomy_support::ChatAutomationSuggestion],
    settings: &AutonomySettings,
    readiness_policy: &crate::core::ReadinessPolicy,
    now: &str,
) -> SentinelCandidateBatch {
    let mut batch = SentinelCandidateBatch::default();
    let threshold = settings.sentinel.confidence_threshold.clamp(0.0, 1.0);

    for suggestion in suggestions {
        if !chat_suggestion_is_open(suggestion) {
            continue;
        }
        let confidence = suggestion.confidence.clamp(0.0, 1.0);
        if confidence < threshold {
            continue;
        }
        let tags = chat_suggestion_tags(suggestion);
        let priority = chat_suggestion_priority(confidence, threshold);
        let metadata = chat_suggestion_metadata(suggestion, &tags);
        let source_label = "Derived chat intent".to_string();
        let fingerprint = normalize_fingerprint(&[
            CHAT_INTENT_SOURCE_KIND,
            suggestion.id.as_str(),
            suggestion.fingerprint.as_str(),
        ]);
        batch.observations.push(SentinelObservation {
            id: String::new(),
            fingerprint: fingerprint.clone(),
            kind: "chat_intent_signal".to_string(),
            title: suggestion.title.clone(),
            detail: suggestion.detail.clone(),
            source_kind: CHAT_INTENT_SOURCE_KIND.to_string(),
            source_id: Some(suggestion.id.clone()),
            source_label: Some(source_label.clone()),
            confidence,
            priority,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            metadata: metadata.clone(),
        });

        let action = super::recommendation(
            &suggestion.title,
            &suggestion.detail,
            "chat_prompt",
            serde_json::json!({
                "prompt": build_chat_suggestion_sentinel_launch_prompt(suggestion),
                "conversation_id": suggestion.conversation_id,
                "suggestion_id": suggestion.id,
                "sentinel_signal_kind": CHAT_INTENT_SOURCE_KIND,
            }),
            &settings.trust_policy,
            readiness_policy,
        );
        let proposal_id = uuid::Uuid::new_v4().to_string();
        batch.proposals.push(SentinelProposal {
            id: proposal_id.clone(),
            fingerprint: normalize_fingerprint(&[
                "proposal",
                CHAT_INTENT_SOURCE_KIND,
                suggestion.id.as_str(),
                suggestion.fingerprint.as_str(),
            ]),
            proposal_kind: "chat_suggestion_accept".to_string(),
            status: "open".to_string(),
            title: action.title.clone(),
            detail: suggestion.detail.clone(),
            rationale: suggestion.rationale.clone(),
            source_kind: CHAT_INTENT_SOURCE_KIND.to_string(),
            source_id: Some(suggestion.id.clone()),
            source_label: Some(source_label),
            confidence,
            priority,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            snoozed_until: None,
            approved_at: None,
            dismissed_at: None,
            trace_id: None,
            run_status: None,
            last_run_summary: None,
            action: Some(decorate_action_for_sentinel(
                &action,
                &proposal_id,
                CHAT_INTENT_SOURCE_KIND,
                Some(&suggestion.id),
            )),
            chat_suggestion_id: Some(suggestion.id.clone()),
            metadata,
        });
        batch.chat_suggestions += 1;
    }

    batch
}

async fn build_chat_suggestion_candidates(
    storage: &crate::storage::Storage,
    settings: &AutonomySettings,
    readiness_policy: &crate::core::ReadinessPolicy,
    now: &str,
) -> SentinelCandidateBatch {
    let suggestions = super::autonomy_support::load_chat_suggestions(storage).await;
    build_chat_suggestion_candidates_from_suggestions(&suggestions, settings, readiness_policy, now)
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
    storage: &crate::storage::Storage,
    integration_ctx: &crate::core::integration_sync::IntegrationSyncContext,
    settings: &AutonomySettings,
) -> SentinelCandidateBatch {
    let now = now_rfc3339();
    let readiness_policy = crate::core::readiness::load_readiness_policy(storage).await;
    let mut batch = SentinelCandidateBatch::default();

    if settings.sentinel.watch_connected_services {
        let statuses = crate::core::integration_sync::list_statuses(integration_ctx).await;
        batch.connected_services = statuses
            .iter()
            .filter(|status| status.connected && status.supported)
            .count();
        let feed_items =
            crate::core::integration_sync::list_feed_items(integration_ctx, None, 18).await;
        for item in feed_items.into_iter().filter(|entry| {
            entry.important || entry.importance >= settings.sentinel.confidence_threshold
        }) {
            batch.important_service_events += 1;
            let fingerprint = normalize_fingerprint(&[
                "integration_feed",
                item.integration_id.as_str(),
                item.id.as_str(),
            ]);
            batch.observations.push(SentinelObservation {
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
                    &readiness_policy,
                );
                let proposal_id = uuid::Uuid::new_v4().to_string();
                batch.proposals.push(SentinelProposal {
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
                    metadata: serde_json::json!({
                        "integration_id": item.integration_id,
                        "integration_name": item.integration_name,
                        "integration_kind": item.kind,
                        "url": item.url,
                        "occurred_at": item.occurred_at,
                        "detected_at": item.detected_at,
                    }),
                });
            }
        }
    }

    if settings.sentinel.watch_in_app {
        let in_app_batch = build_in_app_candidates(storage, settings, chrono::Utc::now()).await;
        batch.extend(in_app_batch);
    }

    if settings.sentinel.watch_in_app && settings.sentinel.infer_new_automations {
        let chat_batch =
            build_chat_suggestion_candidates(storage, settings, &readiness_policy, &now).await;
        batch.extend(chat_batch);
    }

    batch.observations.sort_by(|a, b| {
        b.priority.cmp(&a.priority).then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    batch.proposals.sort_by(|a, b| {
        b.priority.cmp(&a.priority).then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    batch
        .proposals
        .truncate(settings.sentinel.max_proposals_per_scan.max(1) as usize);
    batch
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
        cached_prompt_tokens: 0,
        cache_creation_prompt_tokens: 0,
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

    let readiness_policy = crate::core::readiness::load_readiness_policy(storage).await;
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
        if action.action_kind == "chat_prompt" {
            continue;
        }
        let risk = score_action_risk(&action.action_kind, &action.payload, &settings.trust_policy);
        let readiness = action.readiness.clone().unwrap_or_else(|| {
            crate::core::readiness::evaluate_recommended_action_readiness(&risk, &readiness_policy)
        });
        let mut action = action;
        action.trust = risk.clone();
        action.readiness = Some(readiness.clone());
        proposal.action = Some(action);
        if let Err(error) = crate::core::readiness::record_readiness_evaluation(
            storage,
            "sentinel_recommended_action",
            &proposal.id,
            &readiness,
        )
        .await
        {
            tracing::warn!(
                proposal_id = %proposal.id,
                error = %error,
                "Failed to record Sentinel readiness evaluation"
            );
        }
        if risk.requires_approval || risk.score > settings.trust_policy.auto_execute_max_score {
            continue;
        }
        if !readiness.allows_auto {
            proposal.last_run_summary =
                Some(format!("Skipped auto-run: {}", readiness.plain_summary));
            proposal.updated_at = now_rfc3339();
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
    tracing::info!(trigger = trigger, "Sentinel scan tick loading state");
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
        tracing::info!(
            trigger = trigger,
            "Sentinel scan skipped because autonomy is disabled"
        );
        return serde_json::json!({
            "status": "disabled",
            "trigger": trigger,
            "generated_at": now.to_rfc3339(),
        });
    }

    let mut scan_state = load_scan_state(&storage).await;
    if !should_run_scan(&scan_state, trigger, now) {
        tracing::info!(trigger = trigger, "Sentinel scan skipped by cooldown");
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
    refresh_snoozed_proposals(&mut proposals, now);
    let mut agent_snapshot: Option<Agent> = None;

    tracing::info!(trigger = trigger, "Sentinel scan building candidates");
    let candidate_batch = build_candidates(&storage, &integration_ctx, &settings).await;
    let connected_services = candidate_batch.connected_services;
    let important_service_events = candidate_batch.important_service_events;
    let in_app_events = candidate_batch.in_app_events;
    let chat_suggestions = candidate_batch.chat_suggestions;
    let (created_observations, mut created_proposals, mut new_proposal_ids) =
        reconcile_sentinel_candidates(
            &mut observations,
            &mut proposals,
            candidate_batch,
            &now,
            true,
        );
    if proposals
        .iter()
        .filter(|proposal| sentinel_proposal_can_block_duplicate(proposal))
        .take(2)
        .count()
        > 1
    {
        if agent_snapshot.is_none() {
            agent_snapshot = Some(Agent::snapshot(&shared).await);
        }
        let removed_duplicates = collapse_semantically_equivalent_sentinel_proposals(
            &agent_snapshot.as_ref().expect("agent snapshot").llm,
            &mut proposals,
            &now.to_rfc3339(),
        )
        .await;
        if removed_duplicates > 0 {
            let live_ids = proposals
                .iter()
                .map(|proposal| proposal.id.clone())
                .collect::<std::collections::HashSet<_>>();
            new_proposal_ids.retain(|id| live_ids.contains(id));
            created_proposals = new_proposal_ids.len();
            tracing::info!(
                trigger = trigger,
                removed_duplicate_proposals = removed_duplicates,
                "Sentinel collapsed semantically duplicate proposals"
            );
        }
    }
    tracing::info!(
        trigger = trigger,
        connected_services = connected_services,
        important_service_events = important_service_events,
        in_app_events = in_app_events,
        chat_suggestions = chat_suggestions,
        created_observations = created_observations,
        created_proposals = created_proposals,
        "Sentinel scan reconciled candidates"
    );

    let auto_executed = if new_proposal_ids.is_empty() {
        0
    } else {
        tracing::info!(
            trigger = trigger,
            new_proposals = new_proposal_ids.len(),
            "Sentinel scan auto-execution check started"
        );
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

    tracing::info!(trigger = trigger, "Sentinel scan persisting results");
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
    tracing::info!(
        trigger = trigger,
        created_observations = created_observations,
        created_proposals = created_proposals,
        auto_executed = auto_executed,
        open_proposals = scan_state.open_proposals,
        "Sentinel scan tick completed"
    );

    let daily_review_notice_sent = if scan_state.open_proposals > 0 {
        if agent_snapshot.is_none() {
            agent_snapshot = Some(Agent::snapshot(&shared).await);
        }
        let agent = agent_snapshot.as_ref().expect("agent snapshot");
        maybe_send_sentinel_daily_review_notice(
            &storage,
            agent,
            &settings,
            now,
            scan_state.open_proposals,
        )
        .await
    } else {
        false
    };

    serde_json::json!({
        "status": "ok",
        "trigger": trigger,
        "generated_at": now.to_rfc3339(),
        "created_observations": created_observations,
        "created_proposals": created_proposals,
        "auto_executed": auto_executed,
        "connected_services": connected_services,
        "important_service_events": important_service_events,
        "in_app_events": in_app_events,
        "chat_suggestions": chat_suggestions,
        "daily_review_notice_sent": daily_review_notice_sent,
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
        if enabled {
            settings.sentinel.enable_all_signals();
        } else {
            settings.sentinel.enabled = false;
        }
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
    settings.enforce_dependencies();

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
    let agent_snapshot = Agent::snapshot(&state.agent).await;
    let (storage, ctx, llm) = {
        let agent = &agent_snapshot;
        (
            agent.storage.clone(),
            crate::core::integration_sync::context_from_agent(agent, None),
            agent.llm.clone(),
        )
    };
    let settings = super::load_autonomy_settings_from_storage(&storage).await;
    let mut observations = load_observations(&storage).await;
    let mut proposals = load_proposals(&storage).await;
    let scan = load_scan_state(&storage).await;
    let statuses = crate::core::integration_sync::list_statuses(&ctx).await;
    let feed_items = crate::core::integration_sync::list_feed_items(&ctx, None, 18).await;
    let recent_runs = if settings.sentinel.watch_in_app {
        storage
            .list_recent_execution_runs(40)
            .await
            .map(|runs| runs.len())
            .unwrap_or_default()
    } else {
        0
    };

    let now = chrono::Utc::now();
    refresh_snoozed_proposals(&mut proposals, now);
    if settings.sentinel.watch_in_app
        && settings.sentinel.enabled
        && !settings.agent_paused
        && !settings.autonomy_mode.eq_ignore_ascii_case("off")
    {
        let readiness_policy = crate::core::readiness::load_readiness_policy(&storage).await;
        let mut in_app_batch = build_in_app_candidates(&storage, &settings, now).await;
        if settings.sentinel.infer_new_automations {
            let chat_batch = build_chat_suggestion_candidates(
                &storage,
                &settings,
                &readiness_policy,
                &now.to_rfc3339(),
            )
            .await;
            in_app_batch.extend(chat_batch);
        }
        let _ = reconcile_sentinel_candidates(
            &mut observations,
            &mut proposals,
            in_app_batch,
            &now,
            true,
        );
    }
    if settings.sentinel.enabled
        && proposals
            .iter()
            .filter(|proposal| sentinel_proposal_can_block_duplicate(proposal))
            .take(2)
            .count()
            > 1
    {
        let removed_duplicates = collapse_semantically_equivalent_sentinel_proposals(
            &llm,
            &mut proposals,
            &now.to_rfc3339(),
        )
        .await;
        if removed_duplicates > 0 {
            tracing::info!(
                removed_duplicate_proposals = removed_duplicates,
                "Sentinel feed collapsed semantically duplicate proposals"
            );
        }
    }
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
        in_app_events: observations
            .iter()
            .filter(|item| item.source_kind == "execution_run")
            .count(),
        chat_suggestions: proposals
            .iter()
            .filter(|proposal| sentinel_proposal_is_derived_chat_intent(proposal))
            .filter(|proposal| {
                matches!(
                    proposal.status.as_str(),
                    "open" | "running" | "queued_for_approval" | "snoozed"
                )
            })
            .count(),
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

    if sentinel_proposal_is_chat_continuation_artifact(&proposals[index]) {
        proposals.remove(index);
        save_proposals(&storage, &proposals).await;
        return (
            StatusCode::GONE,
            Json(ErrorResponse {
                error:
                    "This item was an old chat continuation and is no longer surfaced by Sentinel."
                        .to_string(),
            }),
        )
            .into_response();
    }

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

    let result = {
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

pub(super) async fn update_chat_suggestion_proposal_run_state(
    storage: &crate::storage::Storage,
    proposal_id: Option<&str>,
    suggestion_id: &str,
    status: &str,
    run_status: &str,
    trace_id: Option<&str>,
    summary: Option<&str>,
) {
    let suggestion_id = suggestion_id.trim();
    if suggestion_id.is_empty() {
        return;
    }
    let proposal_id = proposal_id.map(str::trim).filter(|value| !value.is_empty());
    let mut proposals = load_proposals(storage).await;
    let now = now_rfc3339();
    let mut changed = false;
    for proposal in proposals.iter_mut() {
        let is_match = proposal_id
            .map(|id| proposal.id == id)
            .unwrap_or_else(|| proposal.chat_suggestion_id.as_deref() == Some(suggestion_id));
        if !is_match {
            continue;
        }
        proposal.status = status.to_string();
        proposal.run_status = Some(run_status.to_string());
        proposal.updated_at = now.clone();
        if proposal.approved_at.is_none() && status != "open" {
            proposal.approved_at = Some(now.clone());
        }
        if let Some(trace_id) = trace_id.map(str::trim).filter(|value| !value.is_empty()) {
            proposal.trace_id = Some(trace_id.to_string());
        }
        if let Some(summary) = summary.map(str::trim).filter(|value| !value.is_empty()) {
            proposal.last_run_summary = Some(summary.to_string());
        }
        changed = true;
    }
    if changed {
        save_proposals(storage, &proposals).await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(id: &str, status: &str, priority: u8, confidence: f32) -> SentinelProposal {
        SentinelProposal {
            id: id.to_string(),
            fingerprint: format!("fingerprint-{id}"),
            proposal_kind: "recommended_action".to_string(),
            status: status.to_string(),
            title: format!("Proposal {id}"),
            detail: format!("Detail {id}"),
            rationale: format!("Rationale {id}"),
            source_kind: "test".to_string(),
            source_id: Some(id.to_string()),
            source_label: Some("Test".to_string()),
            confidence,
            priority,
            created_at: "2026-05-11T00:00:00Z".to_string(),
            updated_at: "2026-05-11T00:00:00Z".to_string(),
            snoozed_until: None,
            approved_at: None,
            dismissed_at: None,
            trace_id: None,
            run_status: None,
            last_run_summary: None,
            action: None,
            chat_suggestion_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    fn execution_run(
        id: &str,
        kind: &str,
        channel: Option<&str>,
        conversation_id: Option<&str>,
        status: crate::core::ExecutionRunStatus,
    ) -> crate::core::ExecutionRun {
        crate::core::ExecutionRun {
            id: id.to_string(),
            kind: kind.to_string(),
            request_id: Some(id.to_string()),
            status: status.clone(),
            current_stage: status.as_str().to_string(),
            lease_owner: None,
            lease_expires_at: None,
            attempt: 0,
            deadline_at: None,
            cancellation_requested: false,
            degradation: Vec::new(),
            last_error: Some("failed".to_string()),
            result_summary: None,
            trace_id: Some(format!("trace-{id}")),
            conversation_id: conversation_id.map(ToString::to_string),
            channel: channel.map(ToString::to_string),
            request_message: Some("background work".to_string()),
            attempted_models: Vec::new(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            updated_at: "2026-05-11T00:00:00Z".to_string(),
        }
    }

    fn chat_suggestion(
        id: &str,
        kind: &str,
        status: &str,
        confidence: f32,
    ) -> super::super::autonomy_support::ChatAutomationSuggestion {
        super::super::autonomy_support::ChatAutomationSuggestion {
            id: id.to_string(),
            status: status.to_string(),
            kind: kind.to_string(),
            title: "Prepare reliable notification workflow".to_string(),
            detail: "Create an approval-gated follow-up for the requested notification."
                .to_string(),
            rationale: "The conversation contains a concrete durable action that can be reviewed."
                .to_string(),
            confidence,
            created_at: "2026-05-11T00:00:00Z".to_string(),
            updated_at: "2026-05-11T00:00:00Z".to_string(),
            conversation_id: format!("conversation-{id}"),
            conversation_title: "Notification planning".to_string(),
            conversation_channel: "web".to_string(),
            source_message_id: format!("message-{id}"),
            source_snippet: "raw user chat should not be present in Sentinel".to_string(),
            fingerprint: format!("fingerprint-{id}"),
            goal_title: "Notification workflow".to_string(),
            goal_detail: Some("Prepare the durable notification follow-up.".to_string()),
            accepted_goal_id: None,
            dismissed_at: None,
            accepted_at: None,
            accepted_trace_id: None,
            run_status: None,
            last_run_error: None,
            last_run_started_at: None,
            last_run_completed_at: None,
            accepted_outcomes: Vec::new(),
            resolution_checked_at: None,
            resolution_check_signature: None,
            resolved_at: None,
            resolution_summary: None,
        }
    }

    #[test]
    fn prune_removes_chat_continuation_artifacts_from_sentinel() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-11T00:01:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut continuation = proposal("chat-suggestion", "open", 4, 0.9);
        continuation.proposal_kind = "chat_suggestion_accept".to_string();
        continuation.source_kind = "chat_suggestion".to_string();
        continuation.chat_suggestion_id = Some("suggestion-1".to_string());

        let retained = prune_proposals(vec![continuation], now);

        assert!(retained.is_empty());
    }

    #[test]
    fn prune_keeps_derived_chat_intent_proposals() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-11T00:01:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut derived = proposal("derived-intent", "open", 4, 0.91);
        derived.proposal_kind = "chat_suggestion_accept".to_string();
        derived.source_kind = "chat_intent".to_string();
        derived.chat_suggestion_id = Some("suggestion-1".to_string());
        derived.metadata = serde_json::json!({
            "derived_from": "autonomy_chat_suggestion",
            "raw_chat_visible": false,
            "tags": ["derived_from_chat", "task"]
        });

        let retained = prune_proposals(vec![derived], now);

        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].source_kind, "chat_intent");
    }

    #[test]
    fn chat_suggestion_candidates_hide_raw_source_snippet() {
        let settings = AutonomySettings::default();
        let readiness_policy = crate::core::ReadinessPolicy::default();
        let suggestion = chat_suggestion("suggestion-1", "task", "open", 0.94);

        let batch = build_chat_suggestion_candidates_from_suggestions(
            &[suggestion],
            &settings,
            &readiness_policy,
            "2026-05-11T00:01:00Z",
        );

        assert_eq!(batch.chat_suggestions, 1);
        assert_eq!(batch.observations.len(), 1);
        assert_eq!(batch.proposals.len(), 1);
        assert_eq!(batch.proposals[0].source_kind, "chat_intent");
        assert_eq!(
            batch.proposals[0].chat_suggestion_id.as_deref(),
            Some("suggestion-1")
        );
        assert!(batch.proposals[0].metadata.get("source_snippet").is_none());
        assert!(batch.observations[0]
            .metadata
            .get("source_snippet")
            .is_none());
        let payload = batch.proposals[0]
            .action
            .as_ref()
            .map(|action| action.payload.clone())
            .unwrap_or_default();
        let payload_text = serde_json::to_string(&payload).unwrap_or_default();
        assert!(!payload_text.contains("raw user chat"));
    }

    #[test]
    fn prune_removes_chat_owned_execution_proposals_without_background_signal() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-11T00:01:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut chat_owned = proposal("chat-owned", "open", 4, 0.9);
        chat_owned.source_kind = "execution_run".to_string();
        chat_owned.metadata = serde_json::json!({
            "conversation_id": "conversation-1",
            "channel": "web",
            "status": "needs_input"
        });
        let mut background = proposal("background", "open", 4, 0.9);
        background.source_kind = "execution_run".to_string();
        background.metadata = serde_json::json!({
            "background_signal": true,
            "channel": "sentinel",
            "status": "platform_failed"
        });

        let retained = prune_proposals(vec![chat_owned, background], now);

        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].id, "background");
    }

    #[test]
    fn chat_owned_execution_runs_are_not_sentinel_background_signals() {
        let chat_run = execution_run(
            "chat-run",
            "chat",
            Some("web"),
            Some("conversation-1"),
            crate::core::ExecutionRunStatus::PlatformFailed,
        );
        let sentinel_owned_run = execution_run(
            "sentinel-run",
            "chat",
            Some("sentinel"),
            None,
            crate::core::ExecutionRunStatus::PlatformFailed,
        );
        let background_run = execution_run(
            "background-run",
            "automation",
            Some("scheduler"),
            Some("origin-conversation"),
            crate::core::ExecutionRunStatus::PlatformFailed,
        );

        assert!(!execution_run_is_detached_background_signal(&chat_run));
        assert!(execution_run_is_detached_background_signal(
            &sentinel_owned_run
        ));
        assert!(execution_run_is_detached_background_signal(&background_run));
    }

    #[test]
    fn background_failures_can_create_sentinel_proposals_without_chat_continuations() {
        let chat_run = execution_run(
            "chat-run",
            "chat",
            Some("web"),
            Some("conversation-1"),
            crate::core::ExecutionRunStatus::PlatformFailed,
        );
        let background_run = execution_run(
            "background-run",
            "automation",
            Some("scheduler"),
            None,
            crate::core::ExecutionRunStatus::PlatformFailed,
        );

        assert!(!should_create_in_app_execution_proposal(
            &chat_run,
            "failed",
            &[],
            execution_run_is_detached_background_signal(&chat_run),
        ));
        assert!(should_create_in_app_execution_proposal(
            &background_run,
            "failed",
            &[],
            execution_run_is_detached_background_signal(&background_run),
        ));
    }

    #[test]
    fn parse_sentinel_proposal_duplicate_groups_ignores_invalid_or_singleton_ids() {
        let valid_ids = ["a".to_string(), "b".to_string(), "c".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        let groups = parse_sentinel_proposal_duplicate_groups(
            r#"prefix {"duplicate_groups":[["a","b","missing"],["c"],["b","a"]]} suffix"#,
            &valid_ids,
        );

        assert_eq!(groups, vec![vec!["a".to_string(), "b".to_string()]]);
    }

    #[test]
    fn apply_sentinel_proposal_duplicate_groups_keeps_most_useful_open_representative() {
        let mut proposals = vec![
            proposal("lower", "open", 3, 0.95),
            proposal("higher", "open", 5, 0.80),
            proposal("distinct", "open", 4, 0.70),
        ];

        let removed = apply_sentinel_proposal_duplicate_groups(
            &mut proposals,
            &[vec!["lower".to_string(), "higher".to_string()]],
            "2026-05-11T00:01:00Z",
        );

        assert_eq!(removed, 1);
        assert!(proposals.iter().any(|item| item.id == "higher"));
        assert!(proposals.iter().any(|item| item.id == "distinct"));
        assert!(!proposals.iter().any(|item| item.id == "lower"));
    }

    #[test]
    fn apply_sentinel_proposal_duplicate_groups_lets_resolved_item_block_open_repeat() {
        let mut proposals = vec![
            proposal("dismissed", "dismissed", 1, 0.20),
            proposal("open", "open", 5, 0.99),
        ];

        let removed = apply_sentinel_proposal_duplicate_groups(
            &mut proposals,
            &[vec!["dismissed".to_string(), "open".to_string()]],
            "2026-05-11T00:01:00Z",
        );

        assert_eq!(removed, 1);
        assert!(proposals.iter().any(|item| item.id == "dismissed"));
        assert!(!proposals.iter().any(|item| item.id == "open"));
    }
}
