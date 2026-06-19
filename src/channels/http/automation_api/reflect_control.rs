//! Reflect retrospective API.

use super::sentinel_panel;
use super::*;

use crate::core::arkorbit::{ArkOrbitService, Orbit, OrbitChatMessage, OrbitChatTranscriptSummary};
use crate::core::{EmbeddingClient, LlmClient, TaskStatus};
use crate::storage::entities::{
    arkpulse_event, conversation, experience_item, experience_run, llm_usage, message,
    procedural_pattern, semantic_work_unit, task,
};
use crate::storage::Storage;
use chrono::{Datelike, TimeZone, Timelike};
use sea_orm::entity::prelude::PgVector;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::MissedTickBehavior;

const REFLECT_MAX_CONVERSATIONS: u64 = 120;
const REFLECT_MAX_MESSAGES_PER_CONVERSATION: u64 = 80;
const REFLECT_MAX_ORBITS: usize = 80;
const REFLECT_MAX_TRANSCRIPTS_PER_ORBIT: usize = 16;
const REFLECT_MAX_EXPERIENCE_ITEMS: u64 = 200;
const REFLECT_MAX_PROCEDURAL_PATTERNS: u64 = 160;
const REFLECT_MAX_TASKS: u64 = 220;
const REFLECT_MAX_WATCHERS: usize = 160;
const REFLECT_MAX_SENTINEL_ITEMS: usize = 120;
const REFLECT_MAX_PULSE_EVENTS: u64 = 160;
const REFLECT_MAX_LINEAGE_ROWS: usize = 160;
const REFLECT_MAX_LLM_USAGE_ROWS: u64 = 4000;
const REFLECT_MAX_UNITS: u64 = 700;
const REFLECT_BASELINE_MAX_UNITS: u64 = 5000;
const REFLECT_MAX_CLUSTERS: usize = 8;
const REFLECT_KMEANS_ROUNDS: usize = 8;
const REFLECT_EMBED_TEXT_CHARS: usize = 16_000;
const REFLECT_PREVIEW_CHARS: usize = 260;
const REFLECT_CACHE_RETENTION_DAYS: i64 = 400;
const REFLECT_EMBED_TIMEOUT: Duration = Duration::from_secs(20);
const REFLECT_DB_TIMEOUT: Duration = Duration::from_secs(12);
const REFLECT_FS_TIMEOUT: Duration = Duration::from_secs(8);
const REFLECT_REFRESH_TIMEOUT: Duration = Duration::from_secs(120);
const REFLECT_CLUSTER_TIMEOUT: Duration = Duration::from_secs(4);
const REFLECT_CLUSTER_QUEUE_TIMEOUT: Duration = Duration::from_millis(250);
const REFLECT_RELATED_HISTORY_TIMEOUT: Duration = Duration::from_millis(500);
const REFLECT_RELATED_HISTORY_TOTAL_TIMEOUT: Duration = Duration::from_secs(2);
const REFLECT_IDLE_INTERVAL: Duration = Duration::from_secs(10 * 60);
const REFLECT_IDLE_LOOKBACK_DAYS: i64 = 35;
const REFLECT_STALE_AFTER_SECS: i64 = 60 * 60;
const REFLECT_REFRESH_LEASE_KEY: &str = "arkreflect_refresh_lease_v1";
const REFLECT_REFRESH_LEASE_TTL_SECS: i64 = 180;
const REFLECT_RELATED_HISTORY_LIMIT: u64 = 8;
const REFLECT_RELATED_HISTORY_DISPLAY_LIMIT: usize = 3;
const REFLECT_RELATED_HISTORY_MAX_DISTANCE: f64 = 0.32;
const REFLECT_BASELINE_LOOKBACK_DAYS: i64 = 183;
const REFLECT_DAILY_DIGEST_STATUS_KEY: &str = "arkreflect_daily_digest_status_v1";
const REFLECT_DAILY_DIGEST_LEASE_KEY: &str = "arkreflect_daily_digest_lease_v1";
const REFLECT_FOLLOWUP_FEEDBACK_KEY: &str = "arkreflect_followup_feedback_v1";
const REFLECT_DAILY_DIGEST_LEASE_TTL_SECS: i64 = 180;
const REFLECT_DAILY_DIGEST_TIMEOUT: Duration = Duration::from_secs(35);
const REFLECT_DAILY_DIGEST_NOT_BEFORE_LOCAL_HOUR: u32 = 20;
const REFLECT_MAX_SUGGESTED_FOLLOWUPS: usize = 5;
const REFLECT_SUGGESTION_EXPERIENCE_RUN_LIMIT: u64 = 160;
const REFLECT_SUGGESTION_TEXT_CHARS: usize = 220;
const REFLECT_FOLLOWUP_SEARCH_CACHE_KEY: &str = "arkreflect_followup_search_cache_v1";
const REFLECT_FOLLOWUP_PLAN_CACHE_KEY: &str = "arkreflect_followup_plan_cache_v1";
const REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_KEY: &str = "arkreflect_followup_cache_write_lease_v1";
const REFLECT_FOLLOWUP_SEARCH_LEASE_KEY: &str = "arkreflect_followup_search_lease_v1";
const REFLECT_FOLLOWUP_SUMMARY_LEASE_KEY: &str = "arkreflect_followup_summary_lease_v1";
const REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_TTL_SECS: i64 = 60;
const REFLECT_FOLLOWUP_SEARCH_LEASE_TTL_SECS: i64 = 480;
const REFLECT_FOLLOWUP_SUMMARY_LEASE_TTL_SECS: i64 = 480;
const REFLECT_FOLLOWUP_SEARCH_TIMEOUT: Duration = Duration::from_secs(75);
const REFLECT_FOLLOWUP_SUMMARY_JOB_TIMEOUT: Duration = Duration::from_secs(420);
const REFLECT_FOLLOWUP_SEARCH_DUE_AFTER_SECS: i64 = 24 * 60 * 60;
const REFLECT_FOLLOWUP_SEARCH_RESULTS_PER_TOPIC: usize = 3;
const REFLECT_FOLLOWUP_BACKGROUND_SEARCH_LIMIT: usize = 3;
const REFLECT_FOLLOWUP_PLAN_LIMIT: usize = 8;
const REFLECT_FOLLOWUP_PLAN_RETENTION_DAYS: i64 = 120;
const REFLECT_FOLLOWUP_PLAN_NEGATIVE_RETENTION_DAYS: i64 = 7;
const REFLECT_FOLLOWUP_PLAN_TOPIC_CHARS: usize = 600;
const REFLECT_FOLLOWUP_PLAN_TIMEOUT: Duration = Duration::from_secs(120);
const REFLECT_FOLLOWUP_SUMMARY_TIMEOUT: Duration = Duration::from_secs(120);
const REFLECT_FOLLOWUP_SUMMARY_MAX_CHARS: usize = 1_000;
const REFLECT_SEMANTIC_FRESHNESS_TIMEOUT: Duration = Duration::from_secs(5);
const REFLECT_SEMANTIC_FRESHNESS_MIN_SIMILARITY: f32 = 0.28;
const REFLECT_SEMANTIC_FRESHNESS_MIN_MARGIN: f32 = 0.04;
const REFLECT_FEEDBACK_SEMANTIC_MAX_DISTANCE: f32 = 0.18;
const REFLECT_FOLLOWUP_TEXT_DUPLICATE_THRESHOLD: f64 = 0.56;
const REFLECT_REFRESH_DUPLICATE_SUPPRESS_SECS: i64 = 15 * 60;
const REFLECT_PUBLIC_DEVELOPMENT_CONCEPT_TEXT: &str = "A reflected topic about public events, external entities, products, places, services, regulations, markets, research, releases, or other outside-world information whose useful answer depends on current source evidence.";
const REFLECT_PRIVATE_WORK_CONCEPT_TEXT: &str = "A reflected local or personal topic about private profile facts, identity, location, preferences, saved memories, internal notes, app UI, code edits, workflow state, system maintenance, or other user-specific context that should continue without public source research.";

static REFLECT_REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static REFLECT_IDLE_LOOP_STARTED: AtomicBool = AtomicBool::new(false);
static REFLECT_FOLLOWUP_COORDINATOR_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static REFLECT_FOLLOWUP_SEARCH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static REFLECT_FOLLOWUP_SUMMARY_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static REFLECT_REFRESH_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REFLECT_REFRESH_STATUS: OnceLock<Arc<RwLock<ReflectRefreshStatus>>> = OnceLock::new();
static REFLECT_CLUSTER_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();
static REFLECT_FOLLOWUP_PLANNING_TELEMETRY: OnceLock<
    Arc<RwLock<ReflectFollowupPlanningTelemetry>>,
> = OnceLock::new();
static REFLECT_FOLLOWUP_PLANNING_ACTIVE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ReflectPeriod {
    Daily,
    Weekly,
    Monthly,
}

impl ReflectPeriod {
    fn from_query(value: Option<&str>) -> Self {
        match value
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "daily" | "day" => Self::Daily,
            "monthly" | "month" => Self::Monthly,
            _ => Self::Weekly,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    fn default_window(
        self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
        match self {
            Self::Daily => (now - chrono::Duration::days(1), now),
            Self::Weekly => (now - chrono::Duration::days(7), now),
            Self::Monthly => (now - chrono::Duration::days(31), now),
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct ReflectUnitResponse {
    id: String,
    source_kind: String,
    source_label: String,
    channel: String,
    title: String,
    summary: String,
    content_preview: String,
    occurred_at: String,
    message_count: i32,
    has_embedding: bool,
    metadata: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct ReflectRelatedUnitResponse {
    id: String,
    source_label: String,
    title: String,
    occurred_at: String,
    similarity: f64,
}

#[derive(Debug, serde::Serialize)]
struct ReflectRelatedHistory {
    mode: String,
    similar_count: usize,
    most_recent_at: Option<String>,
    top_similarity: Option<f64>,
    detail: String,
    items: Vec<ReflectRelatedUnitResponse>,
}

impl ReflectRelatedHistory {
    fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            mode: "unavailable".to_string(),
            similar_count: 0,
            most_recent_at: None,
            top_similarity: None,
            detail: detail.into(),
            items: Vec::new(),
        }
    }

    fn new_this_period() -> Self {
        Self {
            mode: "new".to_string(),
            similar_count: 0,
            most_recent_at: None,
            top_similarity: None,
            detail: "No close match was found in earlier or later reflection history.".to_string(),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct ReflectClusterResponse {
    id: String,
    #[serde(skip)]
    representative_unit_id: String,
    #[serde(skip)]
    centroid_embedding: Option<PgVector>,
    label: String,
    plain_summary: String,
    unit_count: usize,
    message_count: i32,
    source_mix: BTreeMap<String, usize>,
    color: String,
    related_history: ReflectRelatedHistory,
    units: Vec<ReflectUnitResponse>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct ReflectSourceCounts {
    main_chat: usize,
    orbit_chat: usize,
    memory: usize,
    procedures: usize,
    apps: usize,
    goals: usize,
    watchers: usize,
    sentinel: usize,
    arkpulse: usize,
    arkevolve: usize,
    usage: usize,
}

#[derive(Debug, serde::Serialize)]
struct ReflectEmbeddingStatus {
    mode: String,
    embedded_units: usize,
    total_units: usize,
    detail: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ReflectRefreshStatus {
    running: bool,
    status: String,
    trigger: Option<String>,
    period: Option<String>,
    from: Option<String>,
    to: Option<String>,
    requested_at: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    last_error: Option<String>,
    last_source_counts: ReflectSourceCounts,
    sequence: u64,
}

impl Default for ReflectRefreshStatus {
    fn default() -> Self {
        Self {
            running: false,
            status: "idle".to_string(),
            trigger: None,
            period: None,
            from: None,
            to: None,
            requested_at: None,
            started_at: None,
            completed_at: None,
            last_error: None,
            last_source_counts: ReflectSourceCounts::default(),
            sequence: 0,
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct ReflectCacheStatus {
    mode: String,
    cached_units: usize,
    stale: bool,
    detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReflectDigestDeliveryAttempt {
    channel: String,
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReflectDailyDigestStatus {
    enabled: bool,
    status: String,
    target_date: String,
    today_date: String,
    meaningful: bool,
    unit_count: usize,
    cluster_count: usize,
    source_counts: ReflectSourceCounts,
    summary: Option<String>,
    detail: String,
    last_checked_at: Option<String>,
    last_sent_at: Option<String>,
    last_skipped_at: Option<String>,
    last_error: Option<String>,
    delivery_attempts: Vec<ReflectDigestDeliveryAttempt>,
}

impl ReflectDailyDigestStatus {
    fn disabled(today_date: String) -> Self {
        Self {
            enabled: false,
            status: "disabled".to_string(),
            target_date: today_date.clone(),
            today_date,
            meaningful: false,
            unit_count: 0,
            cluster_count: 0,
            source_counts: ReflectSourceCounts::default(),
            summary: None,
            detail: "Daily Reflect digest delivery is off.".to_string(),
            last_checked_at: None,
            last_sent_at: None,
            last_skipped_at: None,
            last_error: None,
            delivery_attempts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct ReflectSuggestedFollowup {
    id: String,
    kind: String,
    title: String,
    detail: String,
    prompt: String,
    status: String,
    source_label: String,
    occurred_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_unit_id: Option<String>,
    rank_score: f64,
    #[serde(default)]
    search_results: Vec<ReflectFollowupSearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_summary_generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_summary_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_summary_evidence_supported: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feedback: Option<ReflectFollowupFeedbackState>,
    #[serde(default)]
    feedback_keys: Vec<String>,
    #[serde(skip_serializing)]
    feedback_vector: Option<Vec<f32>>,
    #[serde(skip_serializing)]
    search_query: Option<String>,
    #[serde(skip_serializing)]
    search_planning_context: Option<String>,
    #[serde(skip_serializing)]
    search_requires_planning: bool,
    #[serde(skip_serializing)]
    source_strategy: ReflectFollowupSourceStrategy,
    #[serde(skip_serializing)]
    structured_context: serde_json::Value,
    #[serde(skip_serializing)]
    allow_unplanned_source_check: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupFeedbackState {
    #[serde(default)]
    useful_count: u32,
    #[serde(default)]
    dismiss_count: u32,
    #[serde(default)]
    snooze_count: u32,
    last_action: Option<String>,
    last_at: Option<String>,
    snoozed_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_vector: Option<Vec<f32>>,
    #[serde(default)]
    renewed_after_feedback: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupFeedbackStore {
    updated_at: Option<String>,
    #[serde(default)]
    entries: BTreeMap<String, ReflectFollowupFeedbackState>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct ReflectFollowupFeedbackRequest {
    action: ReflectFollowupFeedbackAction,
    #[serde(default)]
    keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReflectFollowupFeedbackAction {
    Useful,
    Dismiss,
    Snooze,
}

impl ReflectFollowupFeedbackAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Useful => "useful",
            Self::Dismiss => "dismiss",
            Self::Snooze => "snooze",
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct ReflectFollowupFeedbackResponse {
    status: String,
    id: String,
    feedback: ReflectFollowupFeedbackState,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupSearchCache {
    updated_at: Option<String>,
    #[serde(default)]
    entries: BTreeMap<String, ReflectFollowupSearchEntry>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupSearchEntry {
    source_id: String,
    query: String,
    checked_at: String,
    backend: Option<String>,
    #[serde(default)]
    source_strategy: ReflectFollowupSourceStrategy,
    #[serde(default)]
    structured_context: serde_json::Value,
    #[serde(default)]
    results: Vec<ReflectFollowupSearchResult>,
    error: Option<String>,
    summary: Option<String>,
    summary_generated_at: Option<String>,
    summary_error: Option<String>,
    #[serde(default)]
    summary_evidence_supported: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupPlanCache {
    updated_at: Option<String>,
    #[serde(default)]
    entries: BTreeMap<String, ReflectFollowupPlanEntry>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupPlanEntry {
    id: String,
    #[serde(default)]
    useful: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    search_query: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    source_strategy: ReflectFollowupSourceStrategy,
    #[serde(default)]
    structured_context: serde_json::Value,
    #[serde(default)]
    planned_at: String,
    #[serde(default)]
    topic: String,
}

#[derive(Debug, Clone, Default)]
struct ReflectFollowupPlanningTelemetry {
    last_attempt_at: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct ReflectFollowupPlanningStatus {
    pending_count: usize,
    planning_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_attempt_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

struct ReflectFollowupBuild {
    followups: Vec<ReflectSuggestedFollowup>,
    unplanned: Vec<ReflectSuggestedFollowup>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReflectFollowupSearchResult {
    title: String,
    url: String,
    snippet: String,
    source: String,
    published_date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReflectFollowupSourceStrategy {
    #[default]
    #[serde(alias = "generic_current_sources")]
    PublicSearch,
    FlightPriceDiscovery,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ReflectExternalPursuitPlan {
    id: String,
    #[serde(default)]
    useful: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    search_query: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    source_strategy: ReflectFollowupSourceStrategy,
    #[serde(default)]
    structured_context: serde_json::Value,
}

struct ReflectDueFollowupWork {
    searches: Vec<ReflectDueFollowupSearch>,
    summaries: Vec<String>,
}

#[derive(Debug, Clone)]
struct ReflectDueFollowupSearch {
    source_id: String,
    query: String,
    source_strategy: ReflectFollowupSourceStrategy,
    structured_context: serde_json::Value,
}

impl ReflectDueFollowupWork {
    fn has_work(&self) -> bool {
        !self.searches.is_empty() || !self.summaries.is_empty()
    }
}

#[derive(Debug, Clone)]
struct ReflectSemanticFreshnessContext {
    public_development: Vec<f32>,
    private_work: Vec<f32>,
    dimension: usize,
}

#[derive(Debug, Clone, Copy)]
struct ReflectSemanticFreshnessScore {
    similarity: f32,
    margin: f32,
}

#[derive(Debug, serde::Serialize)]
struct ReflectResponse {
    period: ReflectPeriod,
    from: String,
    to: String,
    generated_at: String,
    source_counts: ReflectSourceCounts,
    baseline_source_counts: ReflectSourceCounts,
    embedding_status: ReflectEmbeddingStatus,
    refresh_status: ReflectRefreshStatus,
    cache_status: ReflectCacheStatus,
    daily_digest_status: ReflectDailyDigestStatus,
    suggested_followups: Vec<ReflectSuggestedFollowup>,
    followup_planning: ReflectFollowupPlanningStatus,
    clusters: Vec<ReflectClusterResponse>,
    unclustered_units: Vec<ReflectUnitResponse>,
}

#[derive(Debug, serde::Serialize)]
struct ReflectRefreshStartResponse {
    accepted: bool,
    running: bool,
    status: String,
    detail: String,
    refresh_status: ReflectRefreshStatus,
}

#[derive(Debug, Clone)]
struct ReflectCandidateUnit {
    source_kind: String,
    source_id: String,
    conversation_id: Option<String>,
    project_id: Option<String>,
    channel: String,
    title: String,
    summary: String,
    content_preview: String,
    embedding_text: String,
    occurred_at: String,
    period_start: Option<String>,
    period_end: Option<String>,
    message_count: i32,
    metadata: serde_json::Value,
    inherited_embedding: Option<PgVector>,
}

#[derive(Debug, Clone)]
struct ReflectRefreshRequest {
    period: ReflectPeriod,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
}

struct ReflectInFlightGuard;

impl Drop for ReflectInFlightGuard {
    fn drop(&mut self) {
        REFLECT_REFRESH_IN_FLIGHT.store(false, Ordering::Release);
    }
}

struct ReflectFollowupSearchInFlightGuard;

impl Drop for ReflectFollowupSearchInFlightGuard {
    fn drop(&mut self) {
        REFLECT_FOLLOWUP_SEARCH_IN_FLIGHT.store(false, Ordering::Release);
    }
}

struct ReflectFollowupSummaryInFlightGuard;

impl Drop for ReflectFollowupSummaryInFlightGuard {
    fn drop(&mut self) {
        REFLECT_FOLLOWUP_SUMMARY_IN_FLIGHT.store(false, Ordering::Release);
    }
}

struct ReflectFollowupCoordinatorInFlightGuard;

impl Drop for ReflectFollowupCoordinatorInFlightGuard {
    fn drop(&mut self) {
        REFLECT_FOLLOWUP_COORDINATOR_IN_FLIGHT.store(false, Ordering::Release);
    }
}

/// Counts in-flight LLM planning passes specifically, unlike the coordinator
/// flag which also covers search/summary coordination.
struct ReflectFollowupPlanningActiveGuard;

impl ReflectFollowupPlanningActiveGuard {
    fn new() -> Self {
        REFLECT_FOLLOWUP_PLANNING_ACTIVE.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ReflectFollowupPlanningActiveGuard {
    fn drop(&mut self) {
        REFLECT_FOLLOWUP_PLANNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

fn refresh_status_store() -> &'static Arc<RwLock<ReflectRefreshStatus>> {
    REFLECT_REFRESH_STATUS.get_or_init(|| Arc::new(RwLock::new(ReflectRefreshStatus::default())))
}

async fn current_refresh_status() -> ReflectRefreshStatus {
    refresh_status_store().read().await.clone()
}

fn planning_telemetry_store() -> &'static Arc<RwLock<ReflectFollowupPlanningTelemetry>> {
    REFLECT_FOLLOWUP_PLANNING_TELEMETRY
        .get_or_init(|| Arc::new(RwLock::new(ReflectFollowupPlanningTelemetry::default())))
}

async fn record_reflect_planning_attempt(error: Option<String>) {
    let mut telemetry = planning_telemetry_store().write().await;
    telemetry.last_attempt_at = Some(chrono::Utc::now().to_rfc3339());
    telemetry.last_error = error;
}

async fn reflect_followup_planning_status(pending_count: usize) -> ReflectFollowupPlanningStatus {
    let telemetry = planning_telemetry_store().read().await.clone();
    let planning_active = REFLECT_FOLLOWUP_PLANNING_ACTIVE.load(Ordering::Acquire) > 0;
    // A stale error is meaningless once nothing is pending: nothing will
    // retry, and the empty state should not read as a failure.
    let last_error = if pending_count == 0 && !planning_active {
        None
    } else {
        telemetry.last_error
    };
    ReflectFollowupPlanningStatus {
        pending_count,
        planning_active,
        last_attempt_at: telemetry.last_attempt_at,
        last_error,
    }
}

async fn update_refresh_status(
    update: impl FnOnce(&mut ReflectRefreshStatus),
) -> ReflectRefreshStatus {
    let mut status = refresh_status_store().write().await;
    update(&mut status);
    status.clone()
}

fn reflect_refresh_status_matches_request(
    status: &ReflectRefreshStatus,
    request: &ReflectRefreshRequest,
) -> bool {
    let from = request.from.to_rfc3339();
    let to = request.to.to_rfc3339();
    status.period.as_deref() == Some(request.period.as_str())
        && status.from.as_deref() == Some(from.as_str())
        && status.to.as_deref() == Some(to.as_str())
}

fn reflect_refresh_recently_completed_for_request(
    status: &ReflectRefreshStatus,
    request: &ReflectRefreshRequest,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if status.running
        || status.status != "completed"
        || !reflect_refresh_status_matches_request(status, request)
    {
        return false;
    }
    status
        .completed_at
        .as_deref()
        .and_then(parse_time)
        .is_some_and(|completed_at| {
            (now - completed_at).num_seconds() < REFLECT_REFRESH_DUPLICATE_SUPPRESS_SECS
        })
}

fn parse_time(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn reflect_timezone_from_profile(profile: &UserProfile) -> Option<chrono_tz::Tz> {
    profile
        .timezone
        .as_deref()
        .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
}

fn reflect_local_date(
    at: chrono::DateTime<chrono::Utc>,
    tz: Option<chrono_tz::Tz>,
) -> chrono::NaiveDate {
    match tz {
        Some(tz) => at.with_timezone(&tz).date_naive(),
        None => at.date_naive(),
    }
}

fn reflect_local_hour(at: chrono::DateTime<chrono::Utc>, tz: Option<chrono_tz::Tz>) -> u32 {
    match tz {
        Some(tz) => at.with_timezone(&tz).hour(),
        None => at.hour(),
    }
}

fn reflect_local_midnight_utc(
    date: chrono::NaiveDate,
    tz: Option<chrono_tz::Tz>,
) -> chrono::DateTime<chrono::Utc> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .unwrap_or_else(|| chrono::Utc::now().naive_utc());
    match tz {
        Some(tz) => match tz.from_local_datetime(&naive) {
            chrono::LocalResult::Single(value) => value.with_timezone(&chrono::Utc),
            chrono::LocalResult::Ambiguous(first, _) => first.with_timezone(&chrono::Utc),
            chrono::LocalResult::None => chrono::Utc.from_utc_datetime(&naive),
        },
        None => chrono::Utc.from_utc_datetime(&naive),
    }
}

fn reflect_daily_window_for_date(
    date: chrono::NaiveDate,
    tz: Option<chrono_tz::Tz>,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let from = reflect_local_midnight_utc(date, tz);
    let next_date = date
        .succ_opt()
        .unwrap_or_else(|| date + chrono::Duration::days(1));
    let to = reflect_local_midnight_utc(next_date, tz);
    (from, to)
}

fn reflect_digest_target_date(
    now: chrono::DateTime<chrono::Utc>,
    tz: Option<chrono_tz::Tz>,
) -> chrono::NaiveDate {
    let today = reflect_local_date(now, tz);
    if reflect_local_hour(now, tz) >= REFLECT_DAILY_DIGEST_NOT_BEFORE_LOCAL_HOUR {
        today
    } else {
        today
            .pred_opt()
            .unwrap_or_else(|| today - chrono::Duration::days(1))
    }
}

fn query_time(
    params: &HashMap<String, String>,
    key: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    params.get(key).and_then(|value| parse_time(value))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn first_non_empty<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    values
        .into_iter()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("Untitled work")
        .to_string()
}

fn stable_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

fn stable_unit_id(source_kind: &str, source_id: &str) -> String {
    format!(
        "reflect-{}",
        stable_hash(&format!("{}:{}", source_kind, source_id))
            .chars()
            .take(32)
            .collect::<String>()
    )
}

fn in_window(
    value: &str,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> bool {
    parse_time(value)
        .map(|dt| dt >= from && dt < to)
        .unwrap_or(false)
}

fn day_key(value: &str) -> Option<String> {
    parse_time(value).map(|dt| dt.format("%Y-%m-%d").to_string())
}

fn source_label(source_kind: &str, channel: &str) -> String {
    match source_kind {
        "conversation" => "Chat".to_string(),
        "orbit_chat" => "Orbit".to_string(),
        "experience_item" => "Memory".to_string(),
        "procedural_pattern" => "Learned workflow".to_string(),
        "app" => "Apps".to_string(),
        "goal" => "Goals".to_string(),
        "watcher" => "Watchers".to_string(),
        "sentinel" => "Sentinel".to_string(),
        "arkpulse" => "Pulse".to_string(),
        "arkevolve" => "Evolve".to_string(),
        "llm_usage" => "Usage".to_string(),
        _ if !channel.trim().is_empty() => channel.trim().to_string(),
        _ => "Work".to_string(),
    }
}

fn increment_source_count(counts: &mut ReflectSourceCounts, source_kind: &str) {
    match source_kind {
        "conversation" => counts.main_chat += 1,
        "orbit_chat" => counts.orbit_chat += 1,
        "experience_item" => counts.memory += 1,
        "procedural_pattern" => counts.procedures += 1,
        "app" => counts.apps += 1,
        "goal" => counts.goals += 1,
        "watcher" => counts.watchers += 1,
        "sentinel" => counts.sentinel += 1,
        "arkpulse" => counts.arkpulse += 1,
        "arkevolve" => counts.arkevolve += 1,
        "llm_usage" => counts.usage += 1,
        _ => {}
    }
}

fn source_counts_from_units(units: &[semantic_work_unit::Model]) -> ReflectSourceCounts {
    let mut counts = ReflectSourceCounts::default();
    for unit in units {
        increment_source_count(&mut counts, &unit.source_kind);
    }
    counts
}

fn total_source_count(counts: &ReflectSourceCounts) -> usize {
    counts.main_chat
        + counts.orbit_chat
        + counts.memory
        + counts.procedures
        + counts.apps
        + counts.goals
        + counts.watchers
        + counts.sentinel
        + counts.arkpulse
        + counts.arkevolve
        + counts.usage
}

fn meaningful_source_count(counts: &ReflectSourceCounts) -> usize {
    total_source_count(counts).saturating_sub(counts.usage)
}

fn background_source_count(counts: &ReflectSourceCounts) -> usize {
    counts.memory
        + counts.procedures
        + counts.apps
        + counts.goals
        + counts.watchers
        + counts.sentinel
        + counts.arkpulse
        + counts.arkevolve
}

fn reflect_activity_is_meaningful(
    counts: &ReflectSourceCounts,
    units: &[semantic_work_unit::Model],
    clusters: &[ReflectClusterResponse],
) -> bool {
    if background_source_count(counts) > 0 {
        return true;
    }
    if meaningful_source_count(counts) >= 2 {
        return true;
    }
    if clusters.len() >= 2 {
        return true;
    }
    let conversational_messages: i32 = units
        .iter()
        .filter(|unit| matches!(unit.source_kind.as_str(), "conversation" | "orbit_chat"))
        .map(|unit| unit.message_count.max(0))
        .sum();
    conversational_messages >= 4
}

fn unit_to_response(unit: &semantic_work_unit::Model) -> ReflectUnitResponse {
    ReflectUnitResponse {
        id: unit.id.clone(),
        source_kind: unit.source_kind.clone(),
        source_label: source_label(&unit.source_kind, &unit.channel),
        channel: unit.channel.clone(),
        title: unit.title.clone(),
        summary: unit.summary.clone(),
        content_preview: unit.content_preview.clone(),
        occurred_at: unit.occurred_at.clone(),
        message_count: unit.message_count,
        has_embedding: unit.embedding.is_some(),
        metadata: unit.metadata.clone(),
    }
}

fn reflect_sentence_fragment(value: &str, max_chars: usize) -> String {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&cleaned, max_chars)
}

fn reflect_label_from_identifier(value: &str) -> String {
    let mut label = String::new();
    let mut capitalize = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize {
                label.push(ch.to_ascii_uppercase());
                capitalize = false;
            } else {
                label.push(ch.to_ascii_lowercase());
            }
        } else if !label.ends_with(' ') && !label.is_empty() {
            label.push(' ');
            capitalize = true;
        } else {
            capitalize = true;
        }
    }
    let label = label.trim();
    if label.is_empty() {
        "Work".to_string()
    } else {
        label.to_string()
    }
}

fn reflect_recency_score(occurred_at: &str, now: chrono::DateTime<chrono::Utc>) -> f64 {
    parse_time(occurred_at)
        .map(|dt| {
            let age_days = (now - dt).num_days().clamp(0, 30) as f64;
            30.0 - age_days
        })
        .unwrap_or(0.0)
}

fn reflect_experience_run_topic(run: &experience_run::Model) -> String {
    first_non_empty([
        run.request_text.as_deref().unwrap_or_default(),
        run.outcome_summary.as_deref().unwrap_or_default(),
        run.failure_reason.as_deref().unwrap_or_default(),
        run.intent_key.as_str(),
    ])
}

fn reflect_experience_run_failed(run: &experience_run::Model) -> bool {
    run.success_state == "failed" || run.correction_state == "corrected"
}

fn reflect_tool_name_is_research(name: &str) -> bool {
    matches!(name, "research" | "web_search" | "page_fetch") || name.starts_with("browser_")
}

fn reflect_experience_run_is_research(run: &experience_run::Model) -> bool {
    if run
        .task_type
        .as_deref()
        .map(|task_type| task_type == "research")
        .unwrap_or(false)
    {
        return true;
    }
    run.tool_sequence_json
        .as_array()
        .map(|items| {
            items.iter().any(|item| {
                item.get("name")
                    .or_else(|| item.get("tool_name"))
                    .and_then(|value| value.as_str())
                    .is_some_and(reflect_tool_name_is_research)
            })
        })
        .unwrap_or(false)
}

fn reflect_search_cache_entry<'a>(
    cache: &'a ReflectFollowupSearchCache,
    source_id: &str,
) -> Option<&'a ReflectFollowupSearchEntry> {
    cache.entries.get(source_id)
}

fn reflect_compact_search_query(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn reflect_cache_query_without_current_year(value: &str) -> String {
    let compact = reflect_compact_search_query(value).to_ascii_lowercase();
    let current_year = chrono::Utc::now().year().to_string();
    let mut parts = compact.split_whitespace().collect::<Vec<_>>();
    if parts.last().is_some_and(|part| *part == current_year) {
        parts.pop();
    }
    parts.join(" ")
}

fn reflect_search_queries_match_for_cache(left: &str, right: &str) -> bool {
    let left = reflect_compact_search_query(left);
    let right = reflect_compact_search_query(right);
    if left.eq_ignore_ascii_case(&right) {
        return true;
    }
    reflect_cache_query_without_current_year(&left)
        == reflect_cache_query_without_current_year(&right)
}

fn reflect_followup_search_is_due(
    cache: &ReflectFollowupSearchCache,
    source_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let Some(entry) = reflect_search_cache_entry(cache, source_id) else {
        return true;
    };
    if entry.checked_at.trim().is_empty() {
        return true;
    }
    parse_time(&entry.checked_at)
        .map(|checked_at| {
            (now - checked_at).num_seconds() >= REFLECT_FOLLOWUP_SEARCH_DUE_AFTER_SECS
        })
        .unwrap_or(true)
}

fn reflect_followup_summary_is_due(entry: &ReflectFollowupSearchEntry) -> bool {
    if entry.results.is_empty() {
        return false;
    }
    if entry
        .error
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return false;
    }
    let has_summary = entry
        .summary
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_summary_error = entry
        .summary_error
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if has_summary {
        return entry.summary_evidence_supported != Some(true);
    }
    if entry.summary_evidence_supported == Some(false) {
        return false;
    }
    if has_summary_error {
        let has_summary_attempt_at = entry
            .summary_generated_at
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        return !has_summary_attempt_at;
    }
    !has_summary && !has_summary_error
}

fn reflect_followup_latest_summary_fields(
    entry: Option<&ReflectFollowupSearchEntry>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(entry) = entry else {
        return (None, None, None);
    };
    let error = entry
        .summary_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if entry.summary_evidence_supported != Some(true) {
        return (None, None, error);
    }
    let summary = entry
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let generated_at = entry
        .summary_generated_at
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (summary, generated_at, error)
}

async fn load_reflect_followup_feedback(storage: &Storage) -> ReflectFollowupFeedbackStore {
    storage
        .get(REFLECT_FOLLOWUP_FEEDBACK_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<ReflectFollowupFeedbackStore>(&bytes).ok())
        .unwrap_or_default()
}

async fn save_reflect_followup_feedback(storage: &Storage, store: &ReflectFollowupFeedbackStore) {
    if let Ok(bytes) = serde_json::to_vec(store) {
        if let Err(error) = storage.set(REFLECT_FOLLOWUP_FEEDBACK_KEY, &bytes).await {
            tracing::warn!(error = %error, "failed to save followup feedback");
        }
    }
}

fn reflect_followup_feedback_weight(feedback: Option<&ReflectFollowupFeedbackState>) -> f64 {
    let Some(feedback) = feedback else {
        return 0.0;
    };
    let mut weight = feedback.useful_count as f64 * 9.0
        - feedback.dismiss_count as f64 * 16.0
        - feedback.snooze_count as f64 * 5.0;
    if feedback.dismiss_count >= 2 {
        weight -= 32.0;
    }
    if feedback.dismiss_count >= 1 && feedback.snooze_count >= 2 {
        weight -= 48.0;
    }
    weight
}

fn reflect_followup_feedback_keys<'a>(
    id: &'a str,
    source_unit_id: Option<&'a str>,
    conversation_id: Option<&'a str>,
) -> Vec<String> {
    let mut keys = Vec::new();
    for key in [
        Some(format!("followup:{}", id)),
        source_unit_id
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("unit:{}", value.trim())),
        conversation_id
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("conversation:{}", value.trim())),
    ]
    .into_iter()
    .flatten()
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn reflect_followup_effective_feedback(
    keys: &[String],
    store: &ReflectFollowupFeedbackStore,
) -> Option<ReflectFollowupFeedbackState> {
    let mut merged = ReflectFollowupFeedbackState::default();
    let mut seen = false;
    for key in keys {
        let Some(feedback) = store.entries.get(key) else {
            continue;
        };
        seen = true;
        merged.useful_count = merged.useful_count.saturating_add(feedback.useful_count);
        merged.dismiss_count = merged.dismiss_count.saturating_add(feedback.dismiss_count);
        merged.snooze_count = merged.snooze_count.saturating_add(feedback.snooze_count);
        if feedback
            .last_at
            .as_deref()
            .zip(merged.last_at.as_deref())
            .map(|(candidate, current)| candidate > current)
            .unwrap_or(merged.last_at.is_none())
        {
            merged.last_action = feedback.last_action.clone();
            merged.last_at = feedback.last_at.clone();
        }
        let candidate_snooze = feedback.snoozed_until.as_deref();
        let current_snooze = merged.snoozed_until.as_deref();
        if candidate_snooze
            .zip(current_snooze)
            .map(|(candidate, current)| candidate > current)
            .unwrap_or(current_snooze.is_none() && candidate_snooze.is_some())
        {
            merged.snoozed_until = feedback.snoozed_until.clone();
        }
        if merged.semantic_vector.is_none() {
            merged.semantic_vector = feedback.semantic_vector.clone();
        }
    }
    seen.then_some(merged)
}

fn reflect_followup_semantic_feedback(
    vector: Option<&[f32]>,
    store: &ReflectFollowupFeedbackStore,
) -> Option<ReflectFollowupFeedbackState> {
    let vector = vector?;
    let mut merged = ReflectFollowupFeedbackState::default();
    let mut seen = false;
    for feedback in store.entries.values() {
        let Some(stored_vector) = feedback.semantic_vector.as_deref() else {
            continue;
        };
        if stored_vector.len() != vector.len()
            || cosine_distance(vector, stored_vector) > REFLECT_FEEDBACK_SEMANTIC_MAX_DISTANCE
        {
            continue;
        }
        seen = true;
        merged.useful_count = merged.useful_count.saturating_add(feedback.useful_count);
        merged.dismiss_count = merged.dismiss_count.saturating_add(feedback.dismiss_count);
        merged.snooze_count = merged.snooze_count.saturating_add(feedback.snooze_count);
        if feedback
            .last_at
            .as_deref()
            .zip(merged.last_at.as_deref())
            .map(|(candidate, current)| candidate > current)
            .unwrap_or(merged.last_at.is_none())
        {
            merged.last_action = feedback.last_action.clone();
            merged.last_at = feedback.last_at.clone();
        }
        let candidate_snooze = feedback.snoozed_until.as_deref();
        let current_snooze = merged.snoozed_until.as_deref();
        if candidate_snooze
            .zip(current_snooze)
            .map(|(candidate, current)| candidate > current)
            .unwrap_or(current_snooze.is_none() && candidate_snooze.is_some())
        {
            merged.snoozed_until = feedback.snoozed_until.clone();
        }
    }
    seen.then_some(merged)
}

fn reflect_feedback_for_response(
    feedback: Option<ReflectFollowupFeedbackState>,
) -> Option<ReflectFollowupFeedbackState> {
    feedback.map(|mut state| {
        state.semantic_vector = None;
        state
    })
}

fn reflect_followup_refresh_feedback_for_new_evidence(
    feedback: Option<ReflectFollowupFeedbackState>,
    occurred_at: &str,
) -> Option<ReflectFollowupFeedbackState> {
    feedback.map(|mut state| {
        let renewed = state
            .last_at
            .as_deref()
            .and_then(parse_time)
            .zip(parse_time(occurred_at))
            .is_some_and(|(last_feedback, evidence_at)| evidence_at > last_feedback);
        if renewed {
            state.renewed_after_feedback = true;
            state.snoozed_until = None;
        }
        state
    })
}

fn reflect_register_followup_feedback_vectors(
    store: &mut ReflectFollowupFeedbackStore,
    candidates: &[ReflectSuggestedFollowup],
) -> bool {
    let mut changed = false;
    for candidate in candidates {
        let Some(vector) = candidate.feedback_vector.as_ref() else {
            continue;
        };
        for key in &candidate.feedback_keys {
            let entry = store.entries.entry(key.clone()).or_default();
            let needs_update = entry
                .semantic_vector
                .as_ref()
                .map(|existing| existing.len() != vector.len())
                .unwrap_or(true);
            if needs_update {
                entry.semantic_vector = Some(vector.clone());
                changed = true;
            }
        }
    }
    changed
}

fn reflect_followup_is_snoozed(
    feedback: Option<&ReflectFollowupFeedbackState>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if feedback.is_some_and(|state| state.renewed_after_feedback) {
        return false;
    }
    feedback
        .and_then(|state| state.snoozed_until.as_deref())
        .and_then(parse_time)
        .is_some_and(|until| until > now)
}

fn reflect_followup_is_dismissed(feedback: Option<&ReflectFollowupFeedbackState>) -> bool {
    feedback.is_some_and(|state| state.dismiss_count > 0 && !state.renewed_after_feedback)
}

fn reflect_followup_latest_detail(cache: &ReflectFollowupSearchCache, source_id: &str) -> String {
    let Some(entry) = reflect_search_cache_entry(cache, source_id) else {
        return "A current-source check will run when AgentArk is idle.".to_string();
    };
    if entry
        .summary
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return format!(
            "Last checked {} via {}. Generated a source-backed insight from {} result{}.",
            format_uiish_time(&entry.checked_at),
            entry.backend.as_deref().unwrap_or("search"),
            entry.results.len(),
            if entry.results.len() == 1 { "" } else { "s" },
        );
    }
    if let Some(result) = entry.results.first() {
        let checked = format_uiish_time(&entry.checked_at);
        return format!(
            "Last checked {} via {}. Found {} result{}. Top result: {}",
            checked,
            entry.backend.as_deref().unwrap_or("search"),
            entry.results.len(),
            if entry.results.len() == 1 { "" } else { "s" },
            reflect_sentence_fragment(&result.title, REFLECT_SUGGESTION_TEXT_CHARS),
        );
    }
    if let Some(error) = entry
        .error
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return format!(
            "Current-source check last failed: {}",
            reflect_sentence_fragment(error, REFLECT_SUGGESTION_TEXT_CHARS),
        );
    }
    "Current-source check ran but returned no results.".to_string()
}

fn reflect_followup_latest_status(
    cache: &ReflectFollowupSearchCache,
    source_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let Some(entry) = reflect_search_cache_entry(cache, source_id) else {
        return "queued".to_string();
    };
    if reflect_followup_search_is_due(cache, source_id, now) {
        return "queued".to_string();
    }
    if entry
        .error
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return "failed".to_string();
    }
    "ready".to_string()
}

fn format_uiish_time(value: &str) -> String {
    parse_time(value)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| value.to_string())
}

fn reflect_failure_suggestion(
    run: &experience_run::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> ReflectSuggestedFollowup {
    let task_label = reflect_label_from_identifier(run.task_type.as_deref().unwrap_or("work"));
    let topic = reflect_experience_run_topic(run);
    let failure = first_non_empty([
        run.failure_reason.as_deref().unwrap_or_default(),
        run.outcome_summary.as_deref().unwrap_or_default(),
        topic.as_str(),
    ]);
    let topic_preview = reflect_sentence_fragment(&topic, 80);
    let title = if topic_preview.is_empty() {
        format!(
            "Stalled {} run needs review",
            task_label.to_ascii_lowercase()
        )
    } else {
        topic_preview.clone()
    };
    ReflectSuggestedFollowup {
        id: format!("reflect-recovery-{}", run.id),
        kind: "recovery_advice".to_string(),
        title,
        detail: reflect_sentence_fragment(&failure, REFLECT_SUGGESTION_TEXT_CHARS),
        prompt: format!(
            "Review this prior {} run and propose the next safest recovery steps: {}",
            task_label.to_ascii_lowercase(),
            topic_preview,
        ),
        status: "ready".to_string(),
        source_label: "Stalled run".to_string(),
        occurred_at: run.updated_at.clone(),
        conversation_id: run.conversation_id.clone(),
        source_unit_id: None,
        rank_score: 100.0 + reflect_recency_score(&run.updated_at, now),
        search_results: Vec::new(),
        search_checked_at: None,
        search_error: None,
        latest_summary: None,
        latest_summary_generated_at: None,
        latest_summary_error: None,
        latest_summary_evidence_supported: None,
        feedback: None,
        feedback_keys: reflect_followup_feedback_keys(
            &format!("reflect-recovery-{}", run.id),
            None,
            run.conversation_id.as_deref(),
        ),
        feedback_vector: None,
        search_query: None,
        search_planning_context: None,
        search_requires_planning: false,
        source_strategy: ReflectFollowupSourceStrategy::PublicSearch,
        structured_context: serde_json::Value::Null,
        allow_unplanned_source_check: false,
    }
}

fn reflect_latest_suggestion(
    run: &experience_run::Model,
    cache: &ReflectFollowupSearchCache,
    now: chrono::DateTime<chrono::Utc>,
) -> ReflectSuggestedFollowup {
    let topic = reflect_experience_run_topic(run);
    let topic_preview = reflect_sentence_fragment(&topic, REFLECT_SUGGESTION_TEXT_CHARS);
    let source_id = format!(
        "latest:{}",
        stable_hash(&topic_preview)
            .chars()
            .take(24)
            .collect::<String>()
    );
    let has_fresh_cache = !reflect_followup_search_is_due(cache, &source_id, now);
    let cache_entry = reflect_search_cache_entry(cache, &source_id);
    let search_results = cache_entry
        .map(|entry| entry.results.clone())
        .unwrap_or_default();
    let search_checked_at = cache_entry.and_then(|entry| {
        if entry.checked_at.trim().is_empty() {
            None
        } else {
            Some(entry.checked_at.clone())
        }
    });
    let search_error = cache_entry.and_then(|entry| {
        entry
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let (latest_summary, latest_summary_generated_at, latest_summary_error) =
        reflect_followup_latest_summary_fields(cache_entry);
    let planned_search_query = cache_entry
        .map(|entry| entry.query.trim().to_string())
        .filter(|query| !query.is_empty())
        .unwrap_or_else(|| topic_preview.clone());
    let title = if topic_preview.is_empty() {
        "Open research thread - what changed?".to_string()
    } else {
        topic_preview.clone()
    };
    ReflectSuggestedFollowup {
        id: source_id.clone(),
        kind: "latest_developments".to_string(),
        title,
        detail: reflect_followup_latest_detail(cache, &source_id),
        prompt: format!(
            "Use the reflected context and current sources to produce a useful next insight for: {}",
            topic_preview
        ),
        status: reflect_followup_latest_status(cache, &source_id, now),
        source_label: "Source insight".to_string(),
        occurred_at: run.updated_at.clone(),
        conversation_id: run.conversation_id.clone(),
        source_unit_id: None,
        rank_score: 82.0
            + reflect_recency_score(&run.updated_at, now)
            + if has_fresh_cache { 8.0 } else { 0.0 },
        search_results,
        search_checked_at,
        search_error,
        latest_summary,
        latest_summary_generated_at,
        latest_summary_error,
        latest_summary_evidence_supported: cache_entry
            .and_then(|entry| entry.summary_evidence_supported),
        feedback: None,
        feedback_keys: reflect_followup_feedback_keys(
            &source_id,
            None,
            run.conversation_id.as_deref(),
        ),
        feedback_vector: None,
        search_query: Some(planned_search_query),
        search_planning_context: Some({
            let mut parts = Vec::new();
            let request =
                reflect_external_text(run.request_text.as_deref().unwrap_or_default(), 360);
            let outcome =
                reflect_external_text(run.outcome_summary.as_deref().unwrap_or_default(), 360);
            let failure =
                reflect_external_text(run.failure_reason.as_deref().unwrap_or_default(), 260);
            if !topic_preview.trim().is_empty() {
                parts.push(format!(
                    "reflected_topic: {}",
                    reflect_external_text(&topic_preview, 220)
                ));
            }
            if !request.trim().is_empty() {
                parts.push(format!("source_request: {}", request));
            }
            if !outcome.trim().is_empty() {
                parts.push(format!("prior_outcome: {}", outcome));
            }
            if !failure.trim().is_empty() {
                parts.push(format!("prior_failure: {}", failure));
            }
            if let Some(task_type) = run
                .task_type
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                parts.push(format!("task_type: {}", task_type.trim()));
            }
            truncate_chars(&parts.join("\n"), 1200)
        }),
        search_requires_planning: true,
        source_strategy: cache_entry
            .map(|entry| entry.source_strategy)
            .unwrap_or_default(),
        structured_context: cache_entry
            .map(|entry| entry.structured_context.clone())
            .unwrap_or(serde_json::Value::Null),
        allow_unplanned_source_check: false,
    }
}

async fn build_reflect_semantic_freshness_context(
    embedder: Option<&EmbeddingClient>,
) -> Option<ReflectSemanticFreshnessContext> {
    let embedder = embedder?;
    let texts = vec![
        REFLECT_PUBLIC_DEVELOPMENT_CONCEPT_TEXT.to_string(),
        REFLECT_PRIVATE_WORK_CONCEPT_TEXT.to_string(),
    ];
    let embeddings = match tokio::time::timeout(
        REFLECT_SEMANTIC_FRESHNESS_TIMEOUT,
        embedder.embed_texts(&texts),
    )
    .await
    {
        Ok(Ok(embeddings)) => embeddings,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "semantic freshness concept embedding failed");
            return None;
        }
        Err(_) => {
            tracing::warn!("semantic freshness concept embedding timed out");
            return None;
        }
    };
    if embeddings.len() != texts.len() {
        return None;
    }
    let dimension = embeddings
        .first()
        .map(|embedding| embedding.as_slice().len())
        .filter(|dimension| *dimension > 0)?;
    let public_development = normalized_vector(&embeddings[0], dimension)?;
    let private_work = normalized_vector(&embeddings[1], dimension)?;
    if public_development.len() != private_work.len() {
        return None;
    }
    Some(ReflectSemanticFreshnessContext {
        public_development,
        private_work,
        dimension,
    })
}

fn reflect_semantic_freshness_score(
    embedding: &PgVector,
    context: &ReflectSemanticFreshnessContext,
) -> Option<ReflectSemanticFreshnessScore> {
    let vector = normalized_vector(embedding, context.dimension)?;
    let public_similarity = 1.0 - cosine_distance(&vector, &context.public_development);
    let private_similarity = 1.0 - cosine_distance(&vector, &context.private_work);
    Some(ReflectSemanticFreshnessScore {
        similarity: public_similarity,
        margin: public_similarity - private_similarity,
    })
}

fn reflect_semantic_freshness_is_actionable(score: ReflectSemanticFreshnessScore) -> bool {
    score.similarity >= REFLECT_SEMANTIC_FRESHNESS_MIN_SIMILARITY
        && score.margin >= REFLECT_SEMANTIC_FRESHNESS_MIN_MARGIN
}

fn reflect_cluster_latest_topic(cluster: &ReflectClusterResponse) -> String {
    let mut seen = HashSet::<String>::new();
    let mut parts = Vec::new();
    let usable_units = cluster
        .units
        .iter()
        .filter(|unit| !matches!(unit.source_kind.as_str(), "llm_usage"))
        .take(3)
        .collect::<Vec<_>>();
    if usable_units.is_empty() {
        return String::new();
    }
    let use_cluster_label = cluster
        .units
        .iter()
        .all(|unit| !matches!(unit.source_kind.as_str(), "llm_usage"));
    let values = use_cluster_label
        .then_some(cluster.label.as_str())
        .into_iter()
        .chain(usable_units.iter().flat_map(|unit| {
            [
                unit.title.as_str(),
                unit.summary.as_str(),
                unit.content_preview.as_str(),
            ]
        }));
    for value in values {
        let part = reflect_sentence_fragment(value, 96);
        if part.is_empty() || !seen.insert(part.clone()) {
            continue;
        }
        parts.push(part);
    }
    reflect_sentence_fragment(&parts.join(". "), REFLECT_SUGGESTION_TEXT_CHARS)
}

fn reflect_cluster_external_planning_context(cluster: &ReflectClusterResponse) -> String {
    let mut seen = HashSet::<String>::new();
    let mut parts = Vec::new();
    for unit in cluster
        .units
        .iter()
        .filter(|unit| !matches!(unit.source_kind.as_str(), "llm_usage"))
        .take(5)
    {
        let mut unit_parts = vec![
            format!("source_kind: {}", unit.source_kind),
            format!("source_label: {}", unit.source_label),
        ];
        for (label, value) in [
            ("title", unit.title.as_str()),
            ("summary", unit.summary.as_str()),
            ("preview", unit.content_preview.as_str()),
        ] {
            let fragment = reflect_external_text(value, 260);
            if fragment.is_empty() || !seen.insert(format!("{}:{}", label, fragment)) {
                continue;
            }
            unit_parts.push(format!("{}: {}", label, fragment));
        }
        if let Some(kind) = unit.metadata.get("kind").and_then(|value| value.as_str()) {
            unit_parts.push(format!("memory_kind: {}", kind));
        }
        parts.push(unit_parts.join("\n"));
    }
    if parts.is_empty() {
        return reflect_cluster_latest_topic(cluster);
    }
    truncate_chars(&parts.join("\n\n"), 1400)
}

fn reflect_source_kind_is_user_surface(kind: &str) -> bool {
    matches!(
        kind,
        "conversation" | "orbit_chat" | "experience_item" | "procedural_pattern" | "app" | "goal"
    )
}

fn reflect_cluster_user_surface_unit(
    cluster: &ReflectClusterResponse,
) -> Option<&ReflectUnitResponse> {
    cluster
        .units
        .iter()
        .find(|unit| reflect_source_kind_is_user_surface(&unit.source_kind))
}

fn reflect_semantic_cluster_latest_suggestion(
    cluster: &ReflectClusterResponse,
    cache: &ReflectFollowupSearchCache,
    now: chrono::DateTime<chrono::Utc>,
    score: ReflectSemanticFreshnessScore,
) -> Option<ReflectSuggestedFollowup> {
    if !reflect_semantic_freshness_is_actionable(score) {
        return None;
    }
    let unit = reflect_cluster_user_surface_unit(cluster)?;
    let topic = reflect_cluster_latest_topic(cluster);
    if topic.trim().is_empty() {
        return None;
    }
    let planning_context = reflect_cluster_external_planning_context(cluster);
    let source_id = format!(
        "latest:semantic:{}",
        stable_hash(&format!("{}:{}", cluster.representative_unit_id, topic))
            .chars()
            .take(24)
            .collect::<String>()
    );
    let cache_entry = reflect_search_cache_entry(cache, &source_id);
    let has_fresh_cache = !reflect_followup_search_is_due(cache, &source_id, now);
    let search_results = cache_entry
        .map(|entry| entry.results.clone())
        .unwrap_or_default();
    let search_checked_at = cache_entry.and_then(|entry| {
        if entry.checked_at.trim().is_empty() {
            None
        } else {
            Some(entry.checked_at.clone())
        }
    });
    let search_error = cache_entry.and_then(|entry| {
        entry
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let (latest_summary, latest_summary_generated_at, latest_summary_error) =
        reflect_followup_latest_summary_fields(cache_entry);
    let planned_search_query = cache_entry
        .map(|entry| entry.query.trim().to_string())
        .filter(|query| !query.is_empty())
        .unwrap_or_else(|| topic.clone());
    let detail = if cache_entry.is_some() {
        reflect_followup_latest_detail(cache, &source_id)
    } else {
        "Reflect inferred that this reflected topic may benefit from current external sources. A source check will run when AgentArk is idle.".to_string()
    };
    let conversation_id = unit
        .metadata
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some(ReflectSuggestedFollowup {
        id: source_id.clone(),
        kind: "latest_developments".to_string(),
        title: topic.clone(),
        detail,
        prompt: format!(
            "Use current source evidence for this reflected topic, then summarize the useful insight and the next practical step: {}",
            topic
        ),
        status: reflect_followup_latest_status(cache, &source_id, now),
        source_label: "Source insight".to_string(),
        occurred_at: unit.occurred_at.clone(),
        conversation_id: conversation_id.clone(),
        source_unit_id: Some(unit.id.clone()),
        rank_score: 76.0
            + reflect_recency_score(&unit.occurred_at, now)
            + cluster.unit_count.min(6) as f64
            + (score.similarity.max(0.0) as f64 * 12.0)
            + (score.margin.max(0.0) as f64 * 24.0)
            + if has_fresh_cache { 8.0 } else { 0.0 },
        search_results,
        search_checked_at,
        search_error,
        latest_summary,
        latest_summary_generated_at,
        latest_summary_error,
        latest_summary_evidence_supported: cache_entry
            .and_then(|entry| entry.summary_evidence_supported),
        feedback: None,
        feedback_keys: reflect_followup_feedback_keys(
            &source_id,
            Some(&unit.id),
            conversation_id.as_deref(),
        ),
        feedback_vector: cluster
            .centroid_embedding
            .as_ref()
            .map(|embedding| embedding.as_slice().to_vec()),
        search_query: Some(planned_search_query),
        search_planning_context: Some(planning_context),
        search_requires_planning: true,
        source_strategy: cache_entry
            .map(|entry| entry.source_strategy)
            .unwrap_or_default(),
        structured_context: cache_entry
            .map(|entry| entry.structured_context.clone())
            .unwrap_or(serde_json::Value::Null),
        allow_unplanned_source_check: false,
    })
}

fn reflect_planned_cluster_latest_suggestion(
    cluster: &ReflectClusterResponse,
    cache: &ReflectFollowupSearchCache,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectSuggestedFollowup> {
    let unit = reflect_cluster_user_surface_unit(cluster)?;
    let topic = reflect_cluster_latest_topic(cluster);
    if topic.trim().is_empty() {
        return None;
    }
    let planning_context = reflect_cluster_external_planning_context(cluster);
    let source_id = format!(
        "latest:planned:{}",
        stable_hash(&format!("{}:{}", cluster.id, topic))
            .chars()
            .take(24)
            .collect::<String>()
    );
    let cache_entry = reflect_search_cache_entry(cache, &source_id);
    let has_fresh_cache = !reflect_followup_search_is_due(cache, &source_id, now);
    let search_results = cache_entry
        .map(|entry| entry.results.clone())
        .unwrap_or_default();
    let search_checked_at = cache_entry.and_then(|entry| {
        if entry.checked_at.trim().is_empty() {
            None
        } else {
            Some(entry.checked_at.clone())
        }
    });
    let search_error = cache_entry.and_then(|entry| {
        entry
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    let (latest_summary, latest_summary_generated_at, latest_summary_error) =
        reflect_followup_latest_summary_fields(cache_entry);
    let planned_search_query = cache_entry
        .map(|entry| entry.query.trim().to_string())
        .filter(|query| !query.is_empty())
        .unwrap_or_else(|| topic.clone());
    let detail = if cache_entry.is_some() {
        reflect_followup_latest_detail(cache, &source_id)
    } else {
        "Reflect found a user-facing reflected topic that may need current public evidence. AgentArk will only keep it if the planner finds a useful source-backed pursuit.".to_string()
    };
    let conversation_id = unit
        .metadata
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some(ReflectSuggestedFollowup {
        id: source_id.clone(),
        kind: "latest_developments".to_string(),
        title: topic.clone(),
        detail,
        prompt: format!(
            "Use current source evidence for this reflected topic, then summarize the useful insight and next practical step: {}",
            topic
        ),
        status: reflect_followup_latest_status(cache, &source_id, now),
        source_label: "Source insight".to_string(),
        occurred_at: unit.occurred_at.clone(),
        conversation_id: conversation_id.clone(),
        source_unit_id: Some(unit.id.clone()),
        rank_score: 70.0
            + reflect_recency_score(&unit.occurred_at, now)
            + cluster.unit_count.min(6) as f64
            + if has_fresh_cache { 8.0 } else { 0.0 },
        search_results,
        search_checked_at,
        search_error,
        latest_summary,
        latest_summary_generated_at,
        latest_summary_error,
        latest_summary_evidence_supported: cache_entry
            .and_then(|entry| entry.summary_evidence_supported),
        feedback: None,
        feedback_keys: reflect_followup_feedback_keys(
            &source_id,
            Some(&unit.id),
            conversation_id.as_deref(),
        ),
        feedback_vector: cluster
            .centroid_embedding
            .as_ref()
            .map(|embedding| embedding.as_slice().to_vec()),
        search_query: Some(planned_search_query),
        search_planning_context: Some(planning_context),
        search_requires_planning: true,
        source_strategy: cache_entry
            .map(|entry| entry.source_strategy)
            .unwrap_or_default(),
        structured_context: cache_entry
            .map(|entry| entry.structured_context.clone())
            .unwrap_or(serde_json::Value::Null),
        allow_unplanned_source_check: false,
    })
}

fn reflect_followup_display_family(kind: &str) -> &'static str {
    match kind {
        "latest_developments" => "next_step",
        "recovery_advice" => "review_thread",
        _ => "other",
    }
}

fn reflect_followup_intent_text(candidate: &ReflectSuggestedFollowup) -> String {
    let mut values = vec![candidate.title.as_str()];
    if let Some(search_query) = candidate.search_query.as_deref() {
        values.push(search_query);
    }
    if candidate.kind == "recovery_advice" || candidate.title.trim().is_empty() {
        values.push(candidate.detail.as_str());
    }
    values
        .into_iter()
        .filter_map(|value| {
            let fragment = reflect_sentence_fragment(value, 260);
            (!fragment.trim().is_empty()).then_some(fragment)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn reflect_followup_token_is_generic(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "again"
            | "against"
            | "and"
            | "are"
            | "can"
            | "current"
            | "for"
            | "from"
            | "help"
            | "into"
            | "new"
            | "next"
            | "now"
            | "old"
            | "out"
            | "prior"
            | "review"
            | "same"
            | "set"
            | "step"
            | "that"
            | "the"
            | "this"
            | "thread"
            | "use"
            | "with"
    )
}

fn reflect_followup_meaning_tokens(value: &str) -> HashSet<String> {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|token| token.chars().count() >= 3 && !reflect_followup_token_is_generic(token))
        .map(str::to_string)
        .collect()
}

fn reflect_followup_text_similarity(left: &str, right: &str) -> f64 {
    let left_tokens = reflect_followup_meaning_tokens(left);
    let right_tokens = reflect_followup_meaning_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let overlap = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    let smaller = left_tokens.len().min(right_tokens.len()) as f64;
    let jaccard = if union > 0.0 { overlap / union } else { 0.0 };
    let containment = if smaller > 0.0 {
        overlap / smaller
    } else {
        0.0
    };
    jaccard.max(containment)
}

fn reflect_followup_vectors_duplicate(
    left: &ReflectSuggestedFollowup,
    right: &ReflectSuggestedFollowup,
) -> bool {
    let (Some(left_vector), Some(right_vector)) = (
        left.feedback_vector.as_deref(),
        right.feedback_vector.as_deref(),
    ) else {
        return false;
    };
    !left_vector.is_empty()
        && left_vector.len() == right_vector.len()
        && cosine_distance(left_vector, right_vector) <= REFLECT_FEEDBACK_SEMANTIC_MAX_DISTANCE
}

fn reflect_followups_are_duplicates(
    left: &ReflectSuggestedFollowup,
    right: &ReflectSuggestedFollowup,
) -> bool {
    if reflect_followup_display_family(&left.kind) != reflect_followup_display_family(&right.kind) {
        return false;
    }
    if !left.id.trim().is_empty() && left.id == right.id {
        return true;
    }
    if left.source_unit_id.is_some()
        && right.source_unit_id.is_some()
        && left.source_unit_id == right.source_unit_id
    {
        return true;
    }
    if reflect_followup_vectors_duplicate(left, right) {
        return true;
    }
    let left_text = reflect_followup_intent_text(left);
    let right_text = reflect_followup_intent_text(right);
    reflect_followup_text_similarity(&left_text, &right_text)
        >= REFLECT_FOLLOWUP_TEXT_DUPLICATE_THRESHOLD
}

fn select_top_reflect_followups(
    mut candidates: Vec<ReflectSuggestedFollowup>,
) -> Vec<ReflectSuggestedFollowup> {
    candidates.sort_by(|a, b| {
        b.rank_score
            .total_cmp(&a.rank_score)
            .then_with(|| b.occurred_at.cmp(&a.occurred_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut selected = Vec::new();
    let mut selected_ids = HashSet::<String>::new();
    let mut kind_counts = BTreeMap::<String, usize>::new();
    for candidate in &candidates {
        if selected.len() >= REFLECT_MAX_SUGGESTED_FOLLOWUPS {
            break;
        }
        if selected_ids.contains(&candidate.id) {
            continue;
        }
        if selected
            .iter()
            .any(|existing| reflect_followups_are_duplicates(existing, candidate))
        {
            continue;
        }
        if kind_counts.get(&candidate.kind).copied().unwrap_or(0) >= 4 {
            continue;
        }
        selected_ids.insert(candidate.id.clone());
        *kind_counts.entry(candidate.kind.clone()).or_default() += 1;
        selected.push(candidate.clone());
    }
    selected
}

async fn load_reflect_followup_search_cache(storage: &Storage) -> ReflectFollowupSearchCache {
    storage
        .get(REFLECT_FOLLOWUP_SEARCH_CACHE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<ReflectFollowupSearchCache>(&bytes).ok())
        .unwrap_or_default()
}

async fn save_reflect_followup_search_cache(storage: &Storage, cache: &ReflectFollowupSearchCache) {
    if let Ok(bytes) = serde_json::to_vec(cache) {
        if let Err(error) = storage.set(REFLECT_FOLLOWUP_SEARCH_CACHE_KEY, &bytes).await {
            tracing::warn!(error = %error, "failed to save followup search cache");
        }
    }
}

async fn load_reflect_followup_plan_cache(storage: &Storage) -> ReflectFollowupPlanCache {
    storage
        .get(REFLECT_FOLLOWUP_PLAN_CACHE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<ReflectFollowupPlanCache>(&bytes).ok())
        .unwrap_or_default()
}

async fn save_reflect_followup_plan_cache(storage: &Storage, cache: &ReflectFollowupPlanCache) {
    if let Ok(bytes) = serde_json::to_vec(cache) {
        if let Err(error) = storage.set(REFLECT_FOLLOWUP_PLAN_CACHE_KEY, &bytes).await {
            tracing::warn!(error = %error, "failed to save followup plan cache");
        }
    }
}

fn reflect_plan_cache_entry_from_plan(
    plan: &ReflectExternalPursuitPlan,
    planned_at: &str,
    topic: &str,
) -> Option<ReflectFollowupPlanEntry> {
    let id = plan.id.trim();
    if id.is_empty() {
        return None;
    }
    let topic = truncate_chars(topic.trim(), REFLECT_FOLLOWUP_PLAN_TOPIC_CHARS);
    if !plan.useful {
        return Some(ReflectFollowupPlanEntry {
            id: truncate_chars(id, 140),
            useful: false,
            title: String::new(),
            search_query: String::new(),
            rationale: reflect_external_text(&plan.rationale, 220),
            source_strategy: plan.source_strategy,
            structured_context: plan.structured_context.clone(),
            planned_at: planned_at.to_string(),
            topic,
        });
    }

    let title = reflect_external_text(&plan.title, 120);
    let search_query = plan.search_query.trim();
    if title.is_empty()
        || search_query.is_empty()
        || !reflect_external_search_query_is_safe(search_query)
    {
        // Cache the failure as a short-lived not-useful verdict instead of
        // dropping it: an uncached candidate would re-enter the planner on
        // every poll, looping the same defective verdict forever. The
        // negative TTL re-judges it in a few days.
        tracing::warn!(
            plan_id = %id,
            "planner returned unusable useful pursuit; caching as not useful"
        );
        return Some(ReflectFollowupPlanEntry {
            id: truncate_chars(id, 140),
            useful: false,
            title: String::new(),
            search_query: String::new(),
            rationale: "Planner returned an unusable pursuit (empty or unsafe query).".to_string(),
            source_strategy: plan.source_strategy,
            structured_context: serde_json::Value::Null,
            planned_at: planned_at.to_string(),
            topic,
        });
    }

    Some(ReflectFollowupPlanEntry {
        id: truncate_chars(id, 140),
        useful: true,
        title,
        search_query: search_query.to_string(),
        rationale: reflect_external_text(&plan.rationale, 220),
        source_strategy: plan.source_strategy,
        structured_context: plan.structured_context.clone(),
        planned_at: planned_at.to_string(),
        topic,
    })
}

fn reflect_plan_from_cache_entry(entry: &ReflectFollowupPlanEntry) -> ReflectExternalPursuitPlan {
    ReflectExternalPursuitPlan {
        id: entry.id.clone(),
        useful: entry.useful,
        title: entry.title.clone(),
        search_query: entry.search_query.clone(),
        rationale: entry.rationale.clone(),
        source_strategy: entry.source_strategy,
        structured_context: entry.structured_context.clone(),
    }
}

fn prune_reflect_followup_plan_cache(
    cache: &mut ReflectFollowupPlanCache,
    now: chrono::DateTime<chrono::Utc>,
) {
    cache.entries.retain(|_, entry| {
        if entry.id.trim().is_empty() {
            return false;
        }
        // Not-useful verdicts expire quickly so evolving interests get
        // re-judged instead of staying pinned dead for months.
        let retention_days = if entry.useful {
            REFLECT_FOLLOWUP_PLAN_RETENTION_DAYS
        } else {
            REFLECT_FOLLOWUP_PLAN_NEGATIVE_RETENTION_DAYS
        };
        parse_time(&entry.planned_at)
            .is_none_or(|planned_at| (now - planned_at).num_days() <= retention_days)
    });
}

fn update_reflect_followup_plan_cache(
    cache: &mut ReflectFollowupPlanCache,
    plans: &BTreeMap<String, ReflectExternalPursuitPlan>,
    topics: &BTreeMap<String, String>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let planned_at = now.to_rfc3339();
    let mut changed = false;
    for plan in plans.values() {
        let topic = topics.get(&plan.id).map(String::as_str).unwrap_or("");
        let Some(entry) = reflect_plan_cache_entry_from_plan(plan, &planned_at, topic) else {
            continue;
        };
        let changed_entry = cache.entries.get(&entry.id).is_none_or(|existing| {
            existing.useful != entry.useful
                || existing.title != entry.title
                || existing.search_query != entry.search_query
                || existing.rationale != entry.rationale
                || existing.topic != entry.topic
        });
        if changed_entry {
            changed = true;
        }
        cache.entries.insert(entry.id.clone(), entry);
    }
    let before = cache.entries.len();
    prune_reflect_followup_plan_cache(cache, now);
    if cache.entries.len() != before {
        changed = true;
    }
    if changed {
        cache.updated_at = Some(planned_at);
    }
    changed
}

async fn update_reflect_followup_search_cache<R, F>(storage: &Storage, update: F) -> Option<R>
where
    F: FnOnce(&mut ReflectFollowupSearchCache) -> R + Send,
    R: Send,
{
    let lease_owner = format!(
        "arkreflect-followup-cache:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let lease_guard = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.acquire_kv_lease_guard(
            REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_KEY,
            &lease_owner,
            REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_TTL_SECS,
        ),
    )
    .await
    {
        Ok(Ok(Some(guard))) => guard,
        Ok(Ok(None)) => {
            tracing::debug!("followup cache write lease is held elsewhere");
            return None;
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "failed to acquire followup cache write lease");
            return None;
        }
        Err(_) => {
            tracing::warn!("followup cache write lease timed out");
            return None;
        }
    };
    let mut cache = load_reflect_followup_search_cache(storage).await;
    let result = update(&mut cache);
    cache.updated_at = Some(chrono::Utc::now().to_rfc3339());
    prune_reflect_followup_search_cache(&mut cache);
    save_reflect_followup_search_cache(storage, &cache).await;
    if let Err(error) = storage
        .release_kv_lease_guard(REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_KEY, &lease_guard)
        .await
    {
        tracing::debug!(error = %error, "failed to release followup cache write lease");
    }
    Some(result)
}

/// Leased read-modify-write for the plan cache, mirroring the search-cache
/// helper: a fresh load happens inside the lease so concurrent planning
/// passes (view-triggered vs idle/post-refresh) cannot clobber each other's
/// persisted verdicts. Returns the merged cache, or None if the lease was
/// unavailable (callers fall back to a local, unpersisted merge).
async fn update_reflect_followup_plan_cache_guarded(
    storage: &Storage,
    plans: &BTreeMap<String, ReflectExternalPursuitPlan>,
    topics: &BTreeMap<String, String>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectFollowupPlanCache> {
    let lease_owner = format!(
        "arkreflect-followup-plan:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let lease_guard = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.acquire_kv_lease_guard(
            REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_KEY,
            &lease_owner,
            REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_TTL_SECS,
        ),
    )
    .await
    {
        Ok(Ok(Some(guard))) => guard,
        Ok(Ok(None)) => {
            tracing::debug!("followup cache write lease is held elsewhere; plan persist skipped");
            return None;
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "failed to acquire followup cache write lease");
            return None;
        }
        Err(_) => {
            tracing::warn!("followup cache write lease timed out");
            return None;
        }
    };
    let mut cache = load_reflect_followup_plan_cache(storage).await;
    if update_reflect_followup_plan_cache(&mut cache, plans, topics, now) {
        save_reflect_followup_plan_cache(storage, &cache).await;
    }
    if let Err(error) = storage
        .release_kv_lease_guard(REFLECT_FOLLOWUP_CACHE_WRITE_LEASE_KEY, &lease_guard)
        .await
    {
        tracing::debug!(error = %error, "failed to release followup cache write lease");
    }
    Some(cache)
}

fn reflect_sensitive_key_fragment(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "client_secret",
        "password",
        "passwd",
        "private_key",
        "refresh_token",
        "access_token",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn reflect_token_looks_sensitive(value: &str) -> bool {
    let trimmed = value.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '@');
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('@') && trimmed.contains('.') {
        return true;
    }
    if reflect_sensitive_key_fragment(trimmed) {
        return true;
    }
    let alnum_count = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count();
    let has_alpha = trimmed.chars().any(|ch| ch.is_ascii_alphabetic());
    let has_digit = trimmed.chars().any(|ch| ch.is_ascii_digit());
    alnum_count >= 40 && has_alpha && has_digit
}

fn reflect_line_looks_sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    reflect_sensitive_key_fragment(&lower)
        && (lower.contains(':') || lower.contains('=') || lower.contains(" is "))
}

fn reflect_external_text(value: &str, max_chars: usize) -> String {
    let mut parts = Vec::new();
    for line in value.lines().take(8) {
        if reflect_line_looks_sensitive(line) {
            parts.push("[redacted-sensitive-context]".to_string());
            continue;
        }
        let redacted = line
            .split_whitespace()
            .map(|token| {
                if reflect_token_looks_sensitive(token) {
                    "[redacted]".to_string()
                } else {
                    token.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !redacted.trim().is_empty() {
            parts.push(redacted);
        }
    }
    truncate_chars(&parts.join("\n"), max_chars)
}

fn reflect_external_search_query_is_safe(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.chars().count() < 3 || trimmed.chars().count() > 240 {
        return false;
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
    {
        return false;
    }
    !reflect_line_looks_sensitive(trimmed)
        && !trimmed
            .split_whitespace()
            .any(reflect_token_looks_sensitive)
}

fn reflect_public_context_field(
    context: &serde_json::Value,
    key: &str,
    max_chars: usize,
) -> Option<String> {
    let raw = context.get(key)?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = reflect_external_text(raw, max_chars)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if value.is_empty() || value.contains("[redacted") {
        return None;
    }
    Some(value)
}

fn reflect_travel_price_search_query(
    _fallback_query: &str,
    structured_context: &serde_json::Value,
) -> Option<String> {
    let origin = reflect_public_context_field(structured_context, "origin", 80)?;
    let destination = reflect_public_context_field(structured_context, "destination", 80)?;
    let trip_window = reflect_public_context_field(structured_context, "trip_window", 80);
    let trip_type = reflect_public_context_field(structured_context, "trip_type", 48);
    let currency = reflect_public_context_field(structured_context, "currency", 24);

    let mut parts = Vec::new();
    parts.push(origin);
    parts.push("to".to_string());
    parts.push(destination);
    if let Some(trip_window) = trip_window {
        parts.push(trip_window);
    }
    if let Some(trip_type) = trip_type {
        parts.push(trip_type);
    }
    parts.push("flexible dates".to_string());
    parts.push("cheap flights".to_string());
    if let Some(currency) = currency {
        parts.push(currency);
    }
    parts.push("current fares".to_string());

    let query = truncate_chars(&parts.join(" "), 240)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    reflect_external_search_query_is_safe(&query).then_some(query)
}

fn extract_reflect_json_value(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    let object = trimmed
        .find('{')
        .zip(trimmed.rfind('}'))
        .and_then(|(start, end)| {
            if end >= start {
                serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]).ok()
            } else {
                None
            }
        });
    if object.is_some() {
        return object;
    }
    trimmed
        .find('[')
        .zip(trimmed.rfind(']'))
        .and_then(|(start, end)| {
            if end >= start {
                serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]).ok()
            } else {
                None
            }
        })
}

fn parse_reflect_external_pursuit_plans(
    response: &str,
) -> BTreeMap<String, ReflectExternalPursuitPlan> {
    let Some(value) = extract_reflect_json_value(response) else {
        return BTreeMap::new();
    };
    let plans_value = value.get("plans").cloned().unwrap_or_else(|| value.clone());
    let plans = match plans_value {
        serde_json::Value::Array(values) => values,
        other => vec![other],
    };
    plans
        .into_iter()
        .filter_map(|value| serde_json::from_value::<ReflectExternalPursuitPlan>(value).ok())
        .filter(|plan| !plan.id.trim().is_empty())
        .map(|mut plan| {
            plan.id = truncate_chars(plan.id.trim(), 140);
            plan.title = truncate_chars(plan.title.trim(), 120);
            plan.search_query = truncate_chars(plan.search_query.trim(), 240);
            plan.rationale = truncate_chars(plan.rationale.trim(), 220);
            (plan.id.clone(), plan)
        })
        .collect()
}

fn reflect_planning_handle(index: usize) -> String {
    format!("item_{}", index + 1)
}

fn reflect_candidate_planning_topic(candidate: &ReflectSuggestedFollowup) -> String {
    candidate
        .search_planning_context
        .as_deref()
        .or(candidate.search_query.as_deref())
        .unwrap_or(&candidate.title)
        .to_string()
}

/// Content-only topic text for plan-cache identity. Deliberately NOT the
/// planning context: that blob is label-scaffolded ("source_kind:", "title:",
/// ...) and the shared template tokens would make unrelated topics look alike.
fn reflect_candidate_topic_for_cache(candidate: &ReflectSuggestedFollowup) -> String {
    reflect_followup_intent_text(candidate)
}

/// Strict meaning match for reusing a cached plan verdict across candidate-id
/// churn: jaccard only (containment lets a short text spuriously match a long
/// one), at least 3 shared meaningful tokens, both sides non-trivial.
fn reflect_plan_topic_reuse_score(candidate_topic: &str, entry_topic: &str) -> Option<f64> {
    let candidate_tokens = reflect_followup_meaning_tokens(candidate_topic);
    let entry_tokens = reflect_followup_meaning_tokens(entry_topic);
    if candidate_tokens.len() < 3 || entry_tokens.len() < 3 {
        return None;
    }
    let overlap = candidate_tokens.intersection(&entry_tokens).count();
    if overlap < 3 {
        return None;
    }
    let union = candidate_tokens.union(&entry_tokens).count();
    if union == 0 {
        return None;
    }
    let jaccard = overlap as f64 / union as f64;
    (jaccard >= REFLECT_FOLLOWUP_TEXT_DUPLICATE_THRESHOLD).then_some(jaccard)
}

fn build_reflect_external_pursuit_planning_items(
    candidates: &[ReflectSuggestedFollowup],
) -> (Vec<serde_json::Value>, BTreeMap<String, String>) {
    let mut handle_to_candidate_id = BTreeMap::new();
    let planning_items = candidates
        .iter()
        .filter(|candidate| candidate.search_requires_planning)
        .take(REFLECT_FOLLOWUP_PLAN_LIMIT)
        .enumerate()
        .map(|(index, candidate)| {
            let handle = reflect_planning_handle(index);
            handle_to_candidate_id.insert(handle.clone(), candidate.id.clone());
            let reflected_topic = reflect_candidate_planning_topic(candidate);
            serde_json::json!({
                "id": handle,
                "reflected_topic": reflect_external_text(&reflected_topic, 900),
                "source_label": candidate.source_label,
                "detail": reflect_external_text(&candidate.detail, 260),
            })
        })
        .collect::<Vec<_>>();
    (planning_items, handle_to_candidate_id)
}

fn remap_reflect_external_pursuit_plan_ids(
    plans: BTreeMap<String, ReflectExternalPursuitPlan>,
    handle_to_candidate_id: &BTreeMap<String, String>,
) -> BTreeMap<String, ReflectExternalPursuitPlan> {
    if handle_to_candidate_id.is_empty() {
        return BTreeMap::new();
    }
    let candidate_ids = handle_to_candidate_id
        .values()
        .cloned()
        .collect::<HashSet<_>>();
    plans
        .into_iter()
        .filter_map(|(key, mut plan)| {
            let resolved_id = handle_to_candidate_id
                .get(plan.id.trim())
                .or_else(|| handle_to_candidate_id.get(key.trim()))
                .cloned()
                .or_else(|| {
                    candidate_ids
                        .contains(plan.id.trim())
                        .then(|| plan.id.clone())
                })?;
            plan.id = resolved_id.clone();
            Some((resolved_id, plan))
        })
        .collect()
}

async fn plan_reflect_external_pursuits(
    llm: Option<&LlmClient>,
    candidates: &[ReflectSuggestedFollowup],
) -> BTreeMap<String, ReflectExternalPursuitPlan> {
    let Some(llm) = llm else {
        return BTreeMap::new();
    };
    let (planning_items, handle_to_candidate_id) =
        build_reflect_external_pursuit_planning_items(candidates);
    if planning_items.is_empty() {
        return BTreeMap::new();
    }
    let _planning_active_guard = ReflectFollowupPlanningActiveGuard::new();
    let system_prompt = reflect_opportunity_classifier_system_prompt();
    let user_message = format!(
        "Classify these reflected topics as human next steps before any source enrichment runs:\n{}\n\nEach id is an opaque handle such as item_1. Return exactly that handle in the id field; do not create, redact, alter, hash, or expose internal identifiers. For each useful next step, include the public-source query that should be used later to enrich the UI. Do not require search results to exist at this stage. source_strategy may be public_search unless a structured discovery surface such as flight_price_discovery is clearly more useful; structured_context should contain only non-sensitive constraints needed by that strategy.\n\nReturn JSON in this exact shape: {{\"plans\":[{{\"id\":\"same item id\",\"useful\":true|false,\"title\":\"short human-facing title when useful\",\"search_query\":\"public web search query for later enrichment when useful\",\"rationale\":\"brief reason\",\"source_strategy\":\"public_search\",\"structured_context\":{{}}}}]}}",
        serde_json::to_string_pretty(&planning_items).unwrap_or_else(|_| "[]".to_string())
    );
    match tokio::time::timeout(
        REFLECT_FOLLOWUP_PLAN_TIMEOUT,
        llm.chat_with_system(system_prompt, &user_message),
    )
    .await
    {
        Ok(Ok(response)) => {
            let plans = remap_reflect_external_pursuit_plan_ids(
                parse_reflect_external_pursuit_plans(&response.content),
                &handle_to_candidate_id,
            );
            if plans.is_empty() {
                tracing::warn!(
                    items = planning_items.len(),
                    "reflect opportunity planner returned no parseable verdicts"
                );
                record_reflect_planning_attempt(Some(
                    "Opportunity planning returned no usable verdicts.".to_string(),
                ))
                .await;
            } else {
                record_reflect_planning_attempt(None).await;
            }
            plans
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "reflect opportunity planning failed");
            record_reflect_planning_attempt(Some(truncate_chars(
                &format!("Opportunity planning failed: {}", error),
                300,
            )))
            .await;
            BTreeMap::new()
        }
        Err(_) => {
            tracing::warn!("reflect opportunity planning timed out");
            record_reflect_planning_attempt(Some("Opportunity planning timed out.".to_string()))
                .await;
            BTreeMap::new()
        }
    }
}

fn reflect_opportunity_classifier_system_prompt() -> &'static str {
    "You classify whether Reflect reflected topics are useful human next steps. Classification happens before search, so do not require source results to already exist. Work from underlying user intent and semantic meaning, not exact words, topic labels, keyword matches, templates, or anticipated phrasing. Differences in wording, order, grammar, punctuation, casing, spacing, tone, abbreviations, typos, and paraphrasing must not change the decision when the intent is the same. A useful item is an x+1, x+2, or x+z pursuit: public evidence would materially help the user compare, decide, prepare, monitor, book, plan, study, understand what changed, estimate cost/timing, or defer intelligently. Do not merely tell the user to continue the old thread. Use the old reflected context as the seed, then create the next researched item that would add new evidence or a concrete decision path. The useful horizon may be immediate, near-term, later, exploratory, or recurring. Reflected memories and ambient interests are eligible only when they imply a real future decision or public-source check; private facts alone are context, not facts to validate. Treat private or personal context only as intent context: do not validate private memories, profile facts, names, relationships, emotions, or assertions against the web. If the useful next step can be grounded in public sources without exposing unrelated private details, emit a concise human-facing title and a public search query for later enrichment. Preserve the user's real deliverable, named entity, domain, constraints, location or timing constraints when relevant, and evaluation criteria when present. If currentness matters, target the freshest reliable evidence needed for the decision rather than generic news. For analytical topics, target the evidence needed to answer the real decision rather than broad background. For potentially sensitive personal topics, avoid diagnosis and avoid turning private claims into public queries; target reputable practical public resources if useful. Remove unrelated private facts, raw memory keys, credentials, and unnecessary personal identifiers from the query. Do not surface routine background/system state unless it clearly contains a user-facing decision or repair path. Judge usefulness from the user's perspective: keep only pursuits a person with this activity would genuinely want to act on, learn from, or monitor right now; an update nobody asked for about something nobody is deciding is not useful. If public evidence would not materially improve the user's next step, mark it not useful. Return only JSON."
}

fn reflect_external_plan_for_candidate(
    candidate: &ReflectSuggestedFollowup,
    plans: &BTreeMap<String, ReflectExternalPursuitPlan>,
    plan_cache: &ReflectFollowupPlanCache,
) -> Option<ReflectExternalPursuitPlan> {
    if let Some(plan) = plans.get(&candidate.id) {
        return Some(plan.clone());
    }
    if let Some(entry) = plan_cache.entries.get(&candidate.id) {
        return Some(reflect_plan_from_cache_entry(entry));
    }
    // Candidate ids hash cluster topology, which drifts as new activity
    // arrives. Reuse USEFUL verdicts by topic meaning so id churn doesn't
    // re-pay the planner. Negative verdicts are intentionally excluded: a
    // false semantic match would silently suppress a distinct topic, while a
    // miss only costs one cheap re-plan through the pending bucket.
    let topic = reflect_candidate_topic_for_cache(candidate);
    plan_cache
        .entries
        .values()
        .filter(|entry| entry.useful && !entry.topic.trim().is_empty())
        .filter_map(|entry| {
            reflect_plan_topic_reuse_score(&topic, &entry.topic).map(|score| (score, entry))
        })
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, entry)| reflect_plan_from_cache_entry(entry))
}

fn apply_reflect_external_pursuit_plans(
    candidates: Vec<ReflectSuggestedFollowup>,
    plans: &BTreeMap<String, ReflectExternalPursuitPlan>,
    plan_cache: &ReflectFollowupPlanCache,
) -> (Vec<ReflectSuggestedFollowup>, Vec<ReflectSuggestedFollowup>) {
    let mut kept = Vec::new();
    let mut unplanned = Vec::new();
    for mut candidate in candidates {
        if !candidate.search_requires_planning {
            kept.push(candidate);
            continue;
        }
        let Some(plan) = reflect_external_plan_for_candidate(&candidate, plans, plan_cache) else {
            if candidate.allow_unplanned_source_check {
                if let Some(applied) = apply_unplanned_reflect_source_check(candidate) {
                    kept.push(applied);
                }
            } else {
                // No verdict yet: report as pending so the caller can plan it
                // instead of silently erasing the topic.
                unplanned.push(candidate);
            }
            continue;
        };
        if !plan.useful || plan.search_query.trim().is_empty() || plan.title.trim().is_empty() {
            continue;
        }
        if !reflect_external_search_query_is_safe(&plan.search_query) {
            tracing::warn!(
                source_id = %candidate.id,
                "dropping planned next step with unsafe external search query"
            );
            continue;
        }
        let cached_plan_query_matches = candidate.search_checked_at.is_some()
            && candidate.search_query.as_deref().is_some_and(|query| {
                candidate.source_strategy == plan.source_strategy
                    && reflect_search_queries_match_for_cache(query, &plan.search_query)
            });
        let query_changed = !cached_plan_query_matches
            && candidate
                .search_query
                .as_deref()
                .map(|query| !reflect_search_queries_match_for_cache(query, &plan.search_query))
                .unwrap_or(true);
        candidate.title = reflect_external_text(&plan.title, 120);
        candidate.search_query = Some(plan.search_query.trim().to_string());
        candidate.search_requires_planning = false;
        candidate.prompt = format!(
            "Use current source evidence to summarize this useful next step: {}",
            candidate.title
        );
        candidate.source_strategy = plan.source_strategy;
        candidate.structured_context = plan.structured_context.clone();
        candidate.detail = if plan.rationale.trim().is_empty() {
            "Reflect classified this reflected topic as a useful next step. Source enrichment is queued.".to_string()
        } else {
            reflect_external_text(&plan.rationale, 220)
        };
        if query_changed {
            candidate.status = "queued".to_string();
            candidate.search_results.clear();
            candidate.search_checked_at = None;
            candidate.search_error = None;
            candidate.latest_summary = None;
            candidate.latest_summary_generated_at = None;
            candidate.latest_summary_error = None;
        }
        candidate.rank_score += 12.0;
        kept.push(candidate);
    }
    (kept, unplanned)
}

fn apply_unplanned_reflect_source_check(
    mut candidate: ReflectSuggestedFollowup,
) -> Option<ReflectSuggestedFollowup> {
    if !candidate.allow_unplanned_source_check {
        return None;
    }
    let query = candidate
        .search_query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .unwrap_or_else(|| candidate.title.trim())
        .to_string();
    if !reflect_external_search_query_is_safe(&query) {
        return None;
    }
    candidate.search_query = Some(query);
    candidate.search_requires_planning = false;
    candidate.source_strategy = ReflectFollowupSourceStrategy::PublicSearch;
    candidate.structured_context = serde_json::Value::Null;
    candidate.status = "queued".to_string();
    candidate.detail =
        "Reflect found a user-authored topic that may benefit from current public sources. Source enrichment is queued."
            .to_string();
    candidate.prompt = format!(
        "Use current source evidence to summarize this reflected topic and the next practical step: {}",
        candidate.title
    );
    candidate.rank_score += 4.0;
    Some(candidate)
}

async fn build_suggested_followups(
    storage: &Storage,
    clusters: &[ReflectClusterResponse],
    embedder: Option<&EmbeddingClient>,
    llm: Option<&LlmClient>,
    persist_feedback_vectors: bool,
) -> ReflectFollowupBuild {
    let now = chrono::Utc::now();
    let cache = load_reflect_followup_search_cache(storage).await;
    let mut plan_cache = load_reflect_followup_plan_cache(storage).await;
    let mut feedback_store = load_reflect_followup_feedback(storage).await;
    let mut candidates = Vec::new();
    let runs = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_recent_experience_runs_any_scope(REFLECT_SUGGESTION_EXPERIENCE_RUN_LIMIT),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .unwrap_or_default();
    for run in runs {
        if reflect_experience_run_failed(&run) {
            candidates.push(reflect_failure_suggestion(&run, now));
        }
        if reflect_experience_run_is_research(&run) {
            candidates.push(reflect_latest_suggestion(&run, &cache, now));
        }
    }
    let mut planned_cluster_ids = HashSet::<String>::new();
    if !clusters.is_empty() {
        if let Some(context) = build_reflect_semantic_freshness_context(embedder).await {
            let mut latest_candidates = clusters
                .iter()
                .filter_map(|cluster| {
                    let score = cluster.centroid_embedding.as_ref().and_then(|embedding| {
                        reflect_semantic_freshness_score(embedding, &context)
                    })?;
                    let suggestion =
                        reflect_semantic_cluster_latest_suggestion(cluster, &cache, now, score)?;
                    planned_cluster_ids.insert(cluster.id.clone());
                    Some(suggestion)
                })
                .collect::<Vec<_>>();
            latest_candidates.sort_by(|a, b| {
                b.rank_score
                    .total_cmp(&a.rank_score)
                    .then_with(|| b.occurred_at.cmp(&a.occurred_at))
                    .then_with(|| a.id.cmp(&b.id))
            });
            candidates.extend(latest_candidates);
        }
    }
    for cluster in clusters {
        if planned_cluster_ids.contains(&cluster.id) {
            continue;
        }
        if let Some(suggestion) = reflect_planned_cluster_latest_suggestion(cluster, &cache, now) {
            candidates.push(suggestion);
        }
    }
    for candidate in &mut candidates {
        let fallback_keys = reflect_followup_feedback_keys(
            &candidate.id,
            candidate.source_unit_id.as_deref(),
            candidate.conversation_id.as_deref(),
        );
        if candidate.feedback_keys.is_empty() {
            candidate.feedback_keys = fallback_keys;
        }
    }
    let plans = plan_reflect_external_pursuits(llm, &candidates).await;
    if !plans.is_empty() {
        let topics = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.id.clone(),
                    reflect_candidate_topic_for_cache(candidate),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let persisted = if persist_feedback_vectors {
            update_reflect_followup_plan_cache_guarded(storage, &plans, &topics, now).await
        } else {
            None
        };
        match persisted {
            Some(merged) => plan_cache = merged,
            None => {
                update_reflect_followup_plan_cache(&mut plan_cache, &plans, &topics, now);
            }
        }
    }
    let (kept, unplanned) = apply_reflect_external_pursuit_plans(candidates, &plans, &plan_cache);
    let mut candidates = kept;
    if persist_feedback_vectors
        && reflect_register_followup_feedback_vectors(&mut feedback_store, &candidates)
    {
        feedback_store.updated_at = Some(now.to_rfc3339());
        save_reflect_followup_feedback(storage, &feedback_store).await;
    }
    candidates = apply_reflect_followup_feedback_gates(candidates, &feedback_store, now);
    let unplanned = apply_reflect_followup_feedback_gates(unplanned, &feedback_store, now);
    ReflectFollowupBuild {
        followups: select_top_reflect_followups(candidates),
        unplanned,
    }
}

fn apply_reflect_followup_feedback_gates(
    candidates: Vec<ReflectSuggestedFollowup>,
    feedback_store: &ReflectFollowupFeedbackStore,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ReflectSuggestedFollowup> {
    candidates
        .into_iter()
        .filter_map(|mut candidate| {
            let feedback =
                reflect_followup_effective_feedback(&candidate.feedback_keys, feedback_store);
            let semantic_feedback = reflect_followup_semantic_feedback(
                candidate.feedback_vector.as_deref(),
                feedback_store,
            );
            let feedback = reflect_followup_refresh_feedback_for_new_evidence(
                feedback,
                &candidate.occurred_at,
            );
            let semantic_feedback = reflect_followup_refresh_feedback_for_new_evidence(
                semantic_feedback,
                &candidate.occurred_at,
            );
            if reflect_followup_is_dismissed(feedback.as_ref())
                || reflect_followup_is_dismissed(semantic_feedback.as_ref())
            {
                return None;
            }
            if reflect_followup_is_snoozed(feedback.as_ref(), now)
                || reflect_followup_is_snoozed(semantic_feedback.as_ref(), now)
            {
                return None;
            }
            if !feedback
                .as_ref()
                .is_some_and(|state| state.renewed_after_feedback)
            {
                candidate.rank_score += reflect_followup_feedback_weight(feedback.as_ref());
            }
            if !semantic_feedback
                .as_ref()
                .is_some_and(|state| state.renewed_after_feedback)
            {
                candidate.rank_score +=
                    reflect_followup_feedback_weight(semantic_feedback.as_ref());
            }
            if candidate.rank_score < 8.0 {
                return None;
            }
            candidate.feedback = reflect_feedback_for_response(feedback.or(semantic_feedback));
            Some(candidate)
        })
        .collect()
}

fn due_latest_followup_searches(
    suggestions: &[ReflectSuggestedFollowup],
    cache: &ReflectFollowupSearchCache,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ReflectDueFollowupSearch> {
    let mut seen = HashSet::<String>::new();
    suggestions
        .iter()
        .filter(|suggestion| suggestion.kind == "latest_developments")
        .filter(|suggestion| !suggestion.search_requires_planning)
        .filter_map(|suggestion| {
            let query = reflect_followup_search_query_for_strategy(suggestion)?;
            let cached_entry = reflect_search_cache_entry(cache, &suggestion.id);
            let cached_query_matches = cached_entry
                .map(|entry| {
                    reflect_search_queries_match_for_cache(&entry.query, &query)
                        && entry.source_strategy == suggestion.source_strategy
                })
                .unwrap_or(false);
            if query.is_empty()
                || !reflect_external_search_query_is_safe(&query)
                || (cached_query_matches
                    && !reflect_followup_search_is_due(cache, &suggestion.id, now))
            {
                return None;
            }
            if !seen.insert(suggestion.id.clone()) {
                return None;
            }
            Some(ReflectDueFollowupSearch {
                source_id: suggestion.id.clone(),
                query,
                source_strategy: suggestion.source_strategy,
                structured_context: suggestion.structured_context.clone(),
            })
        })
        .take(REFLECT_FOLLOWUP_BACKGROUND_SEARCH_LIMIT)
        .collect()
}

fn reflect_followup_search_query_for_strategy(
    suggestion: &ReflectSuggestedFollowup,
) -> Option<String> {
    let query = suggestion.search_query.as_deref()?.trim();
    match suggestion.source_strategy {
        ReflectFollowupSourceStrategy::PublicSearch => {
            reflect_external_search_query_is_safe(query).then(|| query.to_string())
        }
        ReflectFollowupSourceStrategy::FlightPriceDiscovery => {
            reflect_travel_price_search_query(query, &suggestion.structured_context)
        }
    }
}

fn due_latest_followup_summaries(
    suggestions: &[ReflectSuggestedFollowup],
    cache: &ReflectFollowupSearchCache,
) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    suggestions
        .iter()
        .filter(|suggestion| suggestion.kind == "latest_developments")
        .filter_map(|suggestion| {
            let entry = reflect_search_cache_entry(cache, &suggestion.id)?;
            if !reflect_followup_summary_is_due(entry) || !seen.insert(suggestion.id.clone()) {
                return None;
            }
            Some(suggestion.id.clone())
        })
        .take(REFLECT_FOLLOWUP_BACKGROUND_SEARCH_LIMIT)
        .collect()
}

fn prune_reflect_followup_search_cache(cache: &mut ReflectFollowupSearchCache) {
    const MAX_CACHE_ENTRIES: usize = 80;
    if cache.entries.len() <= MAX_CACHE_ENTRIES {
        return;
    }
    let mut keep = cache
        .entries
        .iter()
        .map(|(key, entry)| (key.clone(), entry.checked_at.clone()))
        .collect::<Vec<_>>();
    keep.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let keep = keep
        .into_iter()
        .take(MAX_CACHE_ENTRIES)
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    cache.entries.retain(|key, _| keep.contains(key));
}

fn reflect_cache_insert_search_entry(
    cache: &mut ReflectFollowupSearchCache,
    source_id: String,
    entry: ReflectFollowupSearchEntry,
) {
    cache.entries.insert(source_id, entry);
}

fn reflect_cache_apply_summary_if_current(
    cache: &mut ReflectFollowupSearchCache,
    source_id: &str,
    checked_at: &str,
    summary: Option<String>,
    summary_error: Option<String>,
    summary_evidence_supported: Option<bool>,
    generated_at: String,
) -> bool {
    let Some(current) = cache.entries.get_mut(source_id) else {
        return false;
    };
    if current.checked_at != checked_at || !reflect_followup_summary_is_due(current) {
        return false;
    }
    let has_attempt_result =
        summary.is_some() || summary_error.is_some() || summary_evidence_supported.is_some();
    current.summary = summary;
    current.summary_generated_at = has_attempt_result.then_some(generated_at);
    current.summary_error = summary_error;
    current.summary_evidence_supported = summary_evidence_supported;
    true
}

#[derive(Debug, Default, serde::Deserialize)]
struct ReflectFollowupSummaryVerdict {
    #[serde(default)]
    evidence_supported: Option<bool>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    unsupported_reason: String,
}

fn parse_reflect_followup_summary_verdict(response: &str) -> Option<ReflectFollowupSummaryVerdict> {
    let value = extract_reflect_json_value(response)?;
    serde_json::from_value::<ReflectFollowupSummaryVerdict>(value).ok()
}

async fn generate_reflect_followup_latest_summary(
    llm: &LlmClient,
    query: &str,
    checked_at: &str,
    source_strategy: ReflectFollowupSourceStrategy,
    results: &[ReflectFollowupSearchResult],
) -> (Option<String>, Option<String>, Option<bool>) {
    if results.is_empty() {
        return (None, None, None);
    }
    let source_context = results
        .iter()
        .take(REFLECT_FOLLOWUP_SEARCH_RESULTS_PER_TOPIC)
        .map(|result| {
            serde_json::json!({
                "title": truncate_chars(&result.title, 220),
                "source": truncate_chars(&result.source, 80),
                "published_date": result.published_date,
                "snippet": truncate_chars(&result.snippet, 420),
                "url": truncate_chars(&result.url, 260),
            })
        })
        .collect::<Vec<_>>();
    let context = serde_json::json!({
        "reflected_topic": truncate_chars(query, REFLECT_SUGGESTION_TEXT_CHARS),
        "checked_at": checked_at,
        "source_strategy": source_strategy,
        "sources": source_context,
    });
    let system_prompt = reflect_followup_summary_system_prompt(source_strategy);
    let user_message = format!(
        "Structured Reflect current-source check:\n{}\n\nReturn JSON only in this exact shape: {{\"evidence_supported\":true|false,\"summary\":\"2-4 bullet or short-paragraph news insight when supported, else empty\",\"unsupported_reason\":\"brief reason when evidence_supported is false, else empty\"}}.",
        serde_json::to_string_pretty(&context).unwrap_or_else(|_| "{}".to_string())
    );
    match tokio::time::timeout(
        REFLECT_FOLLOWUP_SUMMARY_TIMEOUT,
        llm.chat_with_system(system_prompt, &user_message),
    )
    .await
    {
        Ok(Ok(response)) => {
            let Some(verdict) = parse_reflect_followup_summary_verdict(&response.content) else {
                return (
                    None,
                    Some(
                        "Latest-development summary did not return a structured evidence verdict."
                            .to_string(),
                    ),
                    Some(false),
                );
            };
            if verdict.evidence_supported == Some(false) {
                let reason = verdict.unsupported_reason.trim();
                (
                    None,
                    Some(if reason.is_empty() {
                        "Source snippets do not directly support the requested topic.".to_string()
                    } else {
                        truncate_chars(reason, 240)
                    }),
                    Some(false),
                )
            } else if verdict.evidence_supported == Some(true) {
                let content = verdict.summary.trim();
                if content.chars().count() >= 16 {
                    (
                        Some(truncate_chars(content, REFLECT_FOLLOWUP_SUMMARY_MAX_CHARS)),
                        None,
                        Some(true),
                    )
                } else {
                    (
                        None,
                        Some("Latest-development summary was too short to use.".to_string()),
                        Some(false),
                    )
                }
            } else {
                (
                    None,
                    Some(
                        "Latest-development summary did not include an evidence verdict."
                            .to_string(),
                    ),
                    Some(false),
                )
            }
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "followup latest summary failed");
            (None, Some(error.to_string()), None)
        }
        Err(_) => {
            tracing::warn!("followup latest summary timed out");
            (
                None,
                Some("Latest-development summary timed out.".to_string()),
                None,
            )
        }
    }
}

fn reflect_followup_summary_system_prompt(
    source_strategy: ReflectFollowupSourceStrategy,
) -> &'static str {
    match source_strategy {
        ReflectFollowupSourceStrategy::PublicSearch => {
            "You synthesize Reflect current-source checks for a personal AI Agent OS. The reflected topic can be any subject inferred from the user's reflected activity and state. Use only the provided topic and source snippets. Do not invent facts, dates, causes, entities, attributes, constraints, or outcomes. Decide whether the snippets directly support the requested topic and current deliverable, not merely a broad adjacent subject. Background-only references, generic landing-page descriptions, index pages, or snippets that omit the requested relationship, event, comparison, constraint, or current development are not enough. If the snippets directly support a useful current insight, return JSON with evidence_supported=true and a concise user-facing summary in 2-4 bullets or one short paragraph. If they do not, return evidence_supported=false, summary empty, and a brief unsupported_reason. Avoid tables, citations markup, and generic advice. Return JSON only."
        }
        ReflectFollowupSourceStrategy::FlightPriceDiscovery => {
            "You synthesize Reflect flight price discovery checks for a personal AI Agent OS. Use only the provided route topic and source snippets. Generic web search results are not a complete calendar-price API. Do not claim the absolute cheapest flight, cheapest next flight in 365 days, live availability, exact fare, fare class, baggage rule, or booking outcome unless a source explicitly supports it as calendar-price or live fare data. Decide whether the snippets directly support the requested route and price-discovery task. If they do, return JSON with evidence_supported=true and a concise user-facing summary in 2-4 bullets or one short paragraph. If they are weak, adjacent, route-mismatched, only generic travel information, or not fare/calendar-price evidence, return evidence_supported=false, summary empty, and a brief unsupported_reason. Return JSON only."
        }
    }
}

async fn run_reflect_followup_search_job(
    storage: Storage,
    config_dir: std::path::PathBuf,
    searches: Vec<ReflectDueFollowupSearch>,
) -> std::result::Result<usize, String> {
    if searches.is_empty() {
        return Ok(0);
    }
    let search_config = crate::runtime::build_search_config(&config_dir, Some(&storage)).await;
    let mut completed = 0usize;
    for search in searches {
        let ReflectDueFollowupSearch {
            source_id,
            query,
            source_strategy,
            structured_context,
        } = search;
        let args = crate::actions::search::SearchArgs {
            query: query.clone(),
            num_results: REFLECT_FOLLOWUP_SEARCH_RESULTS_PER_TOPIC,
            backend: None,
            time_scope: Some(crate::actions::search::SearchTimeScope::Current),
        };
        let checked_at = chrono::Utc::now().to_rfc3339();
        let entry =
            match crate::actions::search::execute_search_response(&args, &search_config).await {
                Ok(response) => ReflectFollowupSearchEntry {
                    source_id: source_id.clone(),
                    query: response.query,
                    checked_at,
                    backend: Some(response.backend),
                    source_strategy,
                    structured_context,
                    results: response
                        .results
                        .into_iter()
                        .take(REFLECT_FOLLOWUP_SEARCH_RESULTS_PER_TOPIC)
                        .map(|result| ReflectFollowupSearchResult {
                            title: result.title,
                            url: result.url,
                            snippet: result.snippet,
                            source: result.source,
                            published_date: result.published_date,
                        })
                        .collect(),
                    error: None,
                    summary: None,
                    summary_generated_at: None,
                    summary_error: None,
                    summary_evidence_supported: None,
                },
                Err(error) => ReflectFollowupSearchEntry {
                    source_id: source_id.clone(),
                    query,
                    checked_at,
                    backend: None,
                    source_strategy,
                    structured_context,
                    results: Vec::new(),
                    error: Some(error.to_string()),
                    summary: None,
                    summary_generated_at: None,
                    summary_error: None,
                    summary_evidence_supported: None,
                },
            };
        if update_reflect_followup_search_cache(&storage, move |cache| {
            reflect_cache_insert_search_entry(cache, source_id, entry);
            true
        })
        .await
        .unwrap_or(false)
        {
            completed += 1;
        }
    }
    Ok(completed)
}

async fn run_reflect_followup_summary_job(
    storage: Storage,
    llm: LlmClient,
    source_ids: Vec<String>,
) -> std::result::Result<usize, String> {
    if source_ids.is_empty() {
        return Ok(0);
    }
    let mut completed = 0usize;
    for source_id in source_ids {
        let cache = load_reflect_followup_search_cache(&storage).await;
        let Some(entry) = cache.entries.get(&source_id).cloned() else {
            continue;
        };
        if !reflect_followup_summary_is_due(&entry) {
            continue;
        }
        let (summary, summary_error, summary_evidence_supported) =
            generate_reflect_followup_latest_summary(
                &llm,
                &entry.query,
                &entry.checked_at,
                entry.source_strategy,
                &entry.results,
            )
            .await;
        if update_reflect_followup_search_cache(&storage, move |cache| {
            reflect_cache_apply_summary_if_current(
                cache,
                &source_id,
                &entry.checked_at,
                summary,
                summary_error,
                summary_evidence_supported,
                chrono::Utc::now().to_rfc3339(),
            )
        })
        .await
        .unwrap_or(false)
        {
            completed += 1;
        }
    }
    Ok(completed)
}

async fn spawn_reflect_followup_summary_worker(
    storage: Storage,
    llm: LlmClient,
    source_ids: Vec<String>,
    trigger: &'static str,
) -> bool {
    if source_ids.is_empty()
        || REFLECT_FOLLOWUP_SUMMARY_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return false;
    }
    let in_flight_guard = ReflectFollowupSummaryInFlightGuard;
    let lease_owner = format!(
        "arkreflect-followup-summary:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let lease_guard = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.acquire_kv_lease_guard(
            REFLECT_FOLLOWUP_SUMMARY_LEASE_KEY,
            &lease_owner,
            REFLECT_FOLLOWUP_SUMMARY_LEASE_TTL_SECS,
        ),
    )
    .await
    {
        Ok(Ok(Some(guard))) => guard,
        Ok(Ok(None)) => {
            return false;
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "failed to acquire followup summary lease");
            return false;
        }
        Err(_) => {
            tracing::warn!("followup summary lease timed out");
            return false;
        }
    };
    crate::spawn_logged!(
        "src/channels/http/reflect_control.rs:followup_summary",
        async move {
            let _guard = in_flight_guard;
            let result = tokio::time::timeout(
                REFLECT_FOLLOWUP_SUMMARY_JOB_TIMEOUT,
                run_reflect_followup_summary_job(storage.clone(), llm, source_ids),
            )
            .await;
            match result {
                Ok(Ok(count)) => {
                    tracing::debug!(
                        trigger,
                        count,
                        "followup latest-summary background pass completed"
                    );
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        trigger,
                        error = %error,
                        "followup latest-summary background pass failed"
                    );
                }
                Err(_) => {
                    tracing::warn!(trigger, "followup latest-summary background pass timed out");
                }
            }
            if let Err(error) = storage
                .release_kv_lease_guard(REFLECT_FOLLOWUP_SUMMARY_LEASE_KEY, &lease_guard)
                .await
            {
                tracing::debug!(error = %error, "failed to release followup summary lease");
            }
        }
    );
    true
}

async fn spawn_reflect_followup_search_worker(
    storage: Storage,
    config_dir: std::path::PathBuf,
    llm: LlmClient,
    searches: Vec<ReflectDueFollowupSearch>,
    trigger: &'static str,
) -> bool {
    if searches.is_empty()
        || REFLECT_FOLLOWUP_SEARCH_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return false;
    }
    let in_flight_guard = ReflectFollowupSearchInFlightGuard;
    let source_ids = searches
        .iter()
        .map(|search| search.source_id.clone())
        .collect::<Vec<_>>();
    let lease_owner = format!(
        "arkreflect-followup:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let lease_guard = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.acquire_kv_lease_guard(
            REFLECT_FOLLOWUP_SEARCH_LEASE_KEY,
            &lease_owner,
            REFLECT_FOLLOWUP_SEARCH_LEASE_TTL_SECS,
        ),
    )
    .await
    {
        Ok(Ok(Some(guard))) => guard,
        Ok(Ok(None)) => {
            return false;
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "failed to acquire followup search lease");
            return false;
        }
        Err(_) => {
            tracing::warn!("followup search lease timed out");
            return false;
        }
    };
    crate::spawn_logged!(
        "src/channels/http/reflect_control.rs:followup_search",
        async move {
            let _guard = in_flight_guard;
            let result = tokio::time::timeout(
                REFLECT_FOLLOWUP_SEARCH_TIMEOUT,
                run_reflect_followup_search_job(storage.clone(), config_dir, searches),
            )
            .await;
            match result {
                Ok(Ok(count)) => {
                    tracing::debug!(
                        trigger,
                        count,
                        "followup latest-search background pass completed"
                    );
                    let _ = spawn_reflect_followup_summary_worker(
                        storage.clone(),
                        llm,
                        source_ids,
                        trigger,
                    )
                    .await;
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        trigger,
                        error = %error,
                        "followup latest-search background pass failed"
                    );
                }
                Err(_) => {
                    tracing::warn!(trigger, "followup latest-search background pass timed out");
                }
            }
            if let Err(error) = storage
                .release_kv_lease_guard(REFLECT_FOLLOWUP_SEARCH_LEASE_KEY, &lease_guard)
                .await
            {
                tracing::debug!(error = %error, "failed to release followup search lease");
            }
        }
    );
    true
}

fn reflect_due_followup_work(
    suggestions: &[ReflectSuggestedFollowup],
    cache: &ReflectFollowupSearchCache,
    now: chrono::DateTime<chrono::Utc>,
) -> ReflectDueFollowupWork {
    ReflectDueFollowupWork {
        searches: due_latest_followup_searches(suggestions, cache, now),
        summaries: due_latest_followup_summaries(suggestions, cache),
    }
}

async fn reflect_foreground_busy(state: &AppState) -> bool {
    let agent = state.agent.read().await;
    agent.active_message_request_count() > 0
}

/// Plan usefulness verdicts for the exact candidates a page view computed,
/// then start source checks for whatever survives. Unlike the idle-gated
/// background passes, this defers only to an active foreground chat, so a
/// viewed window converges to planned opportunities within a few polls.
async fn spawn_reflect_followup_planning_for_candidates(
    state: AppState,
    unplanned: Vec<ReflectSuggestedFollowup>,
    followups: Vec<ReflectSuggestedFollowup>,
    trigger: &'static str,
) -> bool {
    if REFLECT_REFRESH_IN_FLIGHT.load(Ordering::Acquire) {
        return false;
    }
    if REFLECT_FOLLOWUP_COORDINATOR_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let _guard = ReflectFollowupCoordinatorInFlightGuard;
    if reflect_foreground_busy(&state).await {
        return false;
    }
    let (storage, config_dir, llm) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.llm.clone(),
        )
    };
    let now = chrono::Utc::now();
    let mut survivors = followups;
    if !unplanned.is_empty() {
        let plans = plan_reflect_external_pursuits(Some(&llm), &unplanned).await;
        if !plans.is_empty() {
            let topics = unplanned
                .iter()
                .map(|candidate| {
                    (
                        candidate.id.clone(),
                        reflect_candidate_topic_for_cache(candidate),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let plan_cache =
                match update_reflect_followup_plan_cache_guarded(&storage, &plans, &topics, now)
                    .await
                {
                    Some(merged) => merged,
                    None => {
                        let mut cache = load_reflect_followup_plan_cache(&storage).await;
                        update_reflect_followup_plan_cache(&mut cache, &plans, &topics, now);
                        cache
                    }
                };
            let (kept, _) = apply_reflect_external_pursuit_plans(unplanned, &plans, &plan_cache);
            survivors.extend(kept);
        }
    }
    if survivors.is_empty() {
        return false;
    }
    let mut feedback_store = load_reflect_followup_feedback(&storage).await;
    if reflect_register_followup_feedback_vectors(&mut feedback_store, &survivors) {
        feedback_store.updated_at = Some(now.to_rfc3339());
        save_reflect_followup_feedback(&storage, &feedback_store).await;
    }
    let cache = load_reflect_followup_search_cache(&storage).await;
    let due_work = reflect_due_followup_work(&survivors, &cache, chrono::Utc::now());
    if !due_work.has_work() {
        return false;
    }
    let summary_spawned = spawn_reflect_followup_summary_worker(
        storage.clone(),
        llm.clone(),
        due_work.summaries,
        trigger,
    )
    .await;
    let search_spawned =
        spawn_reflect_followup_search_worker(storage, config_dir, llm, due_work.searches, trigger)
            .await;
    summary_spawned || search_spawned
}

async fn maybe_spawn_reflect_followup_search(
    state: AppState,
    suggestions: Vec<ReflectSuggestedFollowup>,
    trigger: &'static str,
) -> bool {
    if suggestions.is_empty() || REFLECT_REFRESH_IN_FLIGHT.load(Ordering::Acquire) {
        return false;
    }
    if REFLECT_FOLLOWUP_COORDINATOR_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let _guard = ReflectFollowupCoordinatorInFlightGuard;
    if reflect_foreground_busy(&state).await {
        return false;
    }
    let (storage, config_dir, llm) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.llm.clone(),
        )
    };
    let cache = load_reflect_followup_search_cache(&storage).await;
    let due_work = reflect_due_followup_work(&suggestions, &cache, chrono::Utc::now());
    if !due_work.has_work() {
        return false;
    }
    let summary_spawned = spawn_reflect_followup_summary_worker(
        storage.clone(),
        llm.clone(),
        due_work.summaries,
        trigger,
    )
    .await;
    let search_spawned =
        spawn_reflect_followup_search_worker(storage, config_dir, llm, due_work.searches, trigger)
            .await;
    summary_spawned || search_spawned
}

async fn maybe_spawn_reflect_followup_search_for_range(
    state: AppState,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    trigger: &'static str,
) -> bool {
    if !reflect_server_is_idle(&state).await {
        return false;
    }
    let (storage, embedding_client, llm) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.embedding_client.clone(),
            agent.llm.clone(),
        )
    };
    let from_s = from.to_rfc3339();
    let to_s = to.to_rfc3339();
    let units = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_semantic_work_units_between(&from_s, &to_s, REFLECT_MAX_UNITS),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .unwrap_or_default();
    let (clusters, _, _) = build_clusters_bounded(units).await;
    let build = build_suggested_followups(
        &storage,
        &clusters,
        embedding_client.as_deref(),
        Some(&llm),
        true,
    )
    .await;
    maybe_spawn_reflect_followup_search(state, build.followups, trigger).await
}

async fn maybe_spawn_reflect_followup_search_from_recent_activity(
    state: AppState,
    trigger: &'static str,
) -> bool {
    if !reflect_server_is_idle(&state).await {
        return false;
    }
    let (storage, embedding_client, llm) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.embedding_client.clone(),
            agent.llm.clone(),
        )
    };
    let now = chrono::Utc::now();
    let from_s = (now - chrono::Duration::days(REFLECT_IDLE_LOOKBACK_DAYS)).to_rfc3339();
    let to_s = now.to_rfc3339();
    let units = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_semantic_work_units_between(&from_s, &to_s, REFLECT_MAX_UNITS),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .unwrap_or_default();
    let (clusters, _, _) = build_clusters_bounded(units).await;
    let build = build_suggested_followups(
        &storage,
        &clusters,
        embedding_client.as_deref(),
        Some(&llm),
        true,
    )
    .await;
    maybe_spawn_reflect_followup_search(state, build.followups, trigger).await
}

fn message_excerpt(messages: &[message::Model], role: &str, reverse: bool) -> String {
    let iter: Box<dyn Iterator<Item = &message::Model>> = if reverse {
        Box::new(messages.iter().rev())
    } else {
        Box::new(messages.iter())
    };
    iter.filter(|message| message.role == role)
        .map(|message| message.content.trim())
        .find(|content| !content.is_empty())
        .map(|content| truncate_chars(content, REFLECT_PREVIEW_CHARS))
        .unwrap_or_default()
}

fn distinct_recent_reflect_fragments<'a>(
    values: impl Iterator<Item = &'a str>,
    max_fragments: usize,
    max_chars_each: usize,
) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut fragments = Vec::new();
    for value in values {
        let fragment = reflect_sentence_fragment(value, max_chars_each);
        if fragment.is_empty() {
            continue;
        }
        let key = fragment.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        fragments.push(fragment);
        if fragments.len() >= max_fragments {
            break;
        }
    }
    fragments.reverse();
    fragments
}

fn message_role_focus(messages: &[message::Model], role: &str) -> String {
    distinct_recent_reflect_fragments(
        messages
            .iter()
            .rev()
            .filter(|message| message.role == role)
            .map(|message| message.content.as_str()),
        4,
        REFLECT_PREVIEW_CHARS,
    )
    .join(" / ")
}

fn orbit_role_focus(messages: &[OrbitChatMessage], role: &str) -> String {
    distinct_recent_reflect_fragments(
        messages
            .iter()
            .rev()
            .filter(|message| message.role == role)
            .map(|message| message.content.as_str()),
        4,
        REFLECT_PREVIEW_CHARS,
    )
    .join(" / ")
}

fn orbit_message_excerpt(messages: &[OrbitChatMessage], role: &str, reverse: bool) -> String {
    let iter: Box<dyn Iterator<Item = &OrbitChatMessage>> = if reverse {
        Box::new(messages.iter().rev())
    } else {
        Box::new(messages.iter())
    };
    iter.filter(|message| message.role == role)
        .map(|message| message.content.trim())
        .find(|content| !content.is_empty())
        .map(|content| truncate_chars(content, REFLECT_PREVIEW_CHARS))
        .unwrap_or_default()
}

fn filter_orbit_messages_between(
    messages: Vec<OrbitChatMessage>,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Vec<OrbitChatMessage> {
    messages
        .into_iter()
        .filter(|message| in_window(&message.created_at, from, to))
        .take(REFLECT_MAX_MESSAGES_PER_CONVERSATION as usize)
        .collect()
}

fn conversation_candidate(
    conversation: &conversation::Model,
    messages: Vec<message::Model>,
) -> Option<ReflectCandidateUnit> {
    if messages.is_empty() {
        return None;
    }
    let first_user = message_excerpt(&messages, "user", false);
    let last_user = message_excerpt(&messages, "user", true);
    let last_assistant = message_excerpt(&messages, "assistant", true);
    let user_focus = message_role_focus(&messages, "user");
    let assistant_context = message_role_focus(&messages, "assistant");
    let title = first_non_empty([
        user_focus.as_str(),
        first_user.as_str(),
        last_user.as_str(),
        conversation.title.as_str(),
    ]);
    let summary = first_non_empty([
        user_focus.as_str(),
        last_user.as_str(),
        first_user.as_str(),
        conversation.title.as_str(),
        last_assistant.as_str(),
    ]);
    let content_preview = first_non_empty([
        last_user.as_str(),
        user_focus.as_str(),
        first_user.as_str(),
        summary.as_str(),
    ]);
    let mut transcript = String::new();
    for message in messages
        .iter()
        .take(REFLECT_MAX_MESSAGES_PER_CONVERSATION as usize)
    {
        if !message.content.trim().is_empty() {
            transcript.push_str(&message.role);
            transcript.push_str(": ");
            transcript.push_str(message.content.trim());
            transcript.push('\n');
        }
        if transcript.chars().count() >= REFLECT_EMBED_TEXT_CHARS {
            break;
        }
    }
    let first_at = messages
        .first()
        .map(|message| message.timestamp.clone())
        .unwrap_or_else(|| conversation.created_at.clone());
    let last_at = messages
        .last()
        .map(|message| message.timestamp.clone())
        .unwrap_or_else(|| conversation.updated_at.clone());
    let embedding_text = truncate_chars(
        &format!(
            "Title: {}\nChannel: {}\nUser focus: {}\nAssistant context: {}\nConversation content:\n{}",
            title, conversation.channel, summary, assistant_context, transcript
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "conversation".to_string(),
        source_id: format!(
            "{}:{}",
            conversation.id,
            day_key(&last_at).unwrap_or_else(|| stable_hash(&last_at).chars().take(10).collect())
        ),
        conversation_id: Some(conversation.id.clone()),
        project_id: conversation.project_id.clone(),
        channel: conversation.channel.clone(),
        title,
        summary,
        content_preview,
        embedding_text,
        occurred_at: last_at.clone(),
        period_start: Some(first_at),
        period_end: Some(last_at),
        message_count: messages.len().min(i32::MAX as usize) as i32,
        metadata: serde_json::json!({ "conversation_id": conversation.id }),
        inherited_embedding: None,
    })
}

fn conversation_candidates(
    conversation: &conversation::Model,
    messages: Vec<message::Model>,
) -> Vec<ReflectCandidateUnit> {
    let mut by_day = BTreeMap::<String, Vec<message::Model>>::new();
    for message in messages {
        let Some(day) = day_key(&message.timestamp) else {
            continue;
        };
        by_day.entry(day).or_default().push(message);
    }
    by_day
        .into_values()
        .filter_map(|daily_messages| conversation_candidate(conversation, daily_messages))
        .collect()
}

fn orbit_candidate(
    orbit: &Orbit,
    transcript: &OrbitChatTranscriptSummary,
    messages: Vec<OrbitChatMessage>,
) -> Option<ReflectCandidateUnit> {
    if messages.is_empty() {
        return None;
    }
    let first_user = orbit_message_excerpt(&messages, "user", false);
    let last_user = orbit_message_excerpt(&messages, "user", true);
    let last_assistant = orbit_message_excerpt(&messages, "assistant", true);
    let user_focus = orbit_role_focus(&messages, "user");
    let assistant_context = orbit_role_focus(&messages, "assistant");
    let transcript_title = first_non_empty([
        user_focus.as_str(),
        first_user.as_str(),
        last_user.as_str(),
        transcript.title.as_str(),
    ]);
    let title = format!("{}: {}", orbit.name.trim(), transcript_title);
    let summary = first_non_empty([
        user_focus.as_str(),
        last_user.as_str(),
        first_user.as_str(),
        transcript_title.as_str(),
        last_assistant.as_str(),
    ]);
    let content_preview = first_non_empty([
        last_user.as_str(),
        user_focus.as_str(),
        first_user.as_str(),
        summary.as_str(),
    ]);
    let mut transcript_text = String::new();
    for message in messages
        .iter()
        .take(REFLECT_MAX_MESSAGES_PER_CONVERSATION as usize)
    {
        if !message.content.trim().is_empty() {
            transcript_text.push_str(&message.role);
            transcript_text.push_str(": ");
            transcript_text.push_str(message.content.trim());
            transcript_text.push('\n');
        }
        if transcript_text.chars().count() >= REFLECT_EMBED_TEXT_CHARS {
            break;
        }
    }
    let first_at = messages
        .first()
        .map(|message| message.created_at.clone())
        .unwrap_or_else(|| transcript.created_at.clone());
    let last_at = messages
        .last()
        .map(|message| message.created_at.clone())
        .unwrap_or_else(|| transcript.updated_at.clone());
    let embedding_text = truncate_chars(
        &format!(
            "Orbit: {}\nTranscript: {}\nUser focus: {}\nAssistant context: {}\nOrbit chat content:\n{}",
            orbit.name, transcript_title, summary, assistant_context, transcript_text
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "orbit_chat".to_string(),
        source_id: format!(
            "{}:{}:{}",
            orbit.id,
            transcript.id,
            day_key(&last_at).unwrap_or_else(|| stable_hash(&last_at).chars().take(10).collect())
        ),
        conversation_id: None,
        project_id: None,
        channel: "arkorbit".to_string(),
        title,
        summary,
        content_preview,
        embedding_text,
        occurred_at: last_at.clone(),
        period_start: Some(first_at),
        period_end: Some(last_at),
        message_count: messages.len().min(i32::MAX as usize) as i32,
        metadata: serde_json::json!({
            "orbit_id": orbit.id,
            "orbit_name": orbit.name,
            "transcript_id": transcript.id,
            "current": transcript.current
        }),
        inherited_embedding: None,
    })
}

fn orbit_candidates(
    orbit: &Orbit,
    transcript: &OrbitChatTranscriptSummary,
    messages: Vec<OrbitChatMessage>,
) -> Vec<ReflectCandidateUnit> {
    let mut by_day = BTreeMap::<String, Vec<OrbitChatMessage>>::new();
    for message in messages {
        let Some(day) = day_key(&message.created_at) else {
            continue;
        };
        by_day.entry(day).or_default().push(message);
    }
    by_day
        .into_values()
        .filter_map(|daily_messages| orbit_candidate(orbit, transcript, daily_messages))
        .collect()
}

fn experience_item_candidate(item: experience_item::Model) -> ReflectCandidateUnit {
    let title = first_non_empty([item.title.as_str(), item.content.as_str()]);
    let summary = first_non_empty([item.content.as_str(), item.title.as_str()]);
    let updated_at = item.updated_at.clone();
    let embedding_text = truncate_chars(
        &format!(
            "Memory kind: {}\nTitle: {}\nContent: {}\nMetadata: {}",
            item.kind, title, summary, item.metadata
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    ReflectCandidateUnit {
        source_kind: "experience_item".to_string(),
        source_id: item.id,
        conversation_id: item.conversation_id,
        project_id: item.project_id,
        channel: "memory".to_string(),
        title,
        summary: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        embedding_text,
        occurred_at: updated_at.clone(),
        period_start: item.last_supported_at.clone(),
        period_end: Some(updated_at),
        message_count: item.support_count,
        metadata: serde_json::json!({
            "kind": item.kind,
            "confidence": item.confidence,
            "support_count": item.support_count
        }),
        inherited_embedding: item.embedding,
    }
}

fn procedural_pattern_candidate(pattern: procedural_pattern::Model) -> ReflectCandidateUnit {
    let title = first_non_empty([
        pattern.title.as_str(),
        pattern.trigger_summary.as_str(),
        pattern.summary.as_str(),
    ]);
    let summary = first_non_empty([
        pattern.summary.as_str(),
        pattern.trigger_summary.as_str(),
        pattern.title.as_str(),
    ]);
    let updated_at = pattern.updated_at.clone();
    let embedding_text = truncate_chars(
        &format!(
            "Workflow: {}\nTrigger: {}\nSummary: {}\nSteps: {}\nTools: {}",
            title,
            pattern.trigger_summary,
            pattern.summary,
            pattern.steps_json,
            pattern.tool_sequence_json
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    ReflectCandidateUnit {
        source_kind: "procedural_pattern".to_string(),
        source_id: pattern.id,
        conversation_id: pattern.conversation_id,
        project_id: pattern.project_id,
        channel: "learning".to_string(),
        title,
        summary: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        embedding_text,
        occurred_at: updated_at.clone(),
        period_start: pattern.last_validated_at.clone(),
        period_end: Some(updated_at),
        message_count: pattern.sample_count,
        metadata: serde_json::json!({
            "intent_key": pattern.intent_key,
            "sample_count": pattern.sample_count,
            "success_rate": pattern.success_rate
        }),
        inherited_embedding: None,
    }
}

fn app_candidate(
    app: &serde_json::Value,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    let id = app.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let created_at = app
        .get("created_at")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if !in_window(created_at, from, to) {
        return None;
    }
    let title = first_non_empty([
        app.get("title")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        id,
    ]);
    let runtime_mode = app
        .get("runtime_mode")
        .and_then(|value| value.as_str())
        .unwrap_or("registered");
    let quality = app
        .get("quality_report_status")
        .and_then(|value| value.as_str())
        .unwrap_or("unavailable");
    let summary = format!(
        "Built app '{}' with runtime {} and quality status {}.",
        title, runtime_mode, quality
    );
    let embedding_text = truncate_chars(
        &format!(
            "App built: {}\nRuntime: {}\nQuality: {}\nStatic: {}\nEnabled: {}",
            title,
            runtime_mode,
            quality,
            app.get("is_static")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            app.get("enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(true)
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "app".to_string(),
        source_id: id.to_string(),
        conversation_id: None,
        project_id: None,
        channel: "apps".to_string(),
        title,
        summary: summary.clone(),
        content_preview: summary,
        embedding_text,
        occurred_at: created_at.to_string(),
        period_start: Some(created_at.to_string()),
        period_end: Some(created_at.to_string()),
        message_count: 1,
        metadata: serde_json::json!({
            "app_id": id,
            "runtime_mode": runtime_mode,
            "quality_report_status": quality
        }),
        inherited_embedding: None,
    })
}

fn goal_candidate(
    row: task::Model,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    if !row.action.eq_ignore_ascii_case("goal") || !in_window(&row.updated_at, from, to) {
        return None;
    }
    let args = serde_json::from_str::<serde_json::Value>(&row.arguments)
        .unwrap_or(serde_json::Value::Null);
    let goal_text = args
        .get("goal")
        .and_then(|value| value.as_str())
        .unwrap_or(row.description.as_str())
        .to_string();
    let goal_id = args
        .get("goal_id")
        .and_then(|value| value.as_str())
        .unwrap_or(row.id.as_str())
        .to_string();
    let title = first_non_empty([goal_text.as_str(), row.description.as_str()]);
    let status = serde_json::from_str::<serde_json::Value>(&row.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or(row.status.clone());
    let due = row
        .scheduled_for
        .as_deref()
        .map(|value| format!(" Due {}.", value))
        .unwrap_or_default();
    let summary = format!("Tracked goal is currently {}.{}", status, due);
    let embedding_text = truncate_chars(
        &format!(
            "Goal: {}\nStatus: {}\nDue: {}\nResult: {}",
            title,
            status,
            row.scheduled_for.as_deref().unwrap_or("not set"),
            row.result.as_deref().unwrap_or("")
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "goal".to_string(),
        source_id: goal_id.clone(),
        conversation_id: None,
        project_id: None,
        channel: "goals".to_string(),
        title,
        summary: summary.clone(),
        content_preview: summary,
        embedding_text,
        occurred_at: row.updated_at.clone(),
        period_start: Some(row.created_at.clone()),
        period_end: Some(row.updated_at),
        message_count: 1,
        metadata: serde_json::json!({
            "task_id": row.id,
            "goal_id": goal_id,
            "status": status,
            "scheduled_for": row.scheduled_for
        }),
        inherited_embedding: None,
    })
}

fn watcher_status_label(status: &crate::core::automation::watcher::WatcherStatus) -> String {
    match status {
        crate::core::automation::watcher::WatcherStatus::Active => "active".to_string(),
        crate::core::automation::watcher::WatcherStatus::Paused => "paused".to_string(),
        crate::core::automation::watcher::WatcherStatus::Triggered => "triggered".to_string(),
        crate::core::automation::watcher::WatcherStatus::TimedOut => "timed out".to_string(),
        crate::core::automation::watcher::WatcherStatus::Cancelled => "cancelled".to_string(),
        crate::core::automation::watcher::WatcherStatus::Failed { .. } => "failed".to_string(),
    }
}

fn watcher_candidate(
    watcher: crate::core::automation::watcher::Watcher,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    let occurred = watcher
        .last_poll_at
        .unwrap_or(watcher.created_at)
        .to_rfc3339();
    if !in_window(&occurred, from, to) {
        return None;
    }
    let status = watcher_status_label(&watcher.status);
    let condition_summary = watcher.condition.summary();
    let title = first_non_empty([watcher.description.as_str(), condition_summary.as_str()]);
    let summary = format!(
        "Watcher is {} after {} poll{}.",
        status,
        watcher.poll_count,
        if watcher.poll_count == 1 { "" } else { "s" }
    );
    let embedding_text = truncate_chars(
        &format!(
            "Watcher: {}\nPoll action: {}\nCondition: {}\nStatus: {}\nOn trigger: {}\nLast outcome: {:?}",
            title,
            watcher.poll_action,
            condition_summary,
            status,
            watcher.on_trigger,
            watcher.last_poll_outcome
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "watcher".to_string(),
        source_id: watcher.id.to_string(),
        conversation_id: None,
        project_id: None,
        channel: "background".to_string(),
        title,
        summary: summary.clone(),
        content_preview: watcher
            .last_result
            .as_deref()
            .or(watcher.last_error.as_deref())
            .map(|value| truncate_chars(value, REFLECT_PREVIEW_CHARS))
            .unwrap_or(summary),
        embedding_text,
        occurred_at: occurred,
        period_start: Some(watcher.created_at.to_rfc3339()),
        period_end: watcher.last_poll_at.map(|value| value.to_rfc3339()),
        message_count: watcher.poll_count.min(i32::MAX as u32) as i32,
        metadata: serde_json::json!({
            "watcher_id": watcher.id.to_string(),
            "status": status,
            "poll_action": watcher.poll_action,
            "poll_count": watcher.poll_count,
            "repeat_on_match": watcher.repeat_on_match
        }),
        inherited_embedding: None,
    })
}

fn supervisor_watcher_candidate(
    state: crate::core::automation::AutomationSupervisorState,
    live_watcher_ids: &HashSet<String>,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    if state.automation_kind != "watcher" || live_watcher_ids.contains(&state.automation_id) {
        return None;
    }
    let occurred = state
        .last_run_at
        .clone()
        .or_else(|| state.created_at.clone())?;
    if !in_window(&occurred, from, to) {
        return None;
    }
    let title = first_non_empty([state.title.as_str(), state.action.as_str()]);
    let summary = format!(
        "Historical watcher is {} after {} attempt{}.",
        state.status,
        state.attempt_count,
        if state.attempt_count == 1 { "" } else { "s" }
    );
    let embedding_text = truncate_chars(
        &format!(
            "Watcher history: {}\nAction: {}\nStatus: {}\nLast error: {}",
            title,
            state.action,
            state.status,
            state.last_error.as_deref().unwrap_or("")
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "watcher".to_string(),
        source_id: state.automation_id.clone(),
        conversation_id: state.origin.conversation_id,
        project_id: state.origin.project_id,
        channel: "background".to_string(),
        title,
        summary: summary.clone(),
        content_preview: state
            .last_error
            .as_deref()
            .map(|value| truncate_chars(value, REFLECT_PREVIEW_CHARS))
            .unwrap_or(summary),
        embedding_text,
        occurred_at: occurred,
        period_start: state.created_at,
        period_end: state.last_run_at,
        message_count: state.attempt_count.min(i32::MAX as u32) as i32,
        metadata: serde_json::json!({
            "watcher_id": state.automation_id,
            "status": state.status,
            "history_only": true
        }),
        inherited_embedding: None,
    })
}

fn sentinel_observation_candidate(
    item: sentinel_panel::SentinelObservation,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    if !in_window(&item.updated_at, from, to) {
        return None;
    }
    let title = first_non_empty([item.title.as_str(), item.kind.as_str()]);
    let summary = first_non_empty([item.detail.as_str(), item.title.as_str()]);
    let embedding_text = truncate_chars(
        &format!(
            "Sentinel observation: {}\nKind: {}\nDetail: {}\nSource: {}",
            title,
            item.kind,
            item.detail,
            item.source_label.as_deref().unwrap_or("")
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "sentinel".to_string(),
        source_id: format!("observation:{}", item.id),
        conversation_id: None,
        project_id: None,
        channel: "sentinel".to_string(),
        title,
        summary: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        embedding_text,
        occurred_at: item.updated_at.clone(),
        period_start: Some(item.created_at),
        period_end: Some(item.updated_at),
        message_count: 1,
        metadata: serde_json::json!({
            "sentinel_id": item.id,
            "kind": item.kind,
            "priority": item.priority,
            "confidence": item.confidence
        }),
        inherited_embedding: None,
    })
}

fn sentinel_proposal_candidate(
    item: sentinel_panel::SentinelProposal,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    if !in_window(&item.updated_at, from, to) {
        return None;
    }
    let title = first_non_empty([item.title.as_str(), item.proposal_kind.as_str()]);
    let summary = first_non_empty([
        item.detail.as_str(),
        item.rationale.as_str(),
        item.title.as_str(),
    ]);
    let embedding_text = truncate_chars(
        &format!(
            "Sentinel proposal: {}\nKind: {}\nStatus: {}\nDetail: {}\nRationale: {}\nLast run: {}",
            title,
            item.proposal_kind,
            item.status,
            item.detail,
            item.rationale,
            item.last_run_summary.as_deref().unwrap_or("")
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "sentinel".to_string(),
        source_id: format!("proposal:{}", item.id),
        conversation_id: None,
        project_id: None,
        channel: "sentinel".to_string(),
        title,
        summary: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        embedding_text,
        occurred_at: item.updated_at.clone(),
        period_start: Some(item.created_at),
        period_end: Some(item.updated_at),
        message_count: 1,
        metadata: serde_json::json!({
            "sentinel_id": item.id,
            "proposal_kind": item.proposal_kind,
            "status": item.status,
            "priority": item.priority,
            "confidence": item.confidence
        }),
        inherited_embedding: None,
    })
}

fn arkpulse_candidate(event: arkpulse_event::Model) -> ReflectCandidateUnit {
    let title = first_non_empty([event.message.as_str(), event.status.as_str()]);
    let summary = first_non_empty([event.summary.as_str(), event.message.as_str()]);
    let embedding_text = truncate_chars(
        &format!(
            "Pulse status: {}\nMessage: {}\nSummary: {}\nFlags: {}\nDetails: {}",
            event.status, event.message, event.summary, event.flags_json, event.details_json
        ),
        REFLECT_EMBED_TEXT_CHARS,
    );
    ReflectCandidateUnit {
        source_kind: "arkpulse".to_string(),
        source_id: event.id,
        conversation_id: None,
        project_id: None,
        channel: "arkpulse".to_string(),
        title,
        summary: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        embedding_text,
        occurred_at: event.timestamp.clone(),
        period_start: Some(event.timestamp.clone()),
        period_end: Some(event.timestamp),
        message_count: event
            .overdue_tasks
            .saturating_add(event.failed_tasks)
            .max(1),
        metadata: serde_json::json!({
            "status": event.status,
            "overdue_tasks": event.overdue_tasks,
            "failed_tasks": event.failed_tasks
        }),
        inherited_embedding: None,
    }
}

fn lineage_time(value: &serde_json::Value) -> Option<String> {
    [
        "timestamp",
        "created_at",
        "started_at",
        "completed_at",
        "promoted_at",
        "evaluated_at",
        "updated_at",
    ]
    .into_iter()
    .find_map(|key| {
        value
            .get(key)
            .and_then(|entry| entry.as_str())
            .map(str::to_string)
    })
}

fn lineage_candidate(
    kind: &str,
    value: serde_json::Value,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Option<ReflectCandidateUnit> {
    let occurred = lineage_time(&value)?;
    if !in_window(&occurred, from, to) {
        return None;
    }
    let raw = serde_json::to_string(&value).unwrap_or_default();
    let id = value
        .get("id")
        .or_else(|| value.get("lineage_entry_id"))
        .or_else(|| value.get("candidate_version"))
        .or_else(|| value.get("version"))
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stable_hash(&raw).chars().take(32).collect());
    let title = first_non_empty([
        value
            .get("title")
            .and_then(|entry| entry.as_str())
            .unwrap_or_default(),
        value
            .get("summary")
            .and_then(|entry| entry.as_str())
            .unwrap_or_default(),
        value
            .get("status")
            .and_then(|entry| entry.as_str())
            .unwrap_or_default(),
        kind,
    ]);
    let summary = first_non_empty([
        value
            .get("summary")
            .and_then(|entry| entry.as_str())
            .unwrap_or_default(),
        value
            .get("decision")
            .and_then(|entry| entry.as_str())
            .unwrap_or_default(),
        raw.as_str(),
    ]);
    let embedding_text = truncate_chars(
        &format!("Evolve lineage kind: {}\nEntry: {}", kind, raw),
        REFLECT_EMBED_TEXT_CHARS,
    );
    Some(ReflectCandidateUnit {
        source_kind: "arkevolve".to_string(),
        source_id: format!("{}:{}", kind, id),
        conversation_id: None,
        project_id: None,
        channel: "arkevolve".to_string(),
        title,
        summary: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&summary, REFLECT_PREVIEW_CHARS),
        embedding_text,
        occurred_at: occurred.clone(),
        period_start: Some(occurred.clone()),
        period_end: Some(occurred),
        message_count: 1,
        metadata: serde_json::json!({
            "lineage_kind": kind
        }),
        inherited_embedding: None,
    })
}

#[derive(Default)]
struct UsageBucket {
    requests: i32,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    cost_usd: f64,
    missing_cost: bool,
    models: BTreeMap<String, i32>,
    channels: BTreeMap<String, i32>,
}

fn usage_candidates(rows: Vec<llm_usage::Model>) -> Vec<ReflectCandidateUnit> {
    let mut buckets = BTreeMap::<String, UsageBucket>::new();
    for row in rows {
        let Some(dt) = parse_time(&row.created_at) else {
            continue;
        };
        let day = dt.format("%Y-%m-%d").to_string();
        let bucket = buckets.entry(day).or_default();
        bucket.requests += 1;
        bucket.prompt_tokens += row.prompt_tokens.max(0) as i64;
        bucket.completion_tokens += row.completion_tokens.max(0) as i64;
        bucket.total_tokens += row.total_tokens.max(0) as i64;
        if let Some(cost) = row.cost_usd {
            bucket.cost_usd += cost.max(0.0);
        } else {
            bucket.missing_cost = true;
        }
        *bucket.models.entry(row.model).or_default() += 1;
        *bucket.channels.entry(row.channel).or_default() += 1;
    }
    buckets
        .into_iter()
        .map(|(day, bucket)| {
            let top_model = bucket
                .models
                .iter()
                .max_by_key(|(_, count)| **count)
                .map(|(model, _)| model.clone())
                .unwrap_or_else(|| "unknown model".to_string());
            let top_channel = bucket
                .channels
                .iter()
                .max_by_key(|(_, count)| **count)
                .map(|(channel, _)| channel.clone())
                .unwrap_or_else(|| "unknown channel".to_string());
            let occurred_at = format!("{}T12:00:00Z", day);
            let cost_text = if bucket.missing_cost {
                "partial cost data".to_string()
            } else {
                format!("${:.4}", bucket.cost_usd)
            };
            let title = format!("LLM usage on {}", day);
            let summary = format!(
                "{} request{} used {} tokens with {}.",
                bucket.requests,
                if bucket.requests == 1 { "" } else { "s" },
                bucket.total_tokens,
                cost_text
            );
            let embedding_text = truncate_chars(
                &format!(
                    "Usage day: {}\nRequests: {}\nTotal tokens: {}\nPrompt tokens: {}\nCompletion tokens: {}\nTop model: {}\nTop channel: {}\nCost: {}",
                    day,
                    bucket.requests,
                    bucket.total_tokens,
                    bucket.prompt_tokens,
                    bucket.completion_tokens,
                    top_model,
                    top_channel,
                    cost_text
                ),
                REFLECT_EMBED_TEXT_CHARS,
            );
            ReflectCandidateUnit {
                source_kind: "llm_usage".to_string(),
                source_id: day.clone(),
                conversation_id: None,
                project_id: None,
                channel: "analytics".to_string(),
                title,
                summary: summary.clone(),
                content_preview: summary,
                embedding_text,
                occurred_at,
                period_start: Some(format!("{}T00:00:00Z", day)),
                period_end: Some(format!("{}T23:59:59Z", day)),
                message_count: bucket.requests,
                metadata: serde_json::json!({
                    "requests": bucket.requests,
                    "prompt_tokens": bucket.prompt_tokens,
                    "completion_tokens": bucket.completion_tokens,
                    "total_tokens": bucket.total_tokens,
                    "cost_usd": if bucket.missing_cost { serde_json::Value::Null } else { serde_json::json!(bucket.cost_usd) },
                    "top_model": top_model,
                    "top_channel": top_channel
                }),
                inherited_embedding: None,
            }
        })
        .collect()
}

async fn embedding_for_candidate(
    storage: &Storage,
    embedder: Option<&EmbeddingClient>,
    candidate: &ReflectCandidateUnit,
    id: &str,
    text_hash: &str,
) -> Option<PgVector> {
    if let Ok(Ok(Some(existing))) =
        tokio::time::timeout(REFLECT_DB_TIMEOUT, storage.get_semantic_work_unit(id)).await
    {
        if existing.text_hash == text_hash {
            if let Some(embedding) = existing.embedding {
                return Some(embedding);
            }
        }
    }
    if let Some(embedding) = candidate.inherited_embedding.clone() {
        return Some(embedding);
    }
    let embedder = embedder?;
    let input = candidate.embedding_text.trim();
    if input.is_empty() {
        return None;
    }
    match tokio::time::timeout(
        REFLECT_EMBED_TIMEOUT,
        embedder.embed_texts(&[input.to_string()]),
    )
    .await
    {
        Ok(Ok(mut embeddings)) => embeddings.pop(),
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "semantic work unit embedding failed");
            None
        }
        Err(_) => {
            tracing::warn!("semantic work unit embedding timed out");
            None
        }
    }
}

async fn upsert_candidate(
    storage: &Storage,
    embedder: Option<&EmbeddingClient>,
    candidate: ReflectCandidateUnit,
) -> bool {
    let id = stable_unit_id(&candidate.source_kind, &candidate.source_id);
    let text_hash = stable_hash(&candidate.embedding_text);
    let embedding = embedding_for_candidate(storage, embedder, &candidate, &id, &text_hash).await;
    let now = chrono::Utc::now().to_rfc3339();
    let mut metadata = candidate.metadata;
    if let Some(object) = metadata.as_object_mut() {
        if let Some(embedding) = embedding.as_ref() {
            object.insert(
                "embedding_dim".to_string(),
                serde_json::json!(embedding.as_slice().len()),
            );
        }
    }
    let model = semantic_work_unit::Model {
        id,
        source_kind: candidate.source_kind,
        source_id: candidate.source_id,
        conversation_id: candidate.conversation_id,
        project_id: candidate.project_id,
        channel: candidate.channel,
        title: truncate_chars(&candidate.title, 180),
        summary: truncate_chars(&candidate.summary, REFLECT_PREVIEW_CHARS),
        content_preview: truncate_chars(&candidate.content_preview, REFLECT_PREVIEW_CHARS),
        text_hash,
        occurred_at: candidate.occurred_at,
        period_start: candidate.period_start,
        period_end: candidate.period_end,
        message_count: candidate.message_count.max(0),
        metadata,
        created_at: now.clone(),
        updated_at: now,
        embedding,
    };
    match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.upsert_semantic_work_unit(&model),
    )
    .await
    {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "failed to upsert semantic work unit");
            false
        }
        Err(_) => {
            tracing::warn!("semantic work unit upsert timed out");
            false
        }
    }
}

async fn upsert_candidates(
    storage: &Storage,
    embedder: Option<&EmbeddingClient>,
    candidates: impl IntoIterator<Item = ReflectCandidateUnit>,
    counts: &mut ReflectSourceCounts,
) {
    for candidate in candidates {
        let source_kind = candidate.source_kind.clone();
        if upsert_candidate(storage, embedder, candidate).await {
            increment_source_count(counts, &source_kind);
        }
    }
}

async fn list_orbit_transcripts_blocking(
    arkorbit: ArkOrbitService,
    orbit_id: String,
) -> Vec<OrbitChatTranscriptSummary> {
    match tokio::time::timeout(
        REFLECT_FS_TIMEOUT,
        tokio::task::spawn_blocking(move || arkorbit.list_orbit_chat_transcripts(&orbit_id)),
    )
    .await
    {
        Ok(Ok(Ok(transcripts))) => transcripts,
        Ok(Ok(Err(error))) => {
            tracing::debug!(error = %error, "failed to list ArkOrbit transcripts");
            Vec::new()
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "ArkOrbit transcript listing task failed");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("ArkOrbit transcript listing timed out");
            Vec::new()
        }
    }
}

async fn read_orbit_transcript_blocking(
    arkorbit: ArkOrbitService,
    orbit_id: String,
    transcript_id: String,
) -> Vec<OrbitChatMessage> {
    match tokio::time::timeout(
        REFLECT_FS_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            arkorbit.read_orbit_chat_transcript(
                &orbit_id,
                &transcript_id,
                REFLECT_MAX_MESSAGES_PER_CONVERSATION as usize,
            )
        }),
    )
    .await
    {
        Ok(Ok(Ok(messages))) => messages,
        Ok(Ok(Err(error))) => {
            tracing::debug!(error = %error, "failed to read ArkOrbit transcript");
            Vec::new()
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "ArkOrbit transcript read task failed");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("ArkOrbit transcript read timed out");
            Vec::new()
        }
    }
}

async fn read_lineage_file(path_rel: &str, limit: usize) -> Vec<serde_json::Value> {
    tokio::time::timeout(REFLECT_FS_TIMEOUT, read_recent_jsonl(path_rel, limit))
        .await
        .unwrap_or_default()
}

async fn refresh_reflect_units(
    storage: &Storage,
    embedder: Option<&EmbeddingClient>,
    arkorbit: &ArkOrbitService,
    user_id: &str,
    app_registry: &crate::actions::app::AppRegistry,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> ReflectSourceCounts {
    let mut counts = ReflectSourceCounts::default();
    let from_s = from.to_rfc3339();
    let to_s = to.to_rfc3339();
    let cutoff =
        (chrono::Utc::now() - chrono::Duration::days(REFLECT_CACHE_RETENTION_DAYS)).to_rfc3339();
    if let Ok(Err(error)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.delete_semantic_work_units_before(&cutoff),
    )
    .await
    {
        tracing::debug!(error = %error, "semantic work unit retention cleanup failed");
    }

    if let Ok(Ok(conversations)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_conversations_updated_between(&from_s, &to_s, REFLECT_MAX_CONVERSATIONS),
    )
    .await
    {
        for conversation in conversations {
            let messages = tokio::time::timeout(
                REFLECT_DB_TIMEOUT,
                storage.get_messages_between(
                    &conversation.id,
                    &from_s,
                    &to_s,
                    REFLECT_MAX_MESSAGES_PER_CONVERSATION,
                ),
            )
            .await
            .ok()
            .and_then(|result| result.ok())
            .unwrap_or_default();
            upsert_candidates(
                storage,
                embedder,
                conversation_candidates(&conversation, messages),
                &mut counts,
            )
            .await;
        }
    }

    if let Ok(Ok(orbits)) =
        tokio::time::timeout(REFLECT_DB_TIMEOUT, arkorbit.list_orbits(user_id)).await
    {
        for orbit in orbits.into_iter().take(REFLECT_MAX_ORBITS) {
            let transcripts =
                list_orbit_transcripts_blocking(arkorbit.clone(), orbit.id.clone()).await;
            for transcript in transcripts
                .into_iter()
                .take(REFLECT_MAX_TRANSCRIPTS_PER_ORBIT)
            {
                if !in_window(&transcript.updated_at, from, to) {
                    continue;
                }
                let messages = read_orbit_transcript_blocking(
                    arkorbit.clone(),
                    orbit.id.clone(),
                    transcript.id.clone(),
                )
                .await;
                let messages = filter_orbit_messages_between(messages, from, to);
                upsert_candidates(
                    storage,
                    embedder,
                    orbit_candidates(&orbit, &transcript, messages),
                    &mut counts,
                )
                .await;
            }
        }
    }

    if let Ok(Ok(items)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_experience_items_between(&from_s, &to_s, REFLECT_MAX_EXPERIENCE_ITEMS),
    )
    .await
    {
        upsert_candidates(
            storage,
            embedder,
            items.into_iter().map(experience_item_candidate),
            &mut counts,
        )
        .await;
    }

    if let Ok(Ok(patterns)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_procedural_patterns_between(&from_s, &to_s, REFLECT_MAX_PROCEDURAL_PATTERNS),
    )
    .await
    {
        upsert_candidates(
            storage,
            embedder,
            patterns.into_iter().map(procedural_pattern_candidate),
            &mut counts,
        )
        .await;
    }

    let apps = tokio::time::timeout(REFLECT_DB_TIMEOUT, app_registry.list())
        .await
        .unwrap_or_default();
    let app_candidates = apps
        .into_iter()
        .filter_map(|app| app_candidate(&app, from, to))
        .collect::<Vec<_>>();
    upsert_candidates(storage, embedder, app_candidates, &mut counts).await;

    if let Ok(Ok(tasks)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_tasks_updated_between(&from_s, &to_s, REFLECT_MAX_TASKS),
    )
    .await
    {
        upsert_candidates(
            storage,
            embedder,
            tasks
                .into_iter()
                .filter_map(|row| goal_candidate(row, from, to)),
            &mut counts,
        )
        .await;
    }

    let watchers = tokio::time::timeout(REFLECT_DB_TIMEOUT, storage.list_watchers())
        .await
        .ok()
        .and_then(|result| result.ok())
        .unwrap_or_default();
    let live_watcher_ids = watchers
        .iter()
        .map(|watcher| watcher.id.to_string())
        .collect::<HashSet<_>>();
    upsert_candidates(
        storage,
        embedder,
        watchers
            .into_iter()
            .take(REFLECT_MAX_WATCHERS)
            .filter_map(|watcher| watcher_candidate(watcher, from, to)),
        &mut counts,
    )
    .await;

    let supervisor_states = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_automation_supervisor_states(),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .unwrap_or_default();
    upsert_candidates(
        storage,
        embedder,
        supervisor_states
            .into_iter()
            .take(REFLECT_MAX_WATCHERS)
            .filter_map(|state| supervisor_watcher_candidate(state, &live_watcher_ids, from, to)),
        &mut counts,
    )
    .await;

    let observations = sentinel_panel::load_observations(storage).await;
    upsert_candidates(
        storage,
        embedder,
        observations
            .into_iter()
            .take(REFLECT_MAX_SENTINEL_ITEMS)
            .filter_map(|item| sentinel_observation_candidate(item, from, to)),
        &mut counts,
    )
    .await;
    let proposals = sentinel_panel::load_proposals(storage).await;
    upsert_candidates(
        storage,
        embedder,
        proposals
            .into_iter()
            .take(REFLECT_MAX_SENTINEL_ITEMS)
            .filter_map(|item| sentinel_proposal_candidate(item, from, to)),
        &mut counts,
    )
    .await;

    if let Ok(Ok(events)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_arkpulse_events_between(&from_s, &to_s, REFLECT_MAX_PULSE_EVENTS),
    )
    .await
    {
        upsert_candidates(
            storage,
            embedder,
            events.into_iter().map(arkpulse_candidate),
            &mut counts,
        )
        .await;
    }

    let routing_lineage =
        read_lineage_file(ROUTING_POLICY_LINEAGE_REL_PATH, REFLECT_MAX_LINEAGE_ROWS).await;
    let prompt_lineage =
        read_lineage_file(PROMPT_BUNDLE_LINEAGE_REL_PATH, REFLECT_MAX_LINEAGE_ROWS).await;
    let specialist_lineage = read_lineage_file(
        SPECIALIST_PROMPT_BUNDLE_LINEAGE_REL_PATH,
        REFLECT_MAX_LINEAGE_ROWS,
    )
    .await;
    let prompt_fragment_lineage = read_lineage_file(
        PROMPT_FRAGMENT_BUNDLE_LINEAGE_REL_PATH,
        REFLECT_MAX_LINEAGE_ROWS,
    )
    .await;
    upsert_candidates(
        storage,
        embedder,
        routing_lineage
            .into_iter()
            .filter_map(|value| lineage_candidate("routing_policy", value, from, to))
            .chain(
                prompt_lineage
                    .into_iter()
                    .filter_map(|value| lineage_candidate("prompt_bundle", value, from, to)),
            )
            .chain(
                specialist_lineage.into_iter().filter_map(|value| {
                    lineage_candidate("specialist_prompt_bundle", value, from, to)
                }),
            )
            .chain(
                prompt_fragment_lineage.into_iter().filter_map(|value| {
                    lineage_candidate("prompt_fragment_bundle", value, from, to)
                }),
            ),
        &mut counts,
    )
    .await;

    if let Ok(Ok(rows)) = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_llm_usage_between(&from_s, &to_s, REFLECT_MAX_LLM_USAGE_ROWS),
    )
    .await
    {
        upsert_candidates(storage, embedder, usage_candidates(rows), &mut counts).await;
    }

    counts
}

fn normalized_vector(embedding: &PgVector, dimension: usize) -> Option<Vec<f32>> {
    let slice = embedding.as_slice();
    if slice.len() != dimension {
        return None;
    }
    let mut values = slice.to_vec();
    let norm = values
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return None;
    }
    for value in &mut values {
        *value = (*value as f64 / norm) as f32;
    }
    Some(values)
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot = a
        .iter()
        .zip(b.iter())
        .map(|(left, right)| (*left as f64) * (*right as f64))
        .sum::<f64>();
    (1.0 - dot.clamp(-1.0, 1.0)) as f32
}

fn choose_seed_vectors(vectors: &[Vec<f32>], k: usize) -> Vec<Vec<f32>> {
    let mut seeds = Vec::new();
    if vectors.is_empty() || k == 0 {
        return seeds;
    }
    seeds.push(vectors[0].clone());
    while seeds.len() < k && seeds.len() < vectors.len() {
        let next = vectors
            .iter()
            .filter(|candidate| {
                !seeds
                    .iter()
                    .any(|seed| cosine_distance(seed, candidate) <= 0.000_01)
            })
            .max_by(|a, b| {
                let da = seeds
                    .iter()
                    .map(|seed| cosine_distance(seed, a))
                    .fold(f32::INFINITY, f32::min);
                let db = seeds
                    .iter()
                    .map(|seed| cosine_distance(seed, b))
                    .fold(f32::INFINITY, f32::min);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();
        if let Some(next) = next {
            seeds.push(next);
        } else {
            break;
        }
    }
    seeds
}

fn recompute_centroid(
    assignments: &[usize],
    vectors: &[Vec<f32>],
    cluster: usize,
    dimension: usize,
) -> Option<Vec<f32>> {
    let mut centroid = vec![0.0f64; dimension];
    let mut count = 0usize;
    for (idx, vector) in vectors.iter().enumerate() {
        if assignments.get(idx).copied() != Some(cluster) {
            continue;
        }
        count += 1;
        for (slot, value) in centroid.iter_mut().zip(vector.iter()) {
            *slot += *value as f64;
        }
    }
    if count == 0 {
        return None;
    }
    let mut centroid = centroid
        .into_iter()
        .map(|value| (value / count as f64) as f32)
        .collect::<Vec<_>>();
    let norm = centroid
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return None;
    }
    for value in &mut centroid {
        *value = (*value as f64 / norm) as f32;
    }
    Some(centroid)
}

fn centroid_for_local_indices(
    indices: &[usize],
    vectors: &[Vec<f32>],
    dimension: usize,
) -> Option<Vec<f32>> {
    if indices.is_empty() || dimension == 0 {
        return None;
    }
    let mut centroid = vec![0.0f64; dimension];
    let mut count = 0usize;
    for idx in indices {
        let Some(vector) = vectors.get(*idx) else {
            continue;
        };
        count += 1;
        for (slot, value) in centroid.iter_mut().zip(vector.iter()) {
            *slot += *value as f64;
        }
    }
    if count == 0 {
        return None;
    }
    let mut centroid = centroid
        .into_iter()
        .map(|value| (value / count as f64) as f32)
        .collect::<Vec<_>>();
    let norm = centroid
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return None;
    }
    for value in &mut centroid {
        *value = (*value as f64 / norm) as f32;
    }
    Some(centroid)
}

fn assign_vectors(vectors: &[Vec<f32>], centroids: &[Vec<f32>]) -> Vec<usize> {
    vectors
        .iter()
        .map(|vector| {
            centroids
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    cosine_distance(vector, a)
                        .partial_cmp(&cosine_distance(vector, b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(idx, _)| idx)
                .unwrap_or(0)
        })
        .collect()
}

fn cluster_assignments(vectors: &[Vec<f32>]) -> Vec<usize> {
    if vectors.is_empty() {
        return Vec::new();
    }
    if vectors.len() <= 2 {
        return (0..vectors.len()).collect();
    }
    let k = REFLECT_MAX_CLUSTERS
        .min(vectors.len())
        .min((vectors.len() as f64).sqrt().ceil().max(2.0) as usize);
    let dimension = vectors[0].len();
    let mut centroids = choose_seed_vectors(vectors, k);
    if centroids.is_empty() {
        return vec![0; vectors.len()];
    }
    let mut assignments = assign_vectors(vectors, &centroids);
    for _ in 0..REFLECT_KMEANS_ROUNDS {
        let next_centroids = (0..centroids.len())
            .map(|cluster| {
                recompute_centroid(&assignments, vectors, cluster, dimension)
                    .unwrap_or_else(|| centroids[cluster].clone())
            })
            .collect::<Vec<_>>();
        centroids = next_centroids;
        assignments = assign_vectors(vectors, &centroids);
    }
    assignments
}

fn representative_index(indices: &[usize], vectors: &[Vec<f32>]) -> usize {
    if indices.len() <= 1 {
        return indices[0];
    }
    indices
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = indices
                .iter()
                .copied()
                .filter(|other| other != a)
                .map(|other| cosine_distance(&vectors[*a], &vectors[other]))
                .sum::<f32>();
            let db = indices
                .iter()
                .copied()
                .filter(|other| other != b)
                .map(|other| cosine_distance(&vectors[*b], &vectors[other]))
                .sum::<f32>();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(indices[0])
}

fn reflect_palette(index: usize) -> String {
    const COLORS: &[&str] = &[
        "#2F80ED", "#00A676", "#B55CFF", "#FF7A45", "#00A8A8", "#D94F70", "#7A6FF0", "#C58A00",
    ];
    COLORS[index % COLORS.len()].to_string()
}

fn build_clusters(
    units: &[semantic_work_unit::Model],
) -> (
    Vec<ReflectClusterResponse>,
    Vec<ReflectUnitResponse>,
    ReflectEmbeddingStatus,
) {
    let dimension = units
        .iter()
        .filter_map(|unit| unit.embedding.as_ref())
        .map(|embedding| embedding.as_slice().len())
        .find(|dimension| *dimension > 0)
        .unwrap_or(0);

    let mut embedded_units = Vec::<(usize, Vec<f32>)>::new();
    let mut mismatched_dimensions = 0usize;
    if dimension > 0 {
        for (idx, unit) in units.iter().enumerate() {
            match unit
                .embedding
                .as_ref()
                .and_then(|embedding| normalized_vector(embedding, dimension))
            {
                Some(embedding) => embedded_units.push((idx, embedding)),
                None if unit.embedding.is_some() => mismatched_dimensions += 1,
                None => {}
            }
        }
    }

    let embedded_count = embedded_units.len();
    if embedded_units.len() < 2 {
        let clusters = units
            .iter()
            .take(REFLECT_MAX_CLUSTERS)
            .enumerate()
            .map(|(idx, unit)| {
                let mut source_mix = BTreeMap::new();
                source_mix.insert(source_label(&unit.source_kind, &unit.channel), 1);
                ReflectClusterResponse {
                    id: format!("activity-{}", idx + 1),
                    representative_unit_id: unit.id.clone(),
                    centroid_embedding: unit.embedding.clone(),
                    label: unit.title.clone(),
                    plain_summary: unit.summary.clone(),
                    unit_count: 1,
                    message_count: unit.message_count,
                    source_mix,
                    color: reflect_palette(idx),
                    related_history: ReflectRelatedHistory::unavailable(
                        "Semantic history comparison needs an embedded representative item.",
                    ),
                    units: vec![unit_to_response(unit)],
                }
            })
            .collect::<Vec<_>>();
        let used = clusters
            .iter()
            .flat_map(|cluster| cluster.units.iter().map(|unit| unit.id.clone()))
            .collect::<HashSet<_>>();
        let unclustered = units
            .iter()
            .filter(|unit| !used.contains(&unit.id))
            .map(unit_to_response)
            .collect::<Vec<_>>();
        return (
            clusters,
            unclustered,
            ReflectEmbeddingStatus {
                mode: "activity".to_string(),
                embedded_units: embedded_count,
                total_units: units.len(),
                detail: "Reflect is showing cached activity while background semantic grouping catches up.".to_string(),
            },
        );
    }

    let vectors = embedded_units
        .iter()
        .map(|(_, vector)| vector.clone())
        .collect::<Vec<_>>();
    let assignments = cluster_assignments(&vectors);
    let mut cluster_to_local_indices: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (local_idx, cluster) in assignments.iter().copied().enumerate() {
        cluster_to_local_indices
            .entry(cluster)
            .or_default()
            .push(local_idx);
    }

    let mut groups = cluster_to_local_indices
        .into_values()
        .map(|local_indices| {
            let global_indices = local_indices
                .iter()
                .filter_map(|local_idx| embedded_units.get(*local_idx).map(|(idx, _)| *idx))
                .collect::<Vec<_>>();
            (local_indices, global_indices)
        })
        .filter(|(_, global_indices)| !global_indices.is_empty())
        .collect::<Vec<_>>();
    groups.sort_by_key(|(_, items)| std::cmp::Reverse(items.len()));

    let mut used = HashSet::new();
    let clusters = groups
        .into_iter()
        .take(REFLECT_MAX_CLUSTERS)
        .enumerate()
        .map(|(idx, (local_indices, global_indices))| {
            let local_representative = representative_index(&local_indices, &vectors);
            let representative_global = embedded_units
                .get(local_representative)
                .map(|(global_idx, _)| *global_idx)
                .unwrap_or(global_indices[0]);
            let representative = &units[representative_global];
            let mut source_mix = BTreeMap::new();
            let mut message_count = 0i32;
            let mut cluster_units = global_indices
                .iter()
                .map(|global_idx| {
                    let unit = &units[*global_idx];
                    used.insert(unit.id.clone());
                    *source_mix
                        .entry(source_label(&unit.source_kind, &unit.channel))
                        .or_insert(0) += 1;
                    message_count = message_count.saturating_add(unit.message_count.max(0));
                    unit_to_response(unit)
                })
                .collect::<Vec<_>>();
            cluster_units.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
            let source_count = source_mix.len();
            ReflectClusterResponse {
                id: format!("cluster-{}", idx + 1),
                representative_unit_id: representative.id.clone(),
                centroid_embedding: centroid_for_local_indices(
                    &local_indices,
                    &vectors,
                    vectors.first().map(|vector| vector.len()).unwrap_or(0),
                )
                .map(PgVector::from),
                label: representative.title.clone(),
                plain_summary: format!(
                    "{} related item{} across {} source{}.",
                    cluster_units.len(),
                    if cluster_units.len() == 1 { "" } else { "s" },
                    source_count,
                    if source_count == 1 { "" } else { "s" }
                ),
                unit_count: cluster_units.len(),
                message_count,
                source_mix,
                color: reflect_palette(idx),
                related_history: ReflectRelatedHistory::unavailable(
                    "Related history has not been checked for this cluster yet.",
                ),
                units: cluster_units,
            }
        })
        .collect::<Vec<_>>();

    let unclustered = units
        .iter()
        .filter(|unit| !used.contains(&unit.id))
        .map(unit_to_response)
        .collect::<Vec<_>>();

    let detail = if mismatched_dimensions > 0 {
        format!(
            "Semantic grouping is active. {} cached embedding{} used a different dimension and will be refreshed by the background pass.",
            mismatched_dimensions,
            if mismatched_dimensions == 1 { "" } else { "s" }
        )
    } else {
        "Semantic grouping is based on local derived work-unit embeddings for this time range."
            .to_string()
    };

    (
        clusters,
        unclustered,
        ReflectEmbeddingStatus {
            mode: "semantic".to_string(),
            embedded_units: embedded_count,
            total_units: units.len(),
            detail,
        },
    )
}

fn reflect_cluster_semaphore() -> Arc<Semaphore> {
    REFLECT_CLUSTER_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

fn build_activity_cluster_fallback(
    units: &[semantic_work_unit::Model],
    detail: impl Into<String>,
) -> (
    Vec<ReflectClusterResponse>,
    Vec<ReflectUnitResponse>,
    ReflectEmbeddingStatus,
) {
    let clusters = units
        .iter()
        .take(REFLECT_MAX_CLUSTERS)
        .enumerate()
        .map(|(idx, unit)| {
            let mut source_mix = BTreeMap::new();
            source_mix.insert(source_label(&unit.source_kind, &unit.channel), 1);
            ReflectClusterResponse {
                id: format!("activity-{}", idx + 1),
                representative_unit_id: unit.id.clone(),
                centroid_embedding: unit.embedding.clone(),
                label: unit.title.clone(),
                plain_summary: unit.summary.clone(),
                unit_count: 1,
                message_count: unit.message_count,
                source_mix,
                color: reflect_palette(idx),
                related_history: ReflectRelatedHistory::unavailable(
                    "Semantic history comparison needs completed cluster grouping.",
                ),
                units: vec![unit_to_response(unit)],
            }
        })
        .collect::<Vec<_>>();
    let used = clusters
        .iter()
        .flat_map(|cluster| cluster.units.iter().map(|unit| unit.id.clone()))
        .collect::<HashSet<_>>();
    let unclustered = units
        .iter()
        .filter(|unit| !used.contains(&unit.id))
        .map(unit_to_response)
        .collect::<Vec<_>>();
    (
        clusters,
        unclustered,
        ReflectEmbeddingStatus {
            mode: "activity".to_string(),
            embedded_units: units.iter().filter(|unit| unit.embedding.is_some()).count(),
            total_units: units.len(),
            detail: detail.into(),
        },
    )
}

async fn build_clusters_bounded(
    units: Vec<semantic_work_unit::Model>,
) -> (
    Vec<ReflectClusterResponse>,
    Vec<ReflectUnitResponse>,
    ReflectEmbeddingStatus,
) {
    let fallback_units = units.clone();
    let permit = match tokio::time::timeout(
        REFLECT_CLUSTER_QUEUE_TIMEOUT,
        reflect_cluster_semaphore().acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            tracing::warn!("cluster semaphore closed; using activity fallback clusters");
            return build_activity_cluster_fallback(
                &fallback_units,
                "Semantic grouping is temporarily unavailable; showing cached activity instead.",
            );
        }
        Err(_) => {
            tracing::debug!("cluster queue busy; using activity fallback clusters");
            return build_activity_cluster_fallback(
                &fallback_units,
                "Semantic grouping is busy; showing cached activity instead.",
            );
        }
    };
    let handle = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        build_clusters(&units)
    });
    match tokio::time::timeout(REFLECT_CLUSTER_TIMEOUT, handle).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "cluster worker failed");
            build_activity_cluster_fallback(
                &fallback_units,
                "Semantic grouping failed; showing cached activity instead.",
            )
        }
        Err(_) => {
            tracing::warn!("cluster worker timed out");
            build_activity_cluster_fallback(
                &fallback_units,
                "Semantic grouping took too long; showing cached activity instead.",
            )
        }
    }
}

async fn enrich_clusters_with_related_history(
    storage: &Storage,
    units: &[semantic_work_unit::Model],
    clusters: &mut [ReflectClusterResponse],
    from: &str,
    to: &str,
) {
    let by_id = units
        .iter()
        .map(|unit| (unit.id.as_str(), unit))
        .collect::<HashMap<_, _>>();
    for cluster in clusters {
        let Some(representative) = by_id.get(cluster.representative_unit_id.as_str()) else {
            cluster.related_history = ReflectRelatedHistory::unavailable(
                "Related history could not find this cluster's representative item.",
            );
            continue;
        };
        let Some(embedding) = cluster
            .centroid_embedding
            .as_ref()
            .or(representative.embedding.as_ref())
        else {
            cluster.related_history = ReflectRelatedHistory::unavailable(
                "Related history needs an embedded representative item.",
            );
            continue;
        };
        let exclude_ids = cluster
            .units
            .iter()
            .map(|unit| unit.id.clone())
            .collect::<Vec<_>>();
        let related = tokio::time::timeout(
            REFLECT_RELATED_HISTORY_TIMEOUT,
            storage.nearest_semantic_work_units_outside_window(
                embedding,
                from,
                to,
                &exclude_ids,
                REFLECT_RELATED_HISTORY_LIMIT,
            ),
        )
        .await;
        let rows = match related {
            Ok(Ok(rows)) => rows,
            Ok(Err(error)) => {
                tracing::debug!(error = %error, "related history lookup failed");
                cluster.related_history = ReflectRelatedHistory::unavailable(
                    "Related history was unavailable for this cluster.",
                );
                continue;
            }
            Err(_) => {
                tracing::debug!("related history lookup timed out");
                cluster.related_history = ReflectRelatedHistory::unavailable(
                    "Related history took too long and was skipped for this cluster.",
                );
                continue;
            }
        };
        let mut items = rows
            .into_iter()
            .filter_map(|(unit, distance)| {
                if distance > REFLECT_RELATED_HISTORY_MAX_DISTANCE {
                    return None;
                }
                Some(ReflectRelatedUnitResponse {
                    id: unit.id.clone(),
                    source_label: source_label(&unit.source_kind, &unit.channel),
                    title: unit.title,
                    occurred_at: unit.occurred_at,
                    similarity: (1.0 - distance).clamp(0.0, 1.0),
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            cluster.related_history = ReflectRelatedHistory::new_this_period();
            continue;
        }
        items.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.occurred_at.cmp(&left.occurred_at))
        });
        let similar_count = items.len();
        let most_recent_at = items.iter().map(|item| item.occurred_at.clone()).max();
        let top_similarity = items.first().map(|item| item.similarity);
        let display_items = items
            .into_iter()
            .take(REFLECT_RELATED_HISTORY_DISPLAY_LIMIT)
            .collect::<Vec<_>>();
        let detail = if let Some(recent) = most_recent_at.as_ref() {
            format!(
                "Found {} close match{} in reflection history, most recently {}.",
                similar_count,
                if similar_count == 1 { "" } else { "es" },
                recent
            )
        } else {
            format!(
                "Found {} close match{} in reflection history.",
                similar_count,
                if similar_count == 1 { "" } else { "es" }
            )
        };
        cluster.related_history = ReflectRelatedHistory {
            mode: "recurring".to_string(),
            similar_count,
            most_recent_at,
            top_similarity,
            detail,
            items: display_items,
        };
    }
}

async fn reflect_server_is_idle(state: &AppState) -> bool {
    if REFLECT_REFRESH_IN_FLIGHT.load(Ordering::Acquire)
        || REFLECT_FOLLOWUP_SEARCH_IN_FLIGHT.load(Ordering::Acquire)
        || REFLECT_FOLLOWUP_SUMMARY_IN_FLIGHT.load(Ordering::Acquire)
    {
        return false;
    }
    if crate::sentinel::is_pulse_running() {
        return false;
    }
    if !state.chat_task_cancellations.read().await.is_empty() {
        return false;
    }
    if !state.action_test_cancellations.read().await.is_empty() {
        return false;
    }
    let tasks = state.tasks.read().await;
    if tasks
        .all()
        .iter()
        .any(|task| matches!(task.status, TaskStatus::InProgress))
    {
        return false;
    }
    drop(tasks);
    let storage = {
        let agent = state.agent.read().await;
        if agent.active_message_request_count() > 0 {
            return false;
        }
        agent.storage.clone()
    };
    storage
        .lease_status_summary()
        .await
        .map(|summary| {
            summary.active_task_leases == 0
                && summary.active_watcher_leases == 0
                && summary.active_run_leases == 0
                && summary.runs_pending_cancellation == 0
        })
        .unwrap_or(false)
}

async fn run_reflect_refresh_job(
    state: AppState,
    request: ReflectRefreshRequest,
) -> std::result::Result<ReflectSourceCounts, String> {
    let (storage, embedder, arkorbit, user_id, app_registry) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.embedding_client.clone(),
            agent.arkorbit.clone(),
            agent.identity.did().to_string(),
            state.app_registry.clone(),
        )
    };
    Ok(refresh_reflect_units(
        &storage,
        embedder.as_deref(),
        &arkorbit,
        &user_id,
        &app_registry,
        request.from,
        request.to,
    )
    .await)
}

async fn spawn_reflect_refresh(
    state: AppState,
    request: ReflectRefreshRequest,
    trigger: &'static str,
    require_idle: bool,
) -> ReflectRefreshStartResponse {
    if require_idle && !reflect_server_is_idle(&state).await {
        let refresh_status = update_refresh_status(|status| {
            status.status = "deferred_busy".to_string();
            status.trigger = Some(trigger.to_string());
            status.requested_at = Some(chrono::Utc::now().to_rfc3339());
            status.period = Some(request.period.as_str().to_string());
            status.from = Some(request.from.to_rfc3339());
            status.to = Some(request.to.to_rfc3339());
        })
        .await;
        return ReflectRefreshStartResponse {
            accepted: false,
            running: false,
            status: "deferred_busy".to_string(),
            detail: "Reflect refresh was deferred because foreground work is active.".to_string(),
            refresh_status,
        };
    }

    let lease_storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };

    let now = chrono::Utc::now();
    let current_status = current_refresh_status().await;
    if reflect_refresh_recently_completed_for_request(&current_status, &request, now) {
        return ReflectRefreshStartResponse {
            accepted: false,
            running: false,
            status: "already_current".to_string(),
            detail:
                "Reflect already refreshed this range recently; it will run again after new activity or the cache becomes stale."
                    .to_string(),
            refresh_status: current_status,
        };
    }

    if REFLECT_REFRESH_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        let refresh_status = update_refresh_status(|status| {
            status.running = true;
            if status.status != "running" {
                status.status = "already_running".to_string();
            }
            status.trigger = status.trigger.clone().or_else(|| Some(trigger.to_string()));
            status.period = status
                .period
                .clone()
                .or_else(|| Some(request.period.as_str().to_string()));
            status.from = status
                .from
                .clone()
                .or_else(|| Some(request.from.to_rfc3339()));
            status.to = status.to.clone().or_else(|| Some(request.to.to_rfc3339()));
            status.requested_at = status
                .requested_at
                .clone()
                .or_else(|| Some(chrono::Utc::now().to_rfc3339()));
        })
        .await;
        return ReflectRefreshStartResponse {
            accepted: false,
            running: true,
            status: "already_running".to_string(),
            detail: "An Reflect refresh is already running.".to_string(),
            refresh_status,
        };
    }

    let in_flight_guard = ReflectInFlightGuard;
    let lease_owner = format!("arkreflect:{}:{}", std::process::id(), uuid::Uuid::new_v4());
    let lease_guard = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        lease_storage.acquire_kv_lease_guard(
            REFLECT_REFRESH_LEASE_KEY,
            &lease_owner,
            REFLECT_REFRESH_LEASE_TTL_SECS,
        ),
    )
    .await
    {
        Ok(Ok(Some(guard))) => guard,
        Ok(Ok(None)) => {
            let refresh_status = update_refresh_status(|status| {
                status.running = true;
                status.status = "already_running".to_string();
                status.trigger = Some(trigger.to_string());
                status.period = Some(request.period.as_str().to_string());
                status.from = Some(request.from.to_rfc3339());
                status.to = Some(request.to.to_rfc3339());
                status.requested_at = Some(chrono::Utc::now().to_rfc3339());
                status.last_error = None;
            })
            .await;
            return ReflectRefreshStartResponse {
                accepted: false,
                running: true,
                status: "already_running".to_string(),
                detail: "An Reflect refresh is already running in another worker.".to_string(),
                refresh_status,
            };
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "failed to acquire refresh lease");
            let refresh_status = update_refresh_status(|status| {
                status.running = false;
                status.status = "lease_failed".to_string();
                status.trigger = Some(trigger.to_string());
                status.requested_at = Some(chrono::Utc::now().to_rfc3339());
                status.period = Some(request.period.as_str().to_string());
                status.from = Some(request.from.to_rfc3339());
                status.to = Some(request.to.to_rfc3339());
                status.last_error = Some(format!("Failed to acquire refresh lease: {}", error));
            })
            .await;
            return ReflectRefreshStartResponse {
                accepted: false,
                running: false,
                status: "lease_failed".to_string(),
                detail: "Reflect could not acquire its background refresh lease.".to_string(),
                refresh_status,
            };
        }
        Err(_) => {
            let refresh_status = update_refresh_status(|status| {
                status.running = false;
                status.status = "lease_timed_out".to_string();
                status.trigger = Some(trigger.to_string());
                status.requested_at = Some(chrono::Utc::now().to_rfc3339());
                status.period = Some(request.period.as_str().to_string());
                status.from = Some(request.from.to_rfc3339());
                status.to = Some(request.to.to_rfc3339());
                status.last_error = Some("Timed out acquiring refresh lease".to_string());
            })
            .await;
            return ReflectRefreshStartResponse {
                accepted: false,
                running: false,
                status: "lease_timed_out".to_string(),
                detail: "Reflect timed out before it could acquire its refresh lease.".to_string(),
                refresh_status,
            };
        }
    };

    let sequence = REFLECT_REFRESH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let requested_at = chrono::Utc::now().to_rfc3339();
    let refresh_status = update_refresh_status(|status| {
        status.running = true;
        status.status = "running".to_string();
        status.trigger = Some(trigger.to_string());
        status.period = Some(request.period.as_str().to_string());
        status.from = Some(request.from.to_rfc3339());
        status.to = Some(request.to.to_rfc3339());
        status.requested_at = Some(requested_at.clone());
        status.started_at = Some(requested_at.clone());
        status.completed_at = None;
        status.last_error = None;
        status.sequence = sequence;
    })
    .await;

    crate::spawn_logged!("src/channels/http/reflect_control.rs:refresh", async move {
        let guard = in_flight_guard;
        let result = tokio::time::timeout(
            REFLECT_REFRESH_TIMEOUT,
            run_reflect_refresh_job(state.clone(), request.clone()),
        )
        .await;
        let completed = match result {
            Ok(Ok(counts)) => {
                update_refresh_status(|status| {
                    status.running = false;
                    status.status = "completed".to_string();
                    status.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    status.last_error = None;
                    status.last_source_counts = counts;
                })
                .await;
                true
            }
            Ok(Err(error)) => {
                update_refresh_status(|status| {
                    status.running = false;
                    status.status = "failed".to_string();
                    status.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    status.last_error = Some(error);
                })
                .await;
                false
            }
            Err(_) => {
                update_refresh_status(|status| {
                    status.running = false;
                    status.status = "timed_out".to_string();
                    status.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    status.last_error = Some(format!(
                        "Refresh exceeded {} seconds",
                        REFLECT_REFRESH_TIMEOUT.as_secs()
                    ));
                })
                .await;
                false
            }
        };
        if let Err(error) = lease_storage
            .release_kv_lease_guard(REFLECT_REFRESH_LEASE_KEY, &lease_guard)
            .await
        {
            tracing::warn!(error = %error, "failed to release refresh lease");
        }
        drop(guard);
        if completed {
            let _ = maybe_spawn_reflect_followup_search_for_range(
                state.clone(),
                request.from,
                request.to,
                trigger,
            )
            .await;
        }
    });

    ReflectRefreshStartResponse {
        accepted: true,
        running: true,
        status: "queued".to_string(),
        detail: "Reflect refresh started in the background.".to_string(),
        refresh_status,
    }
}

fn reflect_request_from_params(
    params: &HashMap<String, String>,
) -> std::result::Result<ReflectRefreshRequest, Response> {
    let period = ReflectPeriod::from_query(params.get("period").map(String::as_str));
    let now = chrono::Utc::now();
    let (default_from, default_to) = period.default_window(now);
    let from = query_time(params, "from").unwrap_or(default_from);
    let to = query_time(params, "to").unwrap_or(default_to);
    if from >= to {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "from must be before to".to_string(),
            }),
        )
            .into_response());
    }
    Ok(ReflectRefreshRequest { period, from, to })
}

fn cache_status_for_units(
    units: &[semantic_work_unit::Model],
    refresh_status: &ReflectRefreshStatus,
) -> ReflectCacheStatus {
    let latest_unit_at = units
        .iter()
        .filter_map(|unit| parse_time(&unit.updated_at))
        .max();
    let stale = latest_unit_at
        .map(|dt| (chrono::Utc::now() - dt).num_seconds() > REFLECT_STALE_AFTER_SECS)
        .unwrap_or(true);
    let mode = if units.is_empty() {
        "empty".to_string()
    } else if refresh_status.running {
        "refreshing".to_string()
    } else if stale {
        "stale".to_string()
    } else {
        "ready".to_string()
    };
    let detail = match mode.as_str() {
        "empty" => "No cached reflection rows exist for this range yet. A scheduled refresh or manual run can prepare them.".to_string(),
        "refreshing" => "Showing cached reflection rows while a background refresh updates this range.".to_string(),
        "stale" => "Showing cached reflection rows. A background refresh can update recent changes.".to_string(),
        _ => "Showing cached reflection rows for this range.".to_string(),
    };
    ReflectCacheStatus {
        mode,
        cached_units: units.len(),
        stale,
        detail,
    }
}

fn reflect_bool_pref(raw: Option<Vec<u8>>) -> bool {
    raw.and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

async fn arkreflect_daily_digest_enabled(storage: &Storage) -> bool {
    let raw = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.get(super::autonomy_support::ARKREFLECT_DAILY_DIGEST_ENABLED_KEY),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .flatten();
    reflect_bool_pref(raw)
}

async fn load_daily_digest_status(
    storage: &Storage,
    enabled: bool,
    today_date: chrono::NaiveDate,
) -> ReflectDailyDigestStatus {
    let today_key = today_date.format("%Y-%m-%d").to_string();
    if !enabled {
        return ReflectDailyDigestStatus::disabled(today_key);
    }
    tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.get(REFLECT_DAILY_DIGEST_STATUS_KEY),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .flatten()
    .and_then(|bytes| serde_json::from_slice::<ReflectDailyDigestStatus>(&bytes).ok())
    .map(|mut status| {
        status.enabled = true;
        status.today_date = today_key.clone();
        status
    })
    .unwrap_or_else(|| ReflectDailyDigestStatus {
        enabled: true,
        status: "waiting".to_string(),
        target_date: today_key.clone(),
        today_date: today_key,
        meaningful: false,
        unit_count: 0,
        cluster_count: 0,
        source_counts: ReflectSourceCounts::default(),
        summary: None,
        detail: "Waiting for the next quiet window to prepare today's Reflect digest.".to_string(),
        last_checked_at: None,
        last_sent_at: None,
        last_skipped_at: None,
        last_error: None,
        delivery_attempts: Vec::new(),
    })
}

async fn save_daily_digest_status(storage: &Storage, status: &ReflectDailyDigestStatus) {
    match serde_json::to_vec(status) {
        Ok(bytes) => {
            if let Err(error) = tokio::time::timeout(
                REFLECT_DB_TIMEOUT,
                storage.set(REFLECT_DAILY_DIGEST_STATUS_KEY, &bytes),
            )
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("daily digest status save timed out")))
            {
                tracing::debug!(error = %error, "failed to save daily digest status");
            }
        }
        Err(error) => {
            tracing::debug!(error = %error, "failed to serialize daily digest status");
        }
    }
}

async fn baseline_source_counts(
    storage: &Storage,
    from: chrono::DateTime<chrono::Utc>,
) -> ReflectSourceCounts {
    let baseline_from =
        (from - chrono::Duration::days(REFLECT_BASELINE_LOOKBACK_DAYS)).to_rfc3339();
    let baseline_to = from.to_rfc3339();
    tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_semantic_work_units_between(
            &baseline_from,
            &baseline_to,
            REFLECT_BASELINE_MAX_UNITS,
        ),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .map(|units| source_counts_from_units(&units))
    .unwrap_or_default()
}

fn reflect_digest_source_lines(counts: &ReflectSourceCounts) -> Vec<String> {
    [
        ("chat", counts.main_chat),
        ("ArkOrbit", counts.orbit_chat),
        ("memory", counts.memory),
        ("learned workflows", counts.procedures),
        ("apps", counts.apps),
        ("goals", counts.goals),
        ("watchers", counts.watchers),
        ("Sentinel", counts.sentinel),
        ("Pulse", counts.arkpulse),
        ("Evolve", counts.arkevolve),
        ("usage", counts.usage),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(label, count)| format!("{} {}", count, label))
    .collect()
}

fn fallback_daily_digest_summary(
    date_key: &str,
    counts: &ReflectSourceCounts,
    clusters: &[ReflectClusterResponse],
    units: &[semantic_work_unit::Model],
) -> String {
    let mut lines = Vec::new();
    let focus = clusters
        .iter()
        .take(3)
        .map(|cluster| cluster.label.trim())
        .filter(|label| !label.is_empty())
        .collect::<Vec<_>>();
    if focus.is_empty() {
        lines.push(format!(
            "Reflect found meaningful activity on {}.",
            date_key
        ));
    } else {
        lines.push(format!("Today centered on {}.", focus.join(", ")));
    }

    let source_lines = reflect_digest_source_lines(counts);
    if !source_lines.is_empty() {
        lines.push(format!("Sources represented: {}.", source_lines.join(", ")));
    }

    let background = background_source_count(counts);
    if background > 0 {
        lines.push(format!(
            "AgentArk recorded {} background or durable work signal{} across memory, apps, goals, watchers, Sentinel, Pulse, or Evolve.",
            background,
            if background == 1 { "" } else { "s" }
        ));
    }

    let examples = units
        .iter()
        .take(2)
        .map(|unit| unit.title.trim())
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>();
    if !examples.is_empty() {
        lines.push(format!("Examples: {}.", examples.join("; ")));
    }

    lines
        .into_iter()
        .map(|line| format!("- {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn generate_daily_digest_summary(
    state: &AppState,
    date_key: &str,
    counts: &ReflectSourceCounts,
    clusters: &[ReflectClusterResponse],
    units: &[semantic_work_unit::Model],
) -> String {
    let fallback = fallback_daily_digest_summary(date_key, counts, clusters, units);
    let cluster_context = clusters
        .iter()
        .take(6)
        .map(|cluster| {
            serde_json::json!({
                "label": cluster.label,
                "summary": cluster.plain_summary,
                "unit_count": cluster.unit_count,
                "message_count": cluster.message_count,
                "source_mix": cluster.source_mix,
            })
        })
        .collect::<Vec<_>>();
    let example_context = units
        .iter()
        .take(8)
        .map(|unit| {
            serde_json::json!({
                "source": source_label(&unit.source_kind, &unit.channel),
                "title": truncate_chars(&unit.title, 120),
                "summary": truncate_chars(&unit.summary, 220),
                "occurred_at": unit.occurred_at,
            })
        })
        .collect::<Vec<_>>();
    let context = serde_json::json!({
        "date": date_key,
        "source_counts": counts,
        "total_units": units.len(),
        "meaningful_units": meaningful_source_count(counts),
        "background_units": background_source_count(counts),
        "clusters": cluster_context,
        "examples": example_context,
    });
    let system_prompt = "You write Reflect daily digests for a personal AI Agent OS. Use only the structured facts provided. Do not invent work, outcomes, sources, dates, costs, or failures. Write for a novice user in plain language. Keep it concise: 3-5 bullets, no heading, no empty-day language, no generic encouragement.";
    let user_message = format!(
        "Structured Reflect daily context:\n{}\n\nWrite the user-readable digest.",
        serde_json::to_string_pretty(&context).unwrap_or_else(|_| "{}".to_string())
    );
    let llm = {
        let agent = state.agent.read().await;
        agent.llm.clone()
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        llm.chat_with_system(system_prompt, &user_message),
    )
    .await
    {
        Ok(Ok(response)) => {
            let content = response.content.trim();
            if content.chars().count() >= 24 {
                truncate_chars(content, 1800)
            } else {
                fallback
            }
        }
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "daily digest LLM summary failed");
            fallback
        }
        Err(_) => {
            tracing::debug!("daily digest LLM summary timed out");
            fallback
        }
    }
}

async fn maybe_prepare_daily_digest(state: AppState) {
    let (storage, profile_arc) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.user_profile.clone())
    };
    let enabled = arkreflect_daily_digest_enabled(&storage).await;
    let profile = profile_arc.read().await.clone();
    let tz = reflect_timezone_from_profile(&profile);
    let now = chrono::Utc::now();
    let today = reflect_local_date(now, tz);
    let today_key = today.format("%Y-%m-%d").to_string();
    if !enabled {
        let status = ReflectDailyDigestStatus::disabled(today_key);
        save_daily_digest_status(&storage, &status).await;
        return;
    }

    let target_date = reflect_digest_target_date(now, tz);
    let target_key = target_date.format("%Y-%m-%d").to_string();
    let previous = load_daily_digest_status(&storage, true, today).await;
    if previous.target_date == target_key && previous.status == "sent" {
        return;
    }

    let lease_owner = format!(
        "arkreflect-digest:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let lease_guard = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.acquire_kv_lease_guard(
            REFLECT_DAILY_DIGEST_LEASE_KEY,
            &lease_owner,
            REFLECT_DAILY_DIGEST_LEASE_TTL_SECS,
        ),
    )
    .await
    {
        Ok(Ok(Some(guard))) => guard,
        Ok(Ok(None)) => return,
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "failed to acquire daily digest lease");
            return;
        }
        Err(_) => return,
    };

    let (from, to) = reflect_daily_window_for_date(target_date, tz);
    let from_s = from.to_rfc3339();
    let to_s = to.to_rfc3339();
    let units = tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_semantic_work_units_between(&from_s, &to_s, REFLECT_MAX_UNITS),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .unwrap_or_default();
    let refresh_status = current_refresh_status().await;
    let cache_status = cache_status_for_units(&units, &refresh_status);
    let was_preparing = previous.target_date == target_key && previous.status == "preparing";
    if (units.is_empty() || cache_status.stale) && !was_preparing {
        let _ = spawn_reflect_refresh(
            state.clone(),
            ReflectRefreshRequest {
                period: ReflectPeriod::Daily,
                from,
                to,
            },
            "daily_digest",
            true,
        )
        .await;
        let status = ReflectDailyDigestStatus {
            enabled: true,
            status: "preparing".to_string(),
            target_date: target_key,
            today_date: today_key,
            meaningful: false,
            unit_count: units.len(),
            cluster_count: 0,
            source_counts: source_counts_from_units(&units),
            summary: None,
            detail: "Preparing the daily Reflect recap in the background.".to_string(),
            last_checked_at: Some(now.to_rfc3339()),
            last_sent_at: previous.last_sent_at,
            last_skipped_at: previous.last_skipped_at,
            last_error: None,
            delivery_attempts: Vec::new(),
        };
        save_daily_digest_status(&storage, &status).await;
        let _ = storage
            .release_kv_lease_guard(REFLECT_DAILY_DIGEST_LEASE_KEY, &lease_guard)
            .await;
        return;
    }

    let counts = source_counts_from_units(&units);
    let (clusters, _, _) = build_clusters_bounded(units.clone()).await;
    let meaningful = reflect_activity_is_meaningful(&counts, &units, &clusters);
    if !meaningful {
        let status = ReflectDailyDigestStatus {
            enabled: true,
            status: "skipped_quiet".to_string(),
            target_date: target_key,
            today_date: today_key,
            meaningful: false,
            unit_count: units.len(),
            cluster_count: clusters.len(),
            source_counts: counts,
            summary: None,
            detail: "No meaningful Reflect activity was found for this day, so no digest was sent."
                .to_string(),
            last_checked_at: Some(now.to_rfc3339()),
            last_sent_at: previous.last_sent_at,
            last_skipped_at: Some(now.to_rfc3339()),
            last_error: None,
            delivery_attempts: Vec::new(),
        };
        save_daily_digest_status(&storage, &status).await;
        let _ = storage
            .release_kv_lease_guard(REFLECT_DAILY_DIGEST_LEASE_KEY, &lease_guard)
            .await;
        return;
    }

    let summary =
        generate_daily_digest_summary(&state, &target_key, &counts, &clusters, &units).await;
    let (in_app, push_attempts) = {
        let agent = state.agent.read().await;
        let in_app = agent
            .emit_notification_with_status("Reflect Daily Digest", &summary, "info", "arkreflect")
            .await;
        let push_attempts = agent.notify_preferred_channel_reported(&summary).await;
        (in_app, push_attempts)
    };
    let mut delivery_attempts = vec![ReflectDigestDeliveryAttempt {
        channel: in_app.channel,
        success: in_app.success,
        error: in_app.error,
    }];
    delivery_attempts.extend(push_attempts.into_iter().map(|attempt| {
        ReflectDigestDeliveryAttempt {
            channel: attempt.channel,
            success: attempt.success,
            error: attempt.error,
        }
    }));
    let sent_ok = delivery_attempts.iter().any(|attempt| attempt.success);
    let status = ReflectDailyDigestStatus {
        enabled: true,
        status: if sent_ok { "sent" } else { "delivery_failed" }.to_string(),
        target_date: target_key,
        today_date: today_key,
        meaningful: true,
        unit_count: units.len(),
        cluster_count: clusters.len(),
        source_counts: counts,
        summary: Some(summary),
        detail: if sent_ok {
            "Daily Reflect digest was prepared and delivered.".to_string()
        } else {
            "Daily Reflect digest was prepared, but delivery failed.".to_string()
        },
        last_checked_at: Some(now.to_rfc3339()),
        last_sent_at: if sent_ok {
            Some(now.to_rfc3339())
        } else {
            previous.last_sent_at
        },
        last_skipped_at: previous.last_skipped_at,
        last_error: if sent_ok {
            None
        } else {
            Some("No notification channel accepted the daily digest.".to_string())
        },
        delivery_attempts,
    };
    save_daily_digest_status(&storage, &status).await;
    let _ = storage
        .release_kv_lease_guard(REFLECT_DAILY_DIGEST_LEASE_KEY, &lease_guard)
        .await;
}

async fn reflect_idle_loop(state: AppState, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(REFLECT_IDLE_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => break,
            _ = interval.tick() => {}
        }
        if !reflect_server_is_idle(&state).await {
            continue;
        }
        let digest_result = tokio::select! {
            _ = shutdown_rx.changed() => break,
            result = tokio::time::timeout(
                REFLECT_DAILY_DIGEST_TIMEOUT,
                maybe_prepare_daily_digest(state.clone()),
            ) => result,
        };
        if digest_result.is_err() {
            tracing::debug!("daily digest background pass timed out");
        }
        if *shutdown_rx.borrow() {
            break;
        }
        if maybe_spawn_reflect_followup_search_from_recent_activity(state.clone(), "idle").await {
            continue;
        }
        let now = chrono::Utc::now();
        let request = ReflectRefreshRequest {
            period: ReflectPeriod::Monthly,
            from: now - chrono::Duration::days(REFLECT_IDLE_LOOKBACK_DAYS),
            to: now,
        };
        let _ = spawn_reflect_refresh(state.clone(), request, "idle", true).await;
    }
}

pub(super) fn spawn_reflect_idle_loop(
    state: AppState,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if REFLECT_IDLE_LOOP_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    Some(crate::spawn_logged!(
        "src/channels/http/reflect_control.rs:idle_loop",
        async move {
            reflect_idle_loop(state, shutdown_rx).await;
        }
    ))
}

pub(super) async fn ark_reflect_endpoint(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let request = match reflect_request_from_params(&params) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let from_s = request.from.to_rfc3339();
    let to_s = request.to.to_rfc3339();

    let (storage, profile_arc, embedding_client) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.user_profile.clone(),
            agent.embedding_client.clone(),
        )
    };
    let profile = profile_arc.read().await.clone();
    let tz = reflect_timezone_from_profile(&profile);
    let today_date = reflect_local_date(chrono::Utc::now(), tz);
    let digest_enabled = arkreflect_daily_digest_enabled(&storage).await;
    let daily_digest_status = load_daily_digest_status(&storage, digest_enabled, today_date).await;
    let units = match tokio::time::timeout(
        REFLECT_DB_TIMEOUT,
        storage.list_semantic_work_units_between(&from_s, &to_s, REFLECT_MAX_UNITS),
    )
    .await
    {
        Ok(Ok(units)) => units,
        Ok(Err(error)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load reflection data: {}", error),
                }),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                Json(ErrorResponse {
                    error: "Timed out loading reflection data".to_string(),
                }),
            )
                .into_response();
        }
    };

    let refresh_status = current_refresh_status().await;
    let source_counts = source_counts_from_units(&units);
    let baseline_source_counts = baseline_source_counts(&storage, request.from).await;
    let cache_status = cache_status_for_units(&units, &refresh_status);
    let (mut clusters, unclustered_units, embedding_status) =
        build_clusters_bounded(units.clone()).await;
    if tokio::time::timeout(
        REFLECT_RELATED_HISTORY_TOTAL_TIMEOUT,
        enrich_clusters_with_related_history(&storage, &units, &mut clusters, &from_s, &to_s),
    )
    .await
    .is_err()
    {
        tracing::debug!("related history enrichment timed out");
    }
    let followup_build = build_suggested_followups(
        &storage,
        &clusters,
        embedding_client.as_deref(),
        None,
        false,
    )
    .await;
    let suggested_followups = followup_build.followups;
    let followup_planning = reflect_followup_planning_status(followup_build.unplanned.len()).await;
    // Plan the very candidates this response computed (no re-cluster, no id
    // drift) and run due source checks for the visible ones.
    let planning_state = state.clone();
    let pending_candidates = followup_build.unplanned;
    let visible_followups = suggested_followups.clone();
    crate::spawn_logged!(
        "src/channels/http/reflect_control.rs:followup_view_planning",
        async move {
            let _ = spawn_reflect_followup_planning_for_candidates(
                planning_state,
                pending_candidates,
                visible_followups,
                "view",
            )
            .await;
        }
    );
    (
        StatusCode::OK,
        Json(ReflectResponse {
            period: request.period,
            from: from_s,
            to: to_s,
            generated_at: chrono::Utc::now().to_rfc3339(),
            source_counts,
            baseline_source_counts,
            embedding_status,
            refresh_status,
            cache_status,
            daily_digest_status,
            suggested_followups,
            followup_planning,
            clusters,
            unclustered_units,
        }),
    )
        .into_response()
}

pub(super) async fn ark_reflect_refresh_endpoint(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let request = match reflect_request_from_params(&params) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let result = spawn_reflect_refresh(state, request, "manual", false).await;
    let status = if result.accepted || result.running {
        StatusCode::ACCEPTED
    } else if result.status == "already_current" {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    (status, Json(result)).into_response()
}

pub(super) async fn ark_reflect_followup_feedback_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ReflectFollowupFeedbackRequest>,
) -> Response {
    let action = payload.action;
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let mut store = load_reflect_followup_feedback(&storage).await;
    let now = chrono::Utc::now();
    let mut keys = payload
        .keys
        .into_iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    let direct_key = format!("followup:{}", id);
    if !keys.contains(&direct_key) {
        keys.push(direct_key);
    }
    for key in &keys {
        let entry = store.entries.entry(key.clone()).or_default();
        match action {
            ReflectFollowupFeedbackAction::Useful => {
                entry.useful_count = entry.useful_count.saturating_add(1);
                entry.snoozed_until = None;
            }
            ReflectFollowupFeedbackAction::Dismiss => {
                entry.dismiss_count = entry.dismiss_count.saturating_add(1);
                entry.snoozed_until = None;
            }
            ReflectFollowupFeedbackAction::Snooze => {
                entry.snooze_count = entry.snooze_count.saturating_add(1);
                let days = if entry.snooze_count >= 2 { 30 } else { 7 };
                entry.snoozed_until = Some((now + chrono::Duration::days(days)).to_rfc3339());
            }
        }
        entry.renewed_after_feedback = false;
        entry.last_action = Some(action.as_str().to_string());
        entry.last_at = Some(now.to_rfc3339());
    }
    let response_entry =
        reflect_feedback_for_response(reflect_followup_effective_feedback(&keys, &store))
            .unwrap_or_default();
    store.updated_at = Some(now.to_rfc3339());
    save_reflect_followup_feedback(&storage, &store).await;
    (
        StatusCode::OK,
        Json(ReflectFollowupFeedbackResponse {
            status: "ok".to_string(),
            id,
            feedback: response_entry,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(values: &[f32]) -> PgVector {
        PgVector::from(values.to_vec())
    }

    fn unit(id: &str, title: &str, values: &[f32]) -> semantic_work_unit::Model {
        unit_with_source_kind(id, "conversation", title, values)
    }

    fn unit_with_source_kind(
        id: &str,
        source_kind: &str,
        title: &str,
        values: &[f32],
    ) -> semantic_work_unit::Model {
        semantic_work_unit::Model {
            id: id.to_string(),
            source_kind: source_kind.to_string(),
            source_id: id.to_string(),
            conversation_id: if source_kind == "conversation" {
                Some(id.to_string())
            } else {
                None
            },
            project_id: None,
            channel: "web".to_string(),
            title: title.to_string(),
            summary: title.to_string(),
            content_preview: title.to_string(),
            text_hash: stable_hash(title),
            occurred_at: "2026-05-02T00:00:00Z".to_string(),
            period_start: None,
            period_end: None,
            message_count: 2,
            metadata: serde_json::json!({}),
            created_at: "2026-05-02T00:00:00Z".to_string(),
            updated_at: "2026-05-02T00:00:00Z".to_string(),
            embedding: Some(pg(values)),
        }
    }

    fn followup(id: &str, kind: &str, score: f64) -> ReflectSuggestedFollowup {
        ReflectSuggestedFollowup {
            id: id.to_string(),
            kind: kind.to_string(),
            title: id.to_string(),
            detail: String::new(),
            prompt: String::new(),
            status: "ready".to_string(),
            source_label: "Test".to_string(),
            occurred_at: "2026-05-02T00:00:00Z".to_string(),
            conversation_id: None,
            source_unit_id: None,
            rank_score: score,
            search_results: Vec::new(),
            search_checked_at: None,
            search_error: None,
            latest_summary: None,
            latest_summary_generated_at: None,
            latest_summary_error: None,
            latest_summary_evidence_supported: None,
            feedback: None,
            feedback_keys: reflect_followup_feedback_keys(id, None, None),
            feedback_vector: None,
            search_query: None,
            search_planning_context: None,
            search_requires_planning: false,
            source_strategy: ReflectFollowupSourceStrategy::PublicSearch,
            structured_context: serde_json::Value::Null,
            allow_unplanned_source_check: false,
        }
    }

    fn conversation_fixture(id: &str, title: &str) -> conversation::Model {
        conversation::Model {
            id: id.to_string(),
            title: title.to_string(),
            channel: "web".to_string(),
            project_id: None,
            created_at: "2026-05-02T10:00:00Z".to_string(),
            updated_at: "2026-05-02T10:04:00Z".to_string(),
            message_count: 4,
            archived: false,
            starred: false,
        }
    }

    fn chat_message(id: &str, role: &str, content: &str, timestamp: &str) -> message::Model {
        message::Model {
            id: id.to_string(),
            conversation_id: "conv-focus".to_string(),
            role: role.to_string(),
            content: content.to_string(),
            tool_calls_json: None,
            tool_call_id: None,
            provider_message_json: None,
            timestamp: timestamp.to_string(),
            model_used: None,
            trace_id: None,
        }
    }

    #[test]
    fn conversation_candidate_keeps_user_focus_ahead_of_assistant_noise() {
        let conversation = conversation_fixture("conv-focus", "Ambient city chat");
        let candidate = conversation_candidate(
            &conversation,
            vec![
                chat_message(
                    "m1",
                    "user",
                    "The rainy season where I live has been beautiful lately.",
                    "2026-05-02T10:00:00Z",
                ),
                chat_message(
                    "m2",
                    "assistant",
                    "A scenic reply with speculative details that are not useful as user intent.",
                    "2026-05-02T10:01:00Z",
                ),
                chat_message(
                    "m3",
                    "user",
                    "Public affairs around my region are shifting, and I want to understand what changed.",
                    "2026-05-02T10:02:00Z",
                ),
                chat_message(
                    "m4",
                    "assistant",
                    "An unsupported personal name and unrelated imagery should not drive Reflect.",
                    "2026-05-02T10:03:00Z",
                ),
            ],
        )
        .expect("conversation activity should produce a reflect unit");

        assert!(candidate.title.contains("Public affairs around my region"));
        assert!(candidate.summary.contains("what changed"));
        assert!(candidate.content_preview.contains("what changed"));
        assert!(!candidate.summary.contains("unsupported personal name"));
        assert!(candidate.embedding_text.contains("User focus:"));
        assert!(candidate.embedding_text.contains("Assistant context:"));
        assert!(candidate
            .embedding_text
            .contains("Public affairs around my region"));
    }

    #[test]
    fn unplanned_candidates_stay_pending_instead_of_displaying() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let cache = ReflectFollowupSearchCache::default();
        let chat_cluster = freshness_cluster(
            "direct-chat-interest",
            "Compare recent public policy shifts affecting a local community",
            &[0.35, 0.34, 0.31],
        );
        let mut chat_candidate =
            reflect_planned_cluster_latest_suggestion(&chat_cluster, &cache, now)
                .expect("direct chat cluster should produce a planned candidate");
        assert!(chat_candidate.search_requires_planning);
        assert!(!chat_candidate.allow_unplanned_source_check);
        chat_candidate.search_query =
            Some("recent public policy shifts affecting a local community".to_string());

        let (kept_chat, pending_chat) = apply_reflect_external_pursuit_plans(
            vec![chat_candidate],
            &BTreeMap::new(),
            &ReflectFollowupPlanCache::default(),
        );
        assert!(kept_chat.is_empty());
        assert_eq!(pending_chat.len(), 1);

        let memory_cluster = freshness_cluster_with_source_kind(
            "memory-interest",
            "experience_item",
            "Learned private preference that has no source-backed next step yet",
            &[0.35, 0.34, 0.31],
        );
        let memory_candidate =
            reflect_planned_cluster_latest_suggestion(&memory_cluster, &cache, now)
                .expect("memory cluster should still produce a planned candidate");
        assert!(!memory_candidate.allow_unplanned_source_check);
        let (kept, pending) = apply_reflect_external_pursuit_plans(
            vec![memory_candidate],
            &BTreeMap::new(),
            &ReflectFollowupPlanCache::default(),
        );
        assert!(kept.is_empty());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn reflect_plan_parser_preserves_travel_source_strategy_and_context() {
        let plans = parse_reflect_external_pursuit_plans(
            r#"{"plans":[{
                "id":"travel-topic",
                "useful":true,
                "title":"Check Kolkata to Thailand fare options",
                "search_query":"Kolkata to Thailand flexible dates cheap flights 2026",
                "rationale":"The user is considering travel and current prices can help.",
                "source_strategy":"flight_price_discovery",
                "structured_context":{
                    "origin":"Kolkata, India",
                    "destination":"Thailand",
                    "trip_window":"next 365 days",
                    "currency":"INR",
                    "confidence":0.86
                }
            }]}"#,
        );

        let plan = plans.get("travel-topic").unwrap();
        assert_eq!(
            plan.source_strategy,
            ReflectFollowupSourceStrategy::FlightPriceDiscovery
        );
        assert_eq!(plan.structured_context["origin"], "Kolkata, India");
        assert_eq!(plan.structured_context["destination"], "Thailand");
    }

    #[test]
    fn reflect_plan_cache_round_trips_source_strategy() {
        let plan = ReflectExternalPursuitPlan {
            id: "travel-topic".to_string(),
            useful: true,
            title: "Check Kolkata to Thailand fare options".to_string(),
            search_query: "Kolkata to Thailand flexible dates cheap flights 2026".to_string(),
            rationale: "Current prices can help decide.".to_string(),
            source_strategy: ReflectFollowupSourceStrategy::FlightPriceDiscovery,
            structured_context: serde_json::json!({
                "origin": "Kolkata, India",
                "destination": "Thailand"
            }),
        };

        let entry = reflect_plan_cache_entry_from_plan(
            &plan,
            "2026-06-03T00:00:00Z",
            "Kolkata to Thailand travel planning",
        )
        .unwrap();
        assert_eq!(
            entry.source_strategy,
            ReflectFollowupSourceStrategy::FlightPriceDiscovery
        );

        let restored = reflect_plan_from_cache_entry(&entry);
        assert_eq!(
            restored.source_strategy,
            ReflectFollowupSourceStrategy::FlightPriceDiscovery
        );
        assert_eq!(restored.structured_context["destination"], "Thailand");
    }

    #[test]
    fn reflect_plan_parser_accepts_generic_current_sources_alias() {
        let plans = parse_reflect_external_pursuit_plans(
            r#"{"plans":[{
                "id":"generic-topic",
                "useful":true,
                "title":"Check public updates",
                "search_query":"current public updates",
                "rationale":"Fresh sources can help.",
                "source_strategy":"generic_current_sources"
            }]}"#,
        );

        let plan = plans.get("generic-topic").unwrap();
        assert_eq!(
            plan.source_strategy,
            ReflectFollowupSourceStrategy::PublicSearch
        );
    }

    #[test]
    fn reflect_planning_items_use_opaque_handles_instead_of_internal_ids() {
        let mut candidate = followup(
            "latest:planned:0123456789abcdef01234567",
            "latest_developments",
            80.0,
        );
        candidate.search_requires_planning = true;
        candidate.search_planning_context =
            Some("A public-source check could help this reflected topic.".to_string());

        let (items, handles) = build_reflect_external_pursuit_planning_items(&[candidate.clone()]);

        assert_eq!(handles.get("item_1"), Some(&candidate.id));
        assert_eq!(items[0]["id"], "item_1");
        assert_ne!(items[0]["id"], candidate.id);
    }

    #[test]
    fn reflect_planning_remaps_opaque_handles_to_candidate_ids() {
        let mut candidate = followup(
            "latest:planned:0123456789abcdef01234567",
            "latest_developments",
            80.0,
        );
        candidate.search_requires_planning = true;
        let (_items, handles) = build_reflect_external_pursuit_planning_items(&[candidate.clone()]);
        let parsed = parse_reflect_external_pursuit_plans(
            r#"{"plans":[{
                "id":"item_1",
                "useful":true,
                "title":"Check current public updates",
                "search_query":"current public updates",
                "rationale":"Fresh public evidence can help."
            }]}"#,
        );

        let remapped = remap_reflect_external_pursuit_plan_ids(parsed, &handles);

        assert!(remapped.contains_key(&candidate.id));
        assert_eq!(remapped[&candidate.id].id, candidate.id);
        assert!(remapped.get("item_1").is_none());
    }

    #[test]
    fn reflect_planning_drops_unmapped_redacted_ids() {
        let mut candidate = followup(
            "latest:planned:0123456789abcdef01234567",
            "latest_developments",
            80.0,
        );
        candidate.search_requires_planning = true;
        let (_items, handles) = build_reflect_external_pursuit_planning_items(&[candidate]);
        let parsed = parse_reflect_external_pursuit_plans(
            r#"{"plans":[{
                "id":"latest:planned:[REDACTED_SECRET]",
                "useful":true,
                "title":"Check current public updates",
                "search_query":"current public updates",
                "rationale":"Fresh public evidence can help."
            }]}"#,
        );

        let remapped = remap_reflect_external_pursuit_plan_ids(parsed, &handles);

        assert!(remapped.is_empty());
    }

    #[test]
    fn travel_query_builder_uses_public_origin_destination_and_flexibility() {
        let query = reflect_travel_price_search_query(
            "Kolkata to Thailand flexible dates cheap flights 2026",
            &serde_json::json!({
                "origin": "Kolkata, India",
                "destination": "Thailand",
                "trip_window": "next 365 days",
                "currency": "INR"
            }),
        )
        .unwrap();

        assert!(query.contains("Kolkata"));
        assert!(query.contains("Thailand"));
        assert!(query.contains("flexible"));
        assert!(query.contains("INR"));
        assert!(reflect_external_search_query_is_safe(&query));
    }

    #[test]
    fn travel_query_builder_drops_sensitive_context() {
        let query = reflect_travel_price_search_query(
            "friend password abc1234567890123456789012345678901234567890 Thailand",
            &serde_json::json!({
                "origin": "Kolkata",
                "destination": "Thailand",
                "traveler_email": "person@example.com",
                "password": "abc1234567890123456789012345678901234567890"
            }),
        )
        .unwrap();

        assert!(query.contains("Kolkata"));
        assert!(query.contains("Thailand"));
        assert!(!query.contains("person@example.com"));
        assert!(!query.contains("password"));
        assert!(reflect_external_search_query_is_safe(&query));
    }

    #[test]
    fn travel_query_builder_requires_route_context() {
        assert!(reflect_travel_price_search_query(
            "I stay in Kolkata",
            &serde_json::json!({ "origin": "Kolkata, India" }),
        )
        .is_none());
    }

    #[test]
    fn travel_query_builder_requires_origin_and_destination() {
        assert!(reflect_travel_price_search_query(
            "Thailand vacation flight prices",
            &serde_json::json!({ "destination": "Thailand" }),
        )
        .is_none());
    }

    #[test]
    fn travel_summary_prompt_disclaims_generic_search_limitations() {
        let prompt = reflect_followup_summary_system_prompt(
            ReflectFollowupSourceStrategy::FlightPriceDiscovery,
        );

        assert!(prompt.contains("flight"));
        assert!(prompt.contains("Do not claim"));
        assert!(prompt.contains("calendar-price"));
    }

    #[test]
    fn latest_summary_fields_require_supported_evidence_verdict() {
        let entry = ReflectFollowupSearchEntry {
            source_id: "topic".to_string(),
            query: "India Iran conflict latest implications".to_string(),
            checked_at: "2026-06-03T11:55:00Z".to_string(),
            backend: Some("playwright".to_string()),
            results: vec![
                ReflectFollowupSearchResult {
                    title: "India Today - Iran latest news".to_string(),
                    url: "https://www.indiatoday.in/topic/iran".to_string(),
                    snippet: "India Today reports recent developments around Iran.".to_string(),
                    source: "India Today".to_string(),
                    published_date: Some("2026-06-01".to_string()),
                },
                ReflectFollowupSearchResult {
                    title: "Diplomatic position".to_string(),
                    url: "https://www.dw.com/example".to_string(),
                    snippet: "Analysis covers India's multi-alignment diplomacy.".to_string(),
                    source: "DW".to_string(),
                    published_date: None,
                },
            ],
            error: None,
            summary: None,
            summary_generated_at: None,
            summary_error: Some("Latest-development summary timed out.".to_string()),
            summary_evidence_supported: None,
            ..Default::default()
        };

        let (summary, generated_at, error) = reflect_followup_latest_summary_fields(Some(&entry));

        assert!(summary.is_none());
        assert!(generated_at.is_none());
        assert_eq!(
            error.as_deref(),
            Some("Latest-development summary timed out.")
        );
    }

    #[test]
    fn legacy_summary_without_evidence_verdict_is_due_for_recheck() {
        let entry = ReflectFollowupSearchEntry {
            source_id: "topic".to_string(),
            query: "current bilateral development".to_string(),
            checked_at: "2026-06-03T11:55:00Z".to_string(),
            backend: Some("bing_rss".to_string()),
            results: vec![ReflectFollowupSearchResult {
                title: "Current public update".to_string(),
                url: "https://example.com/update".to_string(),
                snippet: "A source snippet relevant to the requested public update.".to_string(),
                source: "Example".to_string(),
                published_date: Some("2026-06-03".to_string()),
            }],
            error: None,
            summary: Some("A previously cached free-text summary.".to_string()),
            summary_generated_at: Some("2026-06-03T12:00:00Z".to_string()),
            summary_error: None,
            summary_evidence_supported: None,
            ..Default::default()
        };

        assert!(reflect_followup_summary_is_due(&entry));
    }

    #[test]
    fn legacy_summary_error_without_attempt_time_is_due_for_recheck() {
        let entry = ReflectFollowupSearchEntry {
            source_id: "topic".to_string(),
            query: "current public development".to_string(),
            checked_at: "2026-06-03T11:55:00Z".to_string(),
            backend: Some("bing_rss".to_string()),
            results: vec![ReflectFollowupSearchResult {
                title: "Current public update".to_string(),
                url: "https://example.com/update".to_string(),
                snippet: "A source snippet relevant to the requested public update.".to_string(),
                source: "Example".to_string(),
                published_date: Some("2026-06-03".to_string()),
            }],
            error: None,
            summary: None,
            summary_generated_at: None,
            summary_error: Some("Summary worker pending.".to_string()),
            summary_evidence_supported: None,
            ..Default::default()
        };

        assert!(reflect_followup_summary_is_due(&entry));
    }

    #[test]
    fn summary_error_with_attempt_time_is_not_due_immediately() {
        let entry = ReflectFollowupSearchEntry {
            source_id: "topic".to_string(),
            query: "current public development".to_string(),
            checked_at: "2026-06-03T11:55:00Z".to_string(),
            backend: Some("bing_rss".to_string()),
            results: vec![ReflectFollowupSearchResult {
                title: "Current public update".to_string(),
                url: "https://example.com/update".to_string(),
                snippet: "A source snippet relevant to the requested public update.".to_string(),
                source: "Example".to_string(),
                published_date: Some("2026-06-03".to_string()),
            }],
            error: None,
            summary: None,
            summary_generated_at: Some("2026-06-03T12:00:00Z".to_string()),
            summary_error: Some("Latest-development summary timed out.".to_string()),
            summary_evidence_supported: None,
            ..Default::default()
        };

        assert!(!reflect_followup_summary_is_due(&entry));
    }

    #[test]
    fn unsupported_summary_verdict_does_not_fall_back_to_source_cards() {
        let entry = ReflectFollowupSearchEntry {
            source_id: "topic".to_string(),
            query: "specific current public update".to_string(),
            checked_at: "2026-06-03T11:55:00Z".to_string(),
            backend: Some("bing_rss".to_string()),
            results: vec![
                ReflectFollowupSearchResult {
                    title: "Broad topic reference".to_string(),
                    url: "https://example.com/reference".to_string(),
                    snippet: "General background about one entity.".to_string(),
                    source: "Example".to_string(),
                    published_date: Some("2026-06-03".to_string()),
                },
                ReflectFollowupSearchResult {
                    title: "Live topic hub".to_string(),
                    url: "https://example.com/live".to_string(),
                    snippet: "A page description for latest developments, photos, and maps."
                        .to_string(),
                    source: "Example News".to_string(),
                    published_date: Some("2026-06-03".to_string()),
                },
            ],
            error: None,
            summary: None,
            summary_generated_at: None,
            summary_error: Some(
                "Source snippets do not directly support the requested topic.".to_string(),
            ),
            summary_evidence_supported: Some(false),
            ..Default::default()
        };

        let (summary, generated_at, error) = reflect_followup_latest_summary_fields(Some(&entry));

        assert!(summary.is_none());
        assert!(generated_at.is_none());
        assert_eq!(
            error.as_deref(),
            Some("Source snippets do not directly support the requested topic.")
        );
    }

    fn freshness_context() -> ReflectSemanticFreshnessContext {
        ReflectSemanticFreshnessContext {
            public_development: vec![1.0, 0.0, 0.0],
            private_work: vec![0.0, 1.0, 0.0],
            dimension: 3,
        }
    }

    fn freshness_cluster(id: &str, title: &str, values: &[f32]) -> ReflectClusterResponse {
        freshness_cluster_with_source_kind(id, "conversation", title, values)
    }

    fn freshness_cluster_with_source_kind(
        id: &str,
        source_kind: &str,
        title: &str,
        values: &[f32],
    ) -> ReflectClusterResponse {
        let source = unit_with_source_kind(id, source_kind, title, values);
        let mut source_mix = BTreeMap::new();
        source_mix.insert(source_label(source_kind, &source.channel), 1);
        ReflectClusterResponse {
            id: format!("cluster-{}", id),
            representative_unit_id: source.id.clone(),
            centroid_embedding: source.embedding.clone(),
            label: source.title.clone(),
            plain_summary: source.summary.clone(),
            unit_count: 1,
            message_count: source.message_count,
            source_mix,
            color: "#00ffaa".to_string(),
            related_history: ReflectRelatedHistory::unavailable("test"),
            units: vec![unit_to_response(&source)],
        }
    }

    fn experience_run_with_tools(
        task_type: Option<&str>,
        tool_names: &[&str],
    ) -> experience_run::Model {
        experience_run::Model {
            id: "run-test".to_string(),
            execution_run_id: None,
            trace_id: None,
            conversation_id: None,
            project_id: None,
            channel: "web".to_string(),
            scope: "chat".to_string(),
            intent_key: "intent".to_string(),
            task_type: task_type.map(str::to_string),
            request_text: Some("Compare current public updates for a topic.".to_string()),
            tool_sequence_digest: None,
            tool_sequence_json: serde_json::json!(tool_names
                .iter()
                .map(|name| serde_json::json!({
                    "name": name,
                    "status": "success",
                    "started_at": serde_json::Value::Null,
                    "completed_at": serde_json::Value::Null,
                }))
                .collect::<Vec<_>>()),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: None,
            tokens_in: None,
            tokens_out: None,
            wall_ms: None,
            est_cost_microusd: None,
            success_state: "accepted".to_string(),
            correction_state: "none".to_string(),
            outcome_summary: None,
            failure_reason: None,
            metadata: serde_json::json!({}),
            consolidated: false,
            accepted_at: None,
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: None,
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: "2026-05-02T00:00:00Z".to_string(),
            updated_at: "2026-05-02T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn clusters_by_embedding_geometry_not_wording() {
        let units = vec![
            unit("a", "React dashboard work", &[1.0, 0.0, 0.0]),
            unit("b", "UI chart polish", &[0.96, 0.04, 0.0]),
            unit("c", "Rust storage layer", &[0.0, 1.0, 0.0]),
            unit("d", "Database reflection model", &[0.0, 0.96, 0.04]),
        ];
        let (clusters, _, status) = build_clusters(&units);
        assert_eq!(status.mode, "semantic");
        assert!(clusters.len() >= 2);
        assert!(clusters.iter().any(|cluster| cluster.unit_count >= 2));
    }

    #[test]
    fn suggested_followups_are_capped_and_diversified() {
        let selected = select_top_reflect_followups(vec![
            followup("f1", "recovery_advice", 100.0),
            followup("f2", "recovery_advice", 99.0),
            followup("f3", "recovery_advice", 98.0),
            followup("f4", "recovery_advice", 97.0),
            followup("f5", "recovery_advice", 96.0),
            followup("l1", "latest_developments", 97.0),
            followup("l2", "latest_developments", 96.0),
        ]);
        assert_eq!(selected.len(), REFLECT_MAX_SUGGESTED_FOLLOWUPS);
        assert!(selected
            .iter()
            .any(|item| item.kind == "latest_developments"));
        assert!(
            selected
                .iter()
                .filter(|item| item.kind == "recovery_advice")
                .count()
                <= 4
        );
    }

    #[test]
    fn latest_research_followup_queues_from_structured_research_signal() {
        let run = experience_run_with_tools(Some("research"), &["web_search"]);
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();

        let suggestion =
            reflect_latest_suggestion(&run, &ReflectFollowupSearchCache::default(), now);

        assert_eq!(suggestion.kind, "latest_developments");
        assert_eq!(suggestion.status, "queued");
        assert!(suggestion.search_requires_planning);
        assert!(suggestion.search_planning_context.is_some());
        assert!(suggestion.search_results.is_empty());
    }

    #[test]
    fn selected_followups_collapse_near_duplicate_recovery_threads() {
        let mut first = followup("thread-a", "recovery_advice", 90.0);
        first.title = "Set up the MCP server for AgentArk".to_string();
        first.detail =
            "Review the earlier integration request and decide the next step.".to_string();
        first.prompt =
            "Review this reflected thread and help decide the next concrete step.".to_string();

        let mut second = followup("thread-b", "recovery_advice", 88.0);
        second.title = "Configure AgentArk MCP integration".to_string();
        second.detail = "Continue the prior setup request for the same MCP server.".to_string();
        second.prompt =
            "Review this reflected thread and help decide the next concrete step.".to_string();

        let mut distinct = followup("thread-c", "recovery_advice", 86.0);
        distinct.title = "Compare deployment options for a dashboard".to_string();
        distinct.detail = "A separate reflected workstream deserves review.".to_string();

        let selected = select_top_reflect_followups(vec![first, second, distinct]);

        assert_eq!(selected.len(), 2);
        assert!(selected.iter().any(|item| item.id == "thread-a"));
        assert!(selected.iter().any(|item| item.id == "thread-c"));
        assert!(!selected.iter().any(|item| item.id == "thread-b"));
    }

    #[test]
    fn dismissed_semantic_followup_stays_hidden_until_newer_interest() {
        let mut store = ReflectFollowupFeedbackStore::default();
        store.entries.insert(
            "semantic-topic".to_string(),
            ReflectFollowupFeedbackState {
                dismiss_count: 1,
                last_action: Some("dismiss".to_string()),
                last_at: Some("2026-05-03T00:00:00Z".to_string()),
                semantic_vector: Some(vec![1.0, 0.0, 0.0]),
                ..ReflectFollowupFeedbackState::default()
            },
        );

        let stale_feedback = reflect_followup_refresh_feedback_for_new_evidence(
            reflect_followup_semantic_feedback(Some(&[0.99, 0.01, 0.0]), &store),
            "2026-05-02T00:00:00Z",
        )
        .unwrap();
        assert!(reflect_followup_is_dismissed(Some(&stale_feedback)));

        let renewed_feedback = reflect_followup_refresh_feedback_for_new_evidence(
            reflect_followup_semantic_feedback(Some(&[0.99, 0.01, 0.0]), &store),
            "2026-05-04T00:00:00Z",
        )
        .unwrap();
        assert!(renewed_feedback.renewed_after_feedback);
        assert!(!reflect_followup_is_dismissed(Some(&renewed_feedback)));
    }

    #[test]
    fn research_detection_reads_production_and_legacy_tool_keys() {
        // The production writer serializes tool entries under "name"
        // (experience_tool_sequence_json in message_processing.rs); the chat
        // pipeline always stamps task_type "chat".
        let production = experience_run_with_tools(Some("chat"), &["web_search"]);
        assert!(reflect_experience_run_is_research(&production));

        let browsing = experience_run_with_tools(Some("chat"), &["browser_navigate"]);
        assert!(reflect_experience_run_is_research(&browsing));

        let mut legacy = experience_run_with_tools(Some("chat"), &[]);
        legacy.tool_sequence_json = serde_json::json!([{ "tool_name": "page_fetch" }]);
        assert!(reflect_experience_run_is_research(&legacy));

        let chat_only = experience_run_with_tools(Some("chat"), &["calendar_read"]);
        assert!(!reflect_experience_run_is_research(&chat_only));
    }

    #[test]
    fn latest_followup_uses_structured_research_run_signals() {
        let by_task_type = experience_run_with_tools(Some("research"), &[]);
        assert!(reflect_experience_run_is_research(&by_task_type));

        let by_tool_sequence = experience_run_with_tools(None, &["web_search"]);
        assert!(reflect_experience_run_is_research(&by_tool_sequence));

        let unrelated = experience_run_with_tools(Some("app_deploy"), &["app_deploy"]);
        assert!(!reflect_experience_run_is_research(&unrelated));

        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let topic = reflect_sentence_fragment(
            &reflect_experience_run_topic(&by_tool_sequence),
            REFLECT_SUGGESTION_TEXT_CHARS,
        );
        let source_id = format!(
            "latest:{}",
            stable_hash(&topic).chars().take(24).collect::<String>()
        );
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            source_id,
            ReflectFollowupSearchEntry {
                source_id: "latest-test".to_string(),
                query: topic,
                checked_at: "2026-05-06T11:55:00Z".to_string(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "Fresh public update".to_string(),
                    url: "https://example.com/update".to_string(),
                    snippet: "A source-backed result appears in Reflect.".to_string(),
                    source: "Example".to_string(),
                    published_date: Some("2026-05-06".to_string()),
                }],
                error: None,
                summary: Some("A source-backed insight summary.".to_string()),
                summary_generated_at: Some("2026-05-06T11:56:00Z".to_string()),
                summary_error: None,
                summary_evidence_supported: Some(true),
                ..Default::default()
            },
        );
        let suggestion = reflect_latest_suggestion(&by_tool_sequence, &cache, now);
        assert_eq!(suggestion.status, "ready");
        assert_eq!(suggestion.search_results.len(), 1);
        assert!(suggestion.latest_summary.is_some());
        assert_eq!(
            suggestion.search_results[0].url,
            "https://example.com/update"
        );
    }

    #[test]
    fn semantic_freshness_followup_requires_planner_public_query() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let context = freshness_context();
        let cluster = freshness_cluster("fresh", "Cross-border news tracking", &[0.99, 0.01, 0.0]);
        let score = reflect_semantic_freshness_score(
            cluster.centroid_embedding.as_ref().unwrap(),
            &context,
        )
        .unwrap();
        let suggestion = reflect_semantic_cluster_latest_suggestion(
            &cluster,
            &ReflectFollowupSearchCache::default(),
            now,
            score,
        )
        .unwrap();
        assert_eq!(suggestion.kind, "latest_developments");
        assert_eq!(suggestion.status, "queued");
        assert!(suggestion.search_query.is_some());
        assert!(suggestion.search_requires_planning);
    }

    #[test]
    fn semantic_freshness_followup_uses_cached_source_check_after_planner() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let context = freshness_context();
        let cluster = freshness_cluster("fresh", "Cross-border news tracking", &[0.99, 0.01, 0.0]);
        let score = reflect_semantic_freshness_score(
            cluster.centroid_embedding.as_ref().unwrap(),
            &context,
        )
        .unwrap();
        let topic = reflect_cluster_latest_topic(&cluster);
        let source_id = format!(
            "latest:semantic:{}",
            stable_hash(&format!("{}:{}", cluster.representative_unit_id, topic))
                .chars()
                .take(24)
                .collect::<String>()
        );
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            source_id.clone(),
            ReflectFollowupSearchEntry {
                source_id,
                query: "current cross-border news tracking sources".to_string(),
                checked_at: "2026-05-06T11:30:00Z".to_string(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "Fresh public update".to_string(),
                    url: "https://example.com/update".to_string(),
                    snippet: "A source-backed result appears in Reflect.".to_string(),
                    source: "Example".to_string(),
                    published_date: Some("2026-05-06".to_string()),
                }],
                error: None,
                summary: None,
                summary_generated_at: None,
                summary_error: None,
                ..Default::default()
            },
        );

        let suggestion =
            reflect_semantic_cluster_latest_suggestion(&cluster, &cache, now, score).unwrap();
        assert!(suggestion.search_requires_planning);
        assert_eq!(
            suggestion.search_query.as_deref(),
            Some("current cross-border news tracking sources")
        );
        assert_eq!(suggestion.search_results.len(), 1);

        let mut plan_cache = ReflectFollowupPlanCache::default();
        plan_cache.entries.insert(
            suggestion.id.clone(),
            ReflectFollowupPlanEntry {
                id: suggestion.id.clone(),
                useful: true,
                title: "Track current cross-border news".to_string(),
                search_query: "current cross-border news tracking sources".to_string(),
                rationale: "Fresh source evidence can help monitor the public developments."
                    .to_string(),
                planned_at: "2026-05-06T11:00:00Z".to_string(),
                ..Default::default()
            },
        );
        let (planned, _) =
            apply_reflect_external_pursuit_plans(vec![suggestion], &BTreeMap::new(), &plan_cache);

        assert_eq!(planned.len(), 1);
        assert!(!planned[0].search_requires_planning);
        assert_eq!(planned[0].search_results.len(), 1);
    }

    #[test]
    fn semantic_freshness_ignores_private_local_work() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let context = freshness_context();
        let cluster = freshness_cluster("local", "Local app settings cleanup", &[0.01, 0.99, 0.0]);
        let score = reflect_semantic_freshness_score(
            cluster.centroid_embedding.as_ref().unwrap(),
            &context,
        )
        .unwrap();
        assert!(!reflect_semantic_freshness_is_actionable(score));
        assert!(reflect_semantic_cluster_latest_suggestion(
            &cluster,
            &ReflectFollowupSearchCache::default(),
            now,
            score,
        )
        .is_none());
    }

    #[test]
    fn planned_latest_followup_skips_system_only_clusters() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let cache = ReflectFollowupSearchCache::default();
        let system_cluster = freshness_cluster_with_source_kind(
            "pulse",
            "arkpulse",
            "No critical incidents",
            &[0.5, 0.5, 0.0],
        );
        assert!(reflect_planned_cluster_latest_suggestion(&system_cluster, &cache, now).is_none());

        let user_cluster = freshness_cluster(
            "research",
            "Research an external market update",
            &[0.7, 0.2, 0.1],
        );
        let suggestion =
            reflect_planned_cluster_latest_suggestion(&user_cluster, &cache, now).unwrap();
        assert_eq!(suggestion.kind, "latest_developments");
        assert!(suggestion.search_requires_planning);
    }

    #[test]
    fn planned_latest_followup_uses_cached_source_check_without_replanning() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let user_cluster = freshness_cluster(
            "research",
            "Research an external market update",
            &[0.7, 0.2, 0.1],
        );
        let topic = reflect_cluster_latest_topic(&user_cluster);
        let source_id = format!(
            "latest:planned:{}",
            stable_hash(&format!("{}:{}", user_cluster.id, topic))
                .chars()
                .take(24)
                .collect::<String>()
        );
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            source_id.clone(),
            ReflectFollowupSearchEntry {
                source_id,
                query: "external market update current sources".to_string(),
                checked_at: "2026-05-06T11:30:00Z".to_string(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "Market source".to_string(),
                    url: "https://example.com/market".to_string(),
                    snippet: "A current source is cached for this reflected topic.".to_string(),
                    source: "Example".to_string(),
                    published_date: Some("2026-05-06".to_string()),
                }],
                error: None,
                summary: Some("Cached source-backed insight.".to_string()),
                summary_generated_at: Some("2026-05-06T11:40:00Z".to_string()),
                summary_error: None,
                summary_evidence_supported: Some(true),
                ..Default::default()
            },
        );

        let suggestion =
            reflect_planned_cluster_latest_suggestion(&user_cluster, &cache, now).unwrap();
        assert!(suggestion.search_requires_planning);

        let mut plan_cache = ReflectFollowupPlanCache::default();
        plan_cache.entries.insert(
            suggestion.id.clone(),
            ReflectFollowupPlanEntry {
                id: suggestion.id.clone(),
                useful: true,
                title: "Track current external market updates".to_string(),
                search_query: "external market update current sources".to_string(),
                rationale: "Fresh source evidence can help evaluate the market update.".to_string(),
                planned_at: "2026-05-06T11:00:00Z".to_string(),
                ..Default::default()
            },
        );
        let (planned, _) =
            apply_reflect_external_pursuit_plans(vec![suggestion], &BTreeMap::new(), &plan_cache);

        assert_eq!(planned.len(), 1);
        assert!(!planned[0].search_requires_planning);
        assert_eq!(
            planned[0].search_query.as_deref(),
            Some("external market update current sources")
        );
        assert_eq!(planned[0].search_results.len(), 1);
        assert!(planned[0].latest_summary.is_some());
    }

    #[test]
    fn planned_latest_followup_keeps_cache_when_only_current_year_was_added() {
        let now = chrono::Utc::now();
        let user_cluster =
            freshness_cluster("research", "Research Iran India news", &[0.7, 0.2, 0.1]);
        let topic = reflect_cluster_latest_topic(&user_cluster);
        let source_id = format!(
            "latest:planned:{}",
            stable_hash(&format!("{}:{}", user_cluster.id, topic))
                .chars()
                .take(24)
                .collect::<String>()
        );
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            source_id.clone(),
            ReflectFollowupSearchEntry {
                source_id,
                query: "Iran India news latest".to_string(),
                checked_at: now.to_rfc3339(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "India-Iran source".to_string(),
                    url: "https://example.com/india-iran".to_string(),
                    snippet: "A cached source result for Iran and India news.".to_string(),
                    source: "Example".to_string(),
                    published_date: Some(now.date_naive().to_string()),
                }],
                error: None,
                summary: Some("Cached India-Iran source-backed insight.".to_string()),
                summary_generated_at: Some(now.to_rfc3339()),
                summary_error: None,
                summary_evidence_supported: Some(true),
                ..Default::default()
            },
        );

        let suggestion =
            reflect_planned_cluster_latest_suggestion(&user_cluster, &cache, now).unwrap();
        let mut plan_cache = ReflectFollowupPlanCache::default();
        plan_cache.entries.insert(
            suggestion.id.clone(),
            ReflectFollowupPlanEntry {
                id: suggestion.id.clone(),
                useful: true,
                title: "Latest Iran-India news".to_string(),
                search_query: format!("Iran India news latest {}", now.year()),
                rationale: "Fresh public evidence can help.".to_string(),
                planned_at: now.to_rfc3339(),
                ..Default::default()
            },
        );

        let (planned, _) =
            apply_reflect_external_pursuit_plans(vec![suggestion], &BTreeMap::new(), &plan_cache);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].search_results.len(), 1);
        assert!(planned[0].latest_summary.is_some());
        assert_eq!(planned[0].status, "ready");
    }

    #[test]
    fn planned_latest_followup_cached_search_still_requires_planner_classification() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let user_cluster = freshness_cluster(
            "location",
            "Private location memory reliability check",
            &[0.7, 0.2, 0.1],
        );
        let topic = reflect_cluster_latest_topic(&user_cluster);
        let source_id = format!(
            "latest:planned:{}",
            stable_hash(&format!("{}:{}", user_cluster.id, topic))
                .chars()
                .take(24)
                .collect::<String>()
        );
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            source_id.clone(),
            ReflectFollowupSearchEntry {
                source_id,
                query: "private location memory reliability check".to_string(),
                checked_at: "2026-05-06T11:30:00Z".to_string(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "Generic location API documentation".to_string(),
                    url: "https://example.com/location-api".to_string(),
                    snippet: "Documentation about API location detection.".to_string(),
                    source: "Example Docs".to_string(),
                    published_date: None,
                }],
                error: None,
                summary: Some("Cached but unplanned source result.".to_string()),
                summary_generated_at: Some("2026-05-06T11:40:00Z".to_string()),
                summary_error: None,
                ..Default::default()
            },
        );

        let suggestion =
            reflect_planned_cluster_latest_suggestion(&user_cluster, &cache, now).unwrap();

        assert!(suggestion.search_requires_planning);
        let (planned, pending) = apply_reflect_external_pursuit_plans(
            vec![suggestion],
            &BTreeMap::new(),
            &ReflectFollowupPlanCache::default(),
        );
        assert!(planned.is_empty());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn external_pursuit_planning_drops_unuseful_memory_candidates() {
        let mut candidate = followup("memory-topic", "latest_developments", 80.0);
        candidate.search_requires_planning = true;
        candidate.search_query = Some("Learned user memory about an ordinary nickname".to_string());
        let mut plans = BTreeMap::new();
        plans.insert(
            candidate.id.clone(),
            ReflectExternalPursuitPlan {
                id: candidate.id.clone(),
                useful: false,
                title: String::new(),
                search_query: String::new(),
                rationale: "No public pursuit implied.".to_string(),
                ..Default::default()
            },
        );

        let (planned, pending) = apply_reflect_external_pursuit_plans(
            vec![candidate],
            &plans,
            &ReflectFollowupPlanCache::default(),
        );
        assert!(planned.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn external_pursuit_planning_holds_private_memory_pending_without_plan() {
        let mut candidate = followup("memory-topic", "latest_developments", 80.0);
        candidate.title = "Learned user memory".to_string();
        candidate.search_requires_planning = true;
        candidate.search_query = Some("Learned user memory about an ordinary nickname".to_string());
        candidate.search_planning_context = Some(
            "source_kind: experience_item\nsource_label: Memory\ntitle: Learned user memory\nsummary: ordinary nickname"
                .to_string(),
        );

        let (planned, pending) = apply_reflect_external_pursuit_plans(
            vec![candidate],
            &BTreeMap::new(),
            &ReflectFollowupPlanCache::default(),
        );

        assert!(planned.is_empty());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn external_pursuit_planning_reuses_cached_useful_plan_without_llm() {
        let mut candidate = followup("cloud-pricing-topic", "latest_developments", 80.0);
        candidate.title = "GMI Cloud GPU Pricing Comparison".to_string();
        candidate.detail = "Compare options before using cloud GPU providers.".to_string();
        candidate.search_query = Some("GMI Cloud GPU Pricing Comparison".to_string());
        candidate.search_requires_planning = true;

        let mut cache = ReflectFollowupPlanCache::default();
        cache.entries.insert(
            candidate.id.clone(),
            ReflectFollowupPlanEntry {
                id: candidate.id.clone(),
                useful: true,
                title: "Compare current GPU cloud pricing options".to_string(),
                search_query: "GMI Cloud GPU pricing comparison alternatives 2026".to_string(),
                rationale:
                    "Current provider pricing and alternatives can change and help the user decide."
                        .to_string(),
                planned_at: "2026-05-21T00:00:00Z".to_string(),
                ..Default::default()
            },
        );

        let (planned, _) =
            apply_reflect_external_pursuit_plans(vec![candidate], &BTreeMap::new(), &cache);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].kind, "latest_developments");
        assert_eq!(
            planned[0].title,
            "Compare current GPU cloud pricing options"
        );
        assert_eq!(
            planned[0].search_query.as_deref(),
            Some("GMI Cloud GPU pricing comparison alternatives 2026")
        );
        assert_eq!(planned[0].status, "queued");
    }

    #[test]
    fn external_pursuit_planning_rewrites_useful_memory_candidates() {
        let mut candidate = followup("location-topic", "latest_developments", 80.0);
        candidate.search_requires_planning = true;
        candidate.search_query = Some("Learned user memory about a home location".to_string());
        let mut plans = BTreeMap::new();
        plans.insert(
            candidate.id.clone(),
            ReflectExternalPursuitPlan {
                id: candidate.id.clone(),
                useful: true,
                title: "Places worth exploring nearby".to_string(),
                search_query: "places of interest near a Kolkata neighborhood".to_string(),
                rationale: "The reflected location can support a public local discovery check."
                    .to_string(),
                ..Default::default()
            },
        );

        let (planned, _) = apply_reflect_external_pursuit_plans(
            vec![candidate],
            &plans,
            &ReflectFollowupPlanCache::default(),
        );
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].title, "Places worth exploring nearby");
        assert_eq!(
            planned[0].search_query.as_deref(),
            Some("places of interest near a Kolkata neighborhood")
        );
        assert!(planned[0].rank_score > 80.0);
    }

    #[test]
    fn external_pursuit_planning_applies_travel_strategy_to_candidate() {
        let mut candidate = followup("travel-topic", "latest_developments", 80.0);
        candidate.search_requires_planning = true;
        candidate.search_query = Some("Kolkata to Thailand travel planning".to_string());
        let mut plans = BTreeMap::new();
        plans.insert(
            candidate.id.clone(),
            ReflectExternalPursuitPlan {
                id: candidate.id.clone(),
                useful: true,
                title: "Check Kolkata to Thailand fare options".to_string(),
                search_query: "Kolkata to Thailand flexible dates cheap flights 2026".to_string(),
                rationale: "Current route prices can help decide whether to plan the trip."
                    .to_string(),
                source_strategy: ReflectFollowupSourceStrategy::FlightPriceDiscovery,
                structured_context: serde_json::json!({
                    "origin": "Kolkata, India",
                    "destination": "Thailand",
                    "trip_window": "next 365 days",
                    "currency": "INR"
                }),
            },
        );

        let (planned, _) = apply_reflect_external_pursuit_plans(
            vec![candidate],
            &plans,
            &ReflectFollowupPlanCache::default(),
        );

        assert_eq!(planned.len(), 1);
        assert_eq!(
            planned[0].source_strategy,
            ReflectFollowupSourceStrategy::FlightPriceDiscovery
        );
        assert_eq!(planned[0].structured_context["destination"], "Thailand");
    }

    #[test]
    fn external_pursuit_planning_drops_sensitive_search_queries() {
        let mut candidate = followup("sensitive-topic", "latest_developments", 80.0);
        candidate.search_requires_planning = true;
        candidate.search_query = Some("private token context".to_string());
        let mut plans = BTreeMap::new();
        plans.insert(
            candidate.id.clone(),
            ReflectExternalPursuitPlan {
                id: candidate.id.clone(),
                useful: true,
                title: "Unsafe query should not survive".to_string(),
                search_query: "access_token sk_test_1234567890123456789012345678901234567890"
                    .to_string(),
                rationale: "The query contains credential-shaped material.".to_string(),
                ..Default::default()
            },
        );

        let (planned, pending) = apply_reflect_external_pursuit_plans(
            vec![candidate],
            &plans,
            &ReflectFollowupPlanCache::default(),
        );
        assert!(planned.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn plan_cache_reuses_verdicts_by_topic_meaning_across_id_churn() {
        let mut candidate = followup("latest:planned:new-id", "latest_developments", 80.0);
        candidate.title = "GMI cloud GPU pricing comparison".to_string();
        candidate.search_requires_planning = true;
        candidate.search_query =
            Some("GMI cloud GPU pricing comparison before committing".to_string());

        let mut cache = ReflectFollowupPlanCache::default();
        cache.entries.insert(
            "latest:planned:old-id".to_string(),
            ReflectFollowupPlanEntry {
                id: "latest:planned:old-id".to_string(),
                useful: true,
                title: "Compare current GPU cloud pricing options".to_string(),
                search_query: "GMI Cloud GPU pricing comparison alternatives 2026".to_string(),
                rationale: "Current provider pricing changes often.".to_string(),
                planned_at: "2026-06-01T00:00:00Z".to_string(),
                topic: "GMI cloud GPU pricing comparison before committing to a provider"
                    .to_string(),
                ..Default::default()
            },
        );

        let (planned, pending) =
            apply_reflect_external_pursuit_plans(vec![candidate], &BTreeMap::new(), &cache);

        assert!(pending.is_empty());
        assert_eq!(planned.len(), 1);
        assert_eq!(
            planned[0].title,
            "Compare current GPU cloud pricing options"
        );
    }

    #[test]
    fn plan_cache_topic_reuse_rejects_distinct_topics_sharing_scheme_tokens() {
        // Same-family topics share scheme words ("learned user memory") but
        // have disjoint content; the verdict must NOT transfer.
        let mut candidate = followup("latest:planned:color-id", "latest_developments", 80.0);
        candidate.title = "Learned user memory favorite color".to_string();
        candidate.search_requires_planning = true;
        candidate.search_query = Some("Learned user memory favorite color".to_string());

        let mut cache = ReflectFollowupPlanCache::default();
        cache.entries.insert(
            "latest:planned:nickname-id".to_string(),
            ReflectFollowupPlanEntry {
                id: "latest:planned:nickname-id".to_string(),
                useful: true,
                title: "Nickname etymology resources".to_string(),
                search_query: "common nickname origins".to_string(),
                rationale: "Public references exist.".to_string(),
                planned_at: "2026-06-01T00:00:00Z".to_string(),
                topic: "Learned user memory ordinary nickname".to_string(),
                ..Default::default()
            },
        );

        let (planned, pending) =
            apply_reflect_external_pursuit_plans(vec![candidate], &BTreeMap::new(), &cache);

        assert!(planned.is_empty());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn plan_cache_never_transfers_negative_verdicts_by_similarity() {
        // A not-useful verdict for a same-meaning topic under a different id
        // must not silently suppress the candidate; it stays pending and gets
        // one cheap re-plan instead.
        let mut candidate = followup("latest:planned:new-id", "latest_developments", 80.0);
        candidate.title = "GMI cloud GPU pricing comparison".to_string();
        candidate.search_requires_planning = true;
        candidate.search_query =
            Some("GMI cloud GPU pricing comparison before committing".to_string());

        let mut cache = ReflectFollowupPlanCache::default();
        cache.entries.insert(
            "latest:planned:old-id".to_string(),
            ReflectFollowupPlanEntry {
                id: "latest:planned:old-id".to_string(),
                useful: false,
                planned_at: "2026-06-01T00:00:00Z".to_string(),
                topic: "GMI cloud GPU pricing comparison before committing to a provider"
                    .to_string(),
                ..Default::default()
            },
        );

        let (planned, pending) =
            apply_reflect_external_pursuit_plans(vec![candidate], &BTreeMap::new(), &cache);

        assert!(planned.is_empty());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn unusable_useful_verdicts_are_cached_as_short_lived_negatives() {
        let plan = ReflectExternalPursuitPlan {
            id: "defective-topic".to_string(),
            useful: true,
            title: "Has a title".to_string(),
            search_query: String::new(),
            rationale: "Planner forgot the query.".to_string(),
            ..Default::default()
        };

        let entry = reflect_plan_cache_entry_from_plan(&plan, "2026-06-10T00:00:00Z", "a topic")
            .expect("unusable useful verdict should cache as a negative entry");
        assert!(!entry.useful);
        assert_eq!(entry.topic, "a topic");
    }

    #[test]
    fn negative_plan_verdicts_expire_faster_than_useful_ones() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 6, 10, 0, 0, 0)
            .single()
            .unwrap();
        let planned_at = (now - chrono::Duration::days(30)).to_rfc3339();
        let mut cache = ReflectFollowupPlanCache::default();
        cache.entries.insert(
            "useful".to_string(),
            ReflectFollowupPlanEntry {
                id: "useful".to_string(),
                useful: true,
                title: "Track a useful pursuit".to_string(),
                search_query: "useful pursuit current sources".to_string(),
                planned_at: planned_at.clone(),
                ..Default::default()
            },
        );
        cache.entries.insert(
            "not-useful".to_string(),
            ReflectFollowupPlanEntry {
                id: "not-useful".to_string(),
                useful: false,
                planned_at,
                ..Default::default()
            },
        );

        prune_reflect_followup_plan_cache(&mut cache, now);

        assert!(cache.entries.contains_key("useful"));
        assert!(!cache.entries.contains_key("not-useful"));
    }

    #[test]
    fn due_followup_work_routes_travel_strategy_with_travel_query() {
        let mut candidate = followup("travel", "latest_developments", 90.0);
        candidate.search_query =
            Some("Kolkata to Thailand flexible dates cheap flights 2026".to_string());
        candidate.source_strategy = ReflectFollowupSourceStrategy::FlightPriceDiscovery;
        candidate.structured_context = serde_json::json!({
            "origin": "Kolkata, India",
            "destination": "Thailand",
            "trip_window": "next 365 days",
            "currency": "INR"
        });

        let work = reflect_due_followup_work(
            &[candidate],
            &ReflectFollowupSearchCache::default(),
            chrono::Utc
                .with_ymd_and_hms(2026, 6, 3, 12, 0, 0)
                .single()
                .unwrap(),
        );

        assert_eq!(work.searches.len(), 1);
        assert_eq!(work.searches[0].source_id, "travel");
        assert_eq!(
            work.searches[0].source_strategy,
            ReflectFollowupSourceStrategy::FlightPriceDiscovery
        );
        assert!(work.searches[0].query.contains("Kolkata"));
        assert!(work.searches[0].query.contains("Thailand"));
    }

    #[test]
    fn due_followup_work_keeps_public_search_strategy_default() {
        let mut candidate = followup("generic", "latest_developments", 90.0);
        candidate.search_query = Some("current Kolkata civic updates".to_string());

        let work = reflect_due_followup_work(
            &[candidate],
            &ReflectFollowupSearchCache::default(),
            chrono::Utc
                .with_ymd_and_hms(2026, 6, 3, 12, 0, 0)
                .single()
                .unwrap(),
        );

        assert_eq!(work.searches.len(), 1);
        assert_eq!(
            work.searches[0].source_strategy,
            ReflectFollowupSourceStrategy::PublicSearch
        );
        assert_eq!(work.searches[0].query, "current Kolkata civic updates");
    }

    #[test]
    fn due_followup_work_does_not_search_unplanned_latest_candidates() {
        let mut candidate = followup("unplanned", "latest_developments", 90.0);
        candidate.search_requires_planning = true;
        candidate.search_query = Some("raw reflected context should not be searched".to_string());

        let work = reflect_due_followup_work(
            &[candidate],
            &ReflectFollowupSearchCache::default(),
            chrono::Utc
                .with_ymd_and_hms(2026, 6, 3, 12, 0, 0)
                .single()
                .unwrap(),
        );

        assert!(work.searches.is_empty());
    }

    #[test]
    fn due_followup_work_ignores_unsafe_queries() {
        let mut unsafe_candidate = followup("unsafe", "latest_developments", 90.0);
        unsafe_candidate.search_query =
            Some("refresh_token abc1234567890123456789012345678901234567890".to_string());

        let mut safe_candidate = followup("safe", "latest_developments", 89.0);
        safe_candidate.search_query = Some("current public exam counselling dates".to_string());

        let work = reflect_due_followup_work(
            &[unsafe_candidate, safe_candidate],
            &ReflectFollowupSearchCache::default(),
            chrono::Utc
                .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
                .single()
                .unwrap(),
        );

        assert_eq!(work.searches.len(), 1);
        assert_eq!(work.searches[0].source_id, "safe");
    }

    #[test]
    fn due_followup_work_treats_current_year_normalized_cache_as_fresh() {
        let now = chrono::Utc::now();
        let mut candidate = followup("topic", "latest_developments", 90.0);
        candidate.search_query = Some(format!("Iran India news latest {}", now.year()));
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            "topic".to_string(),
            ReflectFollowupSearchEntry {
                source_id: "topic".to_string(),
                query: "Iran India news latest".to_string(),
                checked_at: now.to_rfc3339(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "India-Iran source".to_string(),
                    url: "https://example.com/india-iran".to_string(),
                    snippet: "Cached source result.".to_string(),
                    source: "Example".to_string(),
                    published_date: Some(now.date_naive().to_string()),
                }],
                error: None,
                ..Default::default()
            },
        );

        let work = reflect_due_followup_work(&[candidate], &cache, now);

        assert!(work.searches.is_empty());
    }

    #[test]
    fn cache_summary_update_preserves_newer_search_entry() {
        let mut cache = ReflectFollowupSearchCache::default();
        cache.entries.insert(
            "topic".to_string(),
            ReflectFollowupSearchEntry {
                source_id: "topic".to_string(),
                query: "new query".to_string(),
                checked_at: "2026-05-06T12:00:00Z".to_string(),
                backend: Some("test".to_string()),
                results: vec![ReflectFollowupSearchResult {
                    title: "New result".to_string(),
                    url: "https://example.com/new".to_string(),
                    snippet: "Newer source result.".to_string(),
                    source: "Example".to_string(),
                    published_date: None,
                }],
                error: None,
                summary: None,
                summary_generated_at: None,
                summary_error: None,
                ..Default::default()
            },
        );

        let applied = reflect_cache_apply_summary_if_current(
            &mut cache,
            "topic",
            "2026-05-06T11:00:00Z",
            Some("Old summary".to_string()),
            None,
            Some(true),
            "2026-05-06T12:05:00Z".to_string(),
        );

        assert!(!applied);
        let current = cache.entries.get("topic").unwrap();
        assert_eq!(current.checked_at, "2026-05-06T12:00:00Z");
        assert!(current.summary.is_none());
    }

    #[test]
    fn refresh_recent_completion_suppresses_same_range_only_temporarily() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 6, 12, 0, 0)
            .single()
            .unwrap();
        let request = ReflectRefreshRequest {
            period: ReflectPeriod::Weekly,
            from: now - chrono::Duration::days(7),
            to: now,
        };
        let mut status = ReflectRefreshStatus {
            status: "completed".to_string(),
            period: Some(request.period.as_str().to_string()),
            from: Some(request.from.to_rfc3339()),
            to: Some(request.to.to_rfc3339()),
            completed_at: Some((now - chrono::Duration::minutes(3)).to_rfc3339()),
            ..ReflectRefreshStatus::default()
        };

        assert!(reflect_refresh_recently_completed_for_request(
            &status, &request, now
        ));

        status.completed_at = Some((now - chrono::Duration::minutes(20)).to_rfc3339());
        assert!(!reflect_refresh_recently_completed_for_request(
            &status, &request, now
        ));
    }
}
