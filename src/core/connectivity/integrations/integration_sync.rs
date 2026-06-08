use crate::core::Agent;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

const INTEGRATION_SYNC_STATE_KEY: &str = "integration_sync_state_v1";
const INTEGRATION_SYNC_FEED_KEY: &str = "integration_sync_feed_v1";
const INTEGRATION_SYNC_RUNS_KEY: &str = "integration_sync_runs_v1";
const MAX_FEED_ITEMS: usize = 300;
const MAX_RUN_ITEMS: usize = 400;
const MAX_RECENT_SOURCE_IDS: usize = 400;
const MAX_BASELINE_ITEMS: usize = 8;
const GITHUB_SYNC_REPO_LIMIT: usize = 25;
const GITHUB_SYNC_EVENT_LIMIT: usize = 30;
const GITHUB_SYNC_ALERT_REPO_LIMIT: usize = 8;
const GITHUB_SYNC_ALERT_LIMIT: usize = 5;
const INTEGRATION_SYNC_STATUS_TIMEOUT_SECS: u64 = 8;
const INTEGRATION_SYNC_FETCH_TIMEOUT_SECS: u64 = 45;
const INTEGRATION_SYNC_HTTP_MAX_RETRIES: usize = 3;
const INTEGRATION_SYNC_HTTP_INITIAL_RETRY_MS: u64 = 750;
const INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS: u64 = 30;
const INTEGRATION_SYNC_BUSY_MESSAGE: &str =
    "Another integration sync run is already in progress. Try again in a moment.";
const DEFAULT_INTEGRATION_SYNC_POLL_SECS: u64 = 30 * 60;
static INTEGRATION_SYNC_LOCKS: Lazy<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static INTEGRATION_SYNC_STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Clone)]
pub struct IntegrationSyncContext {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub encrypted_storage: crate::storage::encrypted::EncryptedStorage,
    pub shared_agent: Option<Arc<RwLock<Agent>>>,
}

pub fn context_from_agent(
    agent: &Agent,
    shared_agent: Option<Arc<RwLock<Agent>>>,
) -> IntegrationSyncContext {
    IntegrationSyncContext {
        config_dir: agent.config_dir.clone(),
        data_dir: agent.data_dir.clone(),
        encrypted_storage: agent.encrypted_storage.clone(),
        shared_agent,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationSyncConfig {
    pub integration_id: String,
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub importance_threshold: f32,
    pub notify_on_important: bool,
    pub push_to_preferred_channel: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrationSyncCursor {
    pub last_sync_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub last_item_at: Option<String>,
    pub seeded_at: Option<String>,
    #[serde(default)]
    pub recent_source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationSyncFeedItem {
    pub id: String,
    pub integration_id: String,
    pub integration_name: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub url: Option<String>,
    pub occurred_at: Option<String>,
    pub detected_at: String,
    pub importance: f32,
    pub important: bool,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationSyncRunItem {
    pub id: String,
    pub integration_id: String,
    pub integration_name: String,
    pub sync_kind: String,
    pub trigger: String,
    pub status: String,
    pub summary: String,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub fetched_item_count: usize,
    pub new_item_count: usize,
    pub recorded_item_count: usize,
    pub important_item_count: usize,
    pub baseline_mode: bool,
    pub connected: bool,
    pub integration_enabled: bool,
    pub last_item_at: Option<String>,
    #[serde(default)]
    pub sample_titles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationSyncRunBucket {
    pub label: String,
    pub runs: usize,
    pub failures: usize,
    pub blocked: usize,
    pub important_hits: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationSyncRunStats {
    pub total_runs: usize,
    pub completed_runs: usize,
    pub failed_runs: usize,
    pub blocked_runs: usize,
    pub important_hits: usize,
    pub avg_duration_ms: Option<u64>,
    pub buckets: Vec<IntegrationSyncRunBucket>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationSyncRunsPage {
    pub items: Vec<IntegrationSyncRunItem>,
    pub total: usize,
    pub stats: IntegrationSyncRunStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationSyncStatusView {
    pub integration_id: String,
    pub integration_name: String,
    pub supported: bool,
    pub enabled: bool,
    pub connected: bool,
    pub integration_enabled: bool,
    pub sync_kind: String,
    pub poll_interval_secs: u64,
    pub importance_threshold: f32,
    pub notify_on_important: bool,
    pub push_to_preferred_channel: bool,
    pub last_sync_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub last_item_at: Option<String>,
    pub recent_item_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntegrationSyncUpdateRequest {
    pub enabled: Option<bool>,
    pub poll_interval_secs: Option<u64>,
    pub importance_threshold: Option<f32>,
    pub notify_on_important: Option<bool>,
    pub push_to_preferred_channel: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IntegrationSyncStateStore {
    #[serde(default)]
    configs: HashMap<String, IntegrationSyncConfig>,
    #[serde(default)]
    cursors: HashMap<String, IntegrationSyncCursor>,
}

#[derive(Debug, Clone)]
struct NormalizedSyncItem {
    source_id: String,
    kind: String,
    title: String,
    summary: String,
    url: Option<String>,
    occurred_at: Option<DateTime<Utc>>,
    importance: f32,
}

#[derive(Debug, Clone, Default)]
struct ApplyItemsResult {
    fetched_item_count: usize,
    new_item_count: usize,
    recorded_item_count: usize,
    important_item_count: usize,
    sample_titles: Vec<String>,
}

#[derive(Debug, Clone)]
struct SyncRunOutcome {
    status: &'static str,
    error: Option<String>,
    fetched_item_count: usize,
    new_item_count: usize,
    recorded_item_count: usize,
    important_item_count: usize,
    baseline_mode: bool,
    sample_titles: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct IntegrationSyncAdapter {
    sync_kind: &'static str,
    default_poll_interval_secs: u64,
    default_importance_threshold: f32,
}

fn adapter_for(integration_id: &str) -> Option<IntegrationSyncAdapter> {
    match integration_id {
        "google_workspace" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.72,
        }),
        "gmail" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.72,
        }),
        "google_calendar" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.68,
        }),
        "github" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.74,
        }),
        "notion" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.66,
        }),
        "twitter" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.7,
        }),
        "onepassword" => Some(IntegrationSyncAdapter {
            sync_kind: "inventory",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.8,
        }),
        "twilio" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.72,
        }),
        "ordering" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.7,
        }),
        "garmin" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.76,
        }),
        "whoop" => Some(IntegrationSyncAdapter {
            sync_kind: "activity",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.76,
        }),
        "ga4" => Some(IntegrationSyncAdapter {
            sync_kind: "analytics",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.7,
        }),
        "gsc" => Some(IntegrationSyncAdapter {
            sync_kind: "analytics",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.7,
        }),
        "social_analytics" => Some(IntegrationSyncAdapter {
            sync_kind: "analytics",
            default_poll_interval_secs: DEFAULT_INTEGRATION_SYNC_POLL_SECS,
            default_importance_threshold: 0.72,
        }),
        _ => None,
    }
}

fn default_config_for(integration_id: &str) -> Option<IntegrationSyncConfig> {
    let adapter = adapter_for(integration_id)?;
    Some(IntegrationSyncConfig {
        integration_id: integration_id.to_string(),
        enabled: true,
        poll_interval_secs: adapter.default_poll_interval_secs,
        importance_threshold: adapter.default_importance_threshold,
        notify_on_important: true,
        push_to_preferred_channel: false,
    })
}

fn integration_manager(ctx: &IntegrationSyncContext) -> crate::integrations::IntegrationManager {
    crate::integrations::IntegrationManager::new(&ctx.config_dir)
}

fn integration_sync_lock_key(integration_id: &str) -> String {
    integration_id.trim().to_ascii_lowercase()
}

async fn try_acquire_integration_sync_lock(integration_id: &str) -> Option<OwnedMutexGuard<()>> {
    let key = integration_sync_lock_key(integration_id);
    let lock = {
        let mut locks = INTEGRATION_SYNC_LOCKS.lock().await;
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    lock.try_lock_owned().ok()
}

async fn load_state_store(ctx: &IntegrationSyncContext) -> IntegrationSyncStateStore {
    match ctx
        .encrypted_storage
        .get_decrypted(INTEGRATION_SYNC_STATE_KEY)
        .await
    {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => IntegrationSyncStateStore::default(),
    }
}

async fn save_state_store(
    ctx: &IntegrationSyncContext,
    store: &IntegrationSyncStateStore,
) -> Result<()> {
    let bytes = serde_json::to_vec(store)?;
    ctx.encrypted_storage
        .set_encrypted(INTEGRATION_SYNC_STATE_KEY, &bytes)
        .await
}

pub async fn ensure_default_enabled(
    ctx: &IntegrationSyncContext,
    integration_id: &str,
) -> Result<()> {
    let Some(config) = default_config_for(integration_id) else {
        return Ok(());
    };
    let _store_guard = INTEGRATION_SYNC_STORE_LOCK.lock().await;
    let mut store = load_state_store(ctx).await;
    if store.configs.contains_key(integration_id) {
        return Ok(());
    }
    store.configs.insert(integration_id.to_string(), config);
    save_state_store(ctx, &store).await
}

async fn load_feed(ctx: &IntegrationSyncContext) -> Vec<IntegrationSyncFeedItem> {
    match ctx
        .encrypted_storage
        .get_decrypted(INTEGRATION_SYNC_FEED_KEY)
        .await
    {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn save_feed(ctx: &IntegrationSyncContext, items: &[IntegrationSyncFeedItem]) -> Result<()> {
    let bytes = serde_json::to_vec(items)?;
    ctx.encrypted_storage
        .set_encrypted(INTEGRATION_SYNC_FEED_KEY, &bytes)
        .await
}

async fn load_runs(ctx: &IntegrationSyncContext) -> Vec<IntegrationSyncRunItem> {
    match ctx
        .encrypted_storage
        .get_decrypted(INTEGRATION_SYNC_RUNS_KEY)
        .await
    {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn save_runs(ctx: &IntegrationSyncContext, items: &[IntegrationSyncRunItem]) -> Result<()> {
    let bytes = serde_json::to_vec(items)?;
    ctx.encrypted_storage
        .set_encrypted(INTEGRATION_SYNC_RUNS_KEY, &bytes)
        .await
}

fn integration_name_from_id(integration_id: &str) -> String {
    match integration_id {
        "google_workspace" => "Google Workspace".to_string(),
        "gmail" => "Gmail".to_string(),
        "google_calendar" => "Google Calendar".to_string(),
        "github" => "GitHub".to_string(),
        "notion" => "Notion".to_string(),
        "twitter" => "Twitter / X".to_string(),
        "onepassword" => "1Password".to_string(),
        "twilio" => "Twilio".to_string(),
        "ordering" => "Ordering".to_string(),
        "garmin" => "Garmin".to_string(),
        "whoop" => "WHOOP".to_string(),
        "ga4" => "Google Analytics 4".to_string(),
        "gsc" => "Google Search Console".to_string(),
        "social_analytics" => "Social Analytics".to_string(),
        other => other.to_string(),
    }
}

fn short_text(input: &str, max_len: usize) -> String {
    let trimmed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.chars().count() <= max_len {
        trimmed
    } else {
        format!(
            "{}...",
            trimmed
                .chars()
                .take(max_len.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn hash_id(parts: &[&str]) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

fn parse_datetime(input: &str) -> Option<DateTime<Utc>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(trimmed)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc2822(trimmed)
                .map(|dt| dt.with_timezone(&Utc))
                .ok()
        })
        .or_else(|| {
            NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date_time| Utc.from_utc_datetime(&date_time))
        })
        .or_else(|| {
            NaiveDate::parse_from_str(trimmed, "%Y%m%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date_time| Utc.from_utc_datetime(&date_time))
        })
}

fn text_importance_boost(text: &str) -> f32 {
    let _ = text;
    0.0
}

fn recency_importance_boost(at: Option<DateTime<Utc>>) -> f32 {
    let Some(at) = at else {
        return 0.0;
    };
    let age = Utc::now() - at;
    if age <= Duration::hours(2) {
        0.2
    } else if age <= Duration::hours(8) {
        0.14
    } else if age <= Duration::hours(24) {
        0.08
    } else {
        0.0
    }
}

fn clamp_importance(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn github_api_url(path_segments: &[&str], query: &[(&str, String)]) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse("https://api.github.com")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow!("Failed to build GitHub API URL"))?;
        for segment in path_segments {
            segments.push(segment);
        }
    }
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url)
}

fn github_authed_get(
    client: &reqwest::Client,
    token: &str,
    url: reqwest::Url,
) -> reqwest::RequestBuilder {
    client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", crate::branding::versioned_user_agent())
        .header("Accept", "application/vnd.github+json")
}

fn integration_sync_retry_after(headers: &reqwest::header::HeaderMap) -> Option<StdDuration> {
    let raw = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())?
        .trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(StdDuration::from_secs(
            seconds.min(INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS),
        ));
    }
    let parsed = chrono::DateTime::parse_from_rfc2822(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })?;
    let delay = (parsed - Utc::now()).to_std().ok()?;
    Some(delay.min(StdDuration::from_secs(INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS)))
}

fn integration_sync_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

async fn send_integration_sync_request(
    label: &str,
    mut request: reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let mut attempt = 0usize;
    let mut fallback_delay = StdDuration::from_millis(INTEGRATION_SYNC_HTTP_INITIAL_RETRY_MS);
    loop {
        let retry_request = if attempt < INTEGRATION_SYNC_HTTP_MAX_RETRIES {
            request.try_clone()
        } else {
            None
        };
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if integration_sync_retryable_status(status) {
                    if let Some(next_request) = retry_request {
                        let wait = integration_sync_retry_after(response.headers())
                            .unwrap_or(fallback_delay)
                            .min(StdDuration::from_secs(INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS));
                        attempt += 1;
                        tracing::warn!(
                            label,
                            status = status.as_u16(),
                            attempt,
                            retry_after_ms = wait.as_millis(),
                            "Integration sync HTTP request returned retryable status"
                        );
                        tokio::time::sleep(wait).await;
                        request = next_request;
                        fallback_delay = fallback_delay
                            .saturating_mul(2)
                            .min(StdDuration::from_secs(INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS));
                        continue;
                    }
                }
                return Ok(response);
            }
            Err(error) => {
                if error.is_timeout() || error.is_connect() {
                    if let Some(next_request) = retry_request {
                        let wait = fallback_delay
                            .min(StdDuration::from_secs(INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS));
                        attempt += 1;
                        tracing::warn!(
                            label,
                            error = %error,
                            attempt,
                            retry_after_ms = wait.as_millis(),
                            "Integration sync HTTP request failed transiently"
                        );
                        tokio::time::sleep(wait).await;
                        request = next_request;
                        fallback_delay = fallback_delay
                            .saturating_mul(2)
                            .min(StdDuration::from_secs(INTEGRATION_SYNC_HTTP_MAX_RETRY_SECS));
                        continue;
                    }
                }
                return Err(error.into());
            }
        }
    }
}

async fn github_get_json_value(
    client: &reqwest::Client,
    token: &str,
    url: reqwest::Url,
    label: &str,
) -> Result<serde_json::Value> {
    let response =
        send_integration_sync_request(label, github_authed_get(client, token, url)).await?;
    let status = response.status();
    if !status.is_success() {
        let error = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "GitHub {} failed: {} {}",
            label,
            status,
            short_text(&error, 180)
        ));
    }
    Ok(response.json::<serde_json::Value>().await?)
}

async fn github_get_json_array(
    client: &reqwest::Client,
    token: &str,
    url: reqwest::Url,
    label: &str,
) -> Result<Vec<serde_json::Value>> {
    let value = github_get_json_value(client, token, url, label).await?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("GitHub {} returned a non-array payload", label))
}

fn github_after_last_success(
    cursor: &IntegrationSyncCursor,
    occurred_at: Option<DateTime<Utc>>,
) -> bool {
    let Some(since) = cursor.last_success_at.as_deref().and_then(parse_datetime) else {
        return true;
    };
    occurred_at.map(|value| value > since).unwrap_or(true)
}

fn github_repo_url(full_name: &str) -> Option<String> {
    if full_name.trim().is_empty() || full_name == "repository" {
        None
    } else {
        Some(format!("https://github.com/{}", full_name))
    }
}

async fn gmail_connected(ctx: &IntegrationSyncContext) -> bool {
    let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &ctx.config_dir,
        Some(&ctx.data_dir),
    ) else {
        return false;
    };
    manager
        .get_custom_secret("gmail_tokens")
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|v| !v.trim().is_empty())
        })
        .unwrap_or(false)
}

async fn calendar_connected(ctx: &IntegrationSyncContext) -> bool {
    let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &ctx.config_dir,
        Some(&ctx.data_dir),
    ) else {
        return false;
    };
    manager
        .get_custom_secret("calendar_tokens")
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|v| !v.trim().is_empty())
        })
        .unwrap_or(false)
        || crate::actions::google_workspace::granted_bundles(&ctx.config_dir)
            .map(|bundles| bundles.iter().any(|bundle| bundle == "calendar"))
            .unwrap_or(false)
}

async fn google_workspace_connected(ctx: &IntegrationSyncContext) -> bool {
    crate::actions::google_workspace::granted_bundles(&ctx.config_dir)
        .map(|bundles| !bundles.is_empty())
        .unwrap_or(false)
}

fn integration_uses_config_only_status(integration_id: &str) -> bool {
    matches!(
        integration_id,
        "twitter"
            | "google_places"
            | "twilio"
            | "ordering"
            | "garmin"
            | "whoop"
            | "ga4"
            | "gsc"
            | "social_analytics"
            | "moltbook"
    )
}

async fn integration_connected(
    ctx: &IntegrationSyncContext,
    manager: &crate::integrations::IntegrationManager,
    integration_id: &str,
) -> bool {
    let started = Instant::now();
    tracing::debug!(
        integration_id = integration_id,
        "Integration sync connection check started"
    );
    if integration_id == "gmail" {
        let connected = gmail_connected(ctx).await;
        tracing::debug!(
            integration_id = integration_id,
            connected = connected,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync connection check completed"
        );
        return connected;
    }
    if integration_id == "google_calendar" {
        let connected = calendar_connected(ctx).await;
        tracing::debug!(
            integration_id = integration_id,
            connected = connected,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync connection check completed"
        );
        return connected;
    }
    if integration_id == "google_workspace" {
        let connected = google_workspace_connected(ctx).await;
        tracing::debug!(
            integration_id = integration_id,
            connected = connected,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync connection check completed"
        );
        return connected;
    }
    if integration_uses_config_only_status(integration_id) {
        tracing::debug!(
            integration_id = integration_id,
            connected = false,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync connection check skipped for config-only integration"
        );
        return false;
    }
    let Some(integration) = manager.get(integration_id) else {
        tracing::debug!(
            integration_id = integration_id,
            connected = false,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync connection check skipped because integration is not registered"
        );
        return false;
    };
    match tokio::time::timeout(
        StdDuration::from_secs(INTEGRATION_SYNC_STATUS_TIMEOUT_SECS),
        integration.status(),
    )
    .await
    {
        Ok(status) => {
            let connected = matches!(status, crate::integrations::IntegrationStatus::Connected);
            tracing::debug!(
                integration_id = integration_id,
                connected = connected,
                status = ?status,
                duration_ms = started.elapsed().as_millis() as u64,
                "Integration sync connection check completed"
            );
            connected
        }
        Err(_) => {
            tracing::warn!(
                integration_id = integration_id,
                "Integration sync status check timed out for '{}' after {}s; skipping background sync for this integration",
                integration_id,
                INTEGRATION_SYNC_STATUS_TIMEOUT_SECS
            );
            false
        }
    }
}

async fn integration_dispatch_enabled(
    ctx: &IntegrationSyncContext,
    manager: &crate::integrations::IntegrationManager,
    integration_id: &str,
) -> bool {
    if integration_id != "gmail" && integration_id != "google_workspace" {
        return manager.is_enabled(integration_id);
    }

    let Ok(manager) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &ctx.config_dir,
        Some(&ctx.data_dir),
    ) else {
        return true;
    };

    manager
        .get_custom_secret(&format!("integration_enabled:{}", integration_id))
        .ok()
        .flatten()
        .map(|value| value.trim().to_ascii_lowercase())
        .and_then(|value| match value.as_str() {
            "1" | "true" | "yes" | "y" | "on" => Some(true),
            "0" | "false" | "no" | "n" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(true)
}

fn next_due(
    cursor: &IntegrationSyncCursor,
    config: &IntegrationSyncConfig,
) -> Option<DateTime<Utc>> {
    let last_sync = cursor.last_sync_at.as_deref().and_then(parse_datetime)?;
    Some(last_sync + Duration::seconds(config.poll_interval_secs as i64))
}

fn is_due(cursor: &IntegrationSyncCursor, config: &IntegrationSyncConfig) -> bool {
    match next_due(cursor, config) {
        Some(due_at) => Utc::now() >= due_at,
        None => true,
    }
}

fn push_recent_source_ids(cursor: &mut IntegrationSyncCursor, source_ids: &[String]) {
    let mut merged = Vec::with_capacity(cursor.recent_source_ids.len() + source_ids.len());
    let mut seen = HashSet::new();
    for source_id in source_ids {
        if seen.insert(source_id.clone()) {
            merged.push(source_id.clone());
        }
    }
    for source_id in &cursor.recent_source_ids {
        if seen.insert(source_id.clone()) {
            merged.push(source_id.clone());
        }
    }
    merged.truncate(MAX_RECENT_SOURCE_IDS);
    cursor.recent_source_ids = merged;
}

fn feed_items_for_integration(feed: &[IntegrationSyncFeedItem], integration_id: &str) -> usize {
    feed.iter()
        .filter(|item| item.integration_id == integration_id)
        .count()
}

fn parse_or_epoch(value: &str) -> DateTime<Utc> {
    parse_datetime(value)
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_else(Utc::now))
}

fn summarize_run_stats(items: &[IntegrationSyncRunItem]) -> IntegrationSyncRunStats {
    let mut total_duration_ms: u128 = 0;
    let mut duration_count: usize = 0;
    let today = Utc::now().date_naive();
    let mut buckets = (0..7)
        .rev()
        .map(|offset| {
            let day = today - chrono::Days::new(offset as u64);
            IntegrationSyncRunBucket {
                label: day.format("%d %b").to_string(),
                runs: 0,
                failures: 0,
                blocked: 0,
                important_hits: 0,
            }
        })
        .collect::<Vec<_>>();

    let bucket_index_by_day = buckets
        .iter()
        .enumerate()
        .map(|(idx, bucket)| (bucket.label.clone(), idx))
        .collect::<HashMap<_, _>>();

    let mut completed_runs = 0usize;
    let mut failed_runs = 0usize;
    let mut blocked_runs = 0usize;
    let mut important_hits = 0usize;

    for item in items {
        match item.status.as_str() {
            "completed" => completed_runs += 1,
            "failed" => failed_runs += 1,
            "blocked" => blocked_runs += 1,
            _ => {}
        }
        important_hits += item.important_item_count;
        total_duration_ms += item.duration_ms as u128;
        duration_count += 1;

        let started = parse_or_epoch(&item.started_at);
        let label = started.date_naive().format("%d %b").to_string();
        if let Some(bucket_idx) = bucket_index_by_day.get(&label).copied() {
            let bucket = &mut buckets[bucket_idx];
            bucket.runs += 1;
            bucket.important_hits += item.important_item_count;
            match item.status.as_str() {
                "failed" => bucket.failures += 1,
                "blocked" => bucket.blocked += 1,
                _ => {}
            }
        }
    }

    IntegrationSyncRunStats {
        total_runs: items.len(),
        completed_runs,
        failed_runs,
        blocked_runs,
        important_hits,
        avg_duration_ms: if duration_count > 0 {
            Some((total_duration_ms / duration_count as u128) as u64)
        } else {
            None
        },
        buckets,
    }
}

pub async fn list_statuses(ctx: &IntegrationSyncContext) -> Vec<IntegrationSyncStatusView> {
    let store = load_state_store(ctx).await;
    let feed = load_feed(ctx).await;
    let manager = integration_manager(ctx);
    let mut integrations = manager.list().await;
    integrations.sort_by(|left, right| left.name.cmp(&right.name));

    let mut rows = Vec::new();
    for info in integrations {
        if info.id == "moltbook" {
            continue;
        }
        let adapter = adapter_for(&info.id);
        let default = default_config_for(&info.id);
        let config = default.as_ref().and_then(|cfg| {
            store
                .configs
                .get(&info.id)
                .cloned()
                .or_else(|| Some(cfg.clone()))
        });
        let cursor = store.cursors.get(&info.id).cloned().unwrap_or_default();
        rows.push(IntegrationSyncStatusView {
            integration_id: info.id.clone(),
            integration_name: info.name.clone(),
            supported: adapter.is_some(),
            connected: integration_connected(ctx, &manager, &info.id).await,
            integration_enabled: integration_dispatch_enabled(ctx, &manager, &info.id).await,
            sync_kind: adapter
                .map(|item| item.sync_kind)
                .unwrap_or("unsupported")
                .to_string(),
            enabled: config.as_ref().map(|cfg| cfg.enabled).unwrap_or(false),
            poll_interval_secs: config
                .as_ref()
                .map(|cfg| cfg.poll_interval_secs)
                .unwrap_or_default(),
            importance_threshold: config
                .as_ref()
                .map(|cfg| cfg.importance_threshold)
                .unwrap_or_default(),
            notify_on_important: config
                .as_ref()
                .map(|cfg| cfg.notify_on_important)
                .unwrap_or(false),
            push_to_preferred_channel: config
                .as_ref()
                .map(|cfg| cfg.push_to_preferred_channel)
                .unwrap_or(false),
            last_sync_at: cursor.last_sync_at.clone(),
            last_success_at: cursor.last_success_at.clone(),
            last_error: cursor.last_error.clone(),
            last_item_at: cursor.last_item_at.clone(),
            recent_item_count: feed_items_for_integration(&feed, &info.id),
        });
    }

    let workspace_adapter = adapter_for("google_workspace");
    let workspace_default = default_config_for("google_workspace");
    let workspace_config = workspace_default.as_ref().and_then(|cfg| {
        store
            .configs
            .get("google_workspace")
            .cloned()
            .or_else(|| Some(cfg.clone()))
    });
    let workspace_cursor = store
        .cursors
        .get("google_workspace")
        .cloned()
        .unwrap_or_default();
    rows.push(IntegrationSyncStatusView {
        integration_id: "google_workspace".to_string(),
        integration_name: "Google Workspace".to_string(),
        supported: workspace_adapter.is_some(),
        connected: google_workspace_connected(ctx).await,
        integration_enabled: integration_dispatch_enabled(ctx, &manager, "google_workspace").await,
        sync_kind: workspace_adapter
            .map(|item| item.sync_kind)
            .unwrap_or("unsupported")
            .to_string(),
        enabled: workspace_config
            .as_ref()
            .map(|cfg| cfg.enabled)
            .unwrap_or(false),
        poll_interval_secs: workspace_config
            .as_ref()
            .map(|cfg| cfg.poll_interval_secs)
            .unwrap_or_default(),
        importance_threshold: workspace_config
            .as_ref()
            .map(|cfg| cfg.importance_threshold)
            .unwrap_or_default(),
        notify_on_important: workspace_config
            .as_ref()
            .map(|cfg| cfg.notify_on_important)
            .unwrap_or(false),
        push_to_preferred_channel: workspace_config
            .as_ref()
            .map(|cfg| cfg.push_to_preferred_channel)
            .unwrap_or(false),
        last_sync_at: workspace_cursor.last_sync_at.clone(),
        last_success_at: workspace_cursor.last_success_at.clone(),
        last_error: workspace_cursor.last_error.clone(),
        last_item_at: workspace_cursor.last_item_at.clone(),
        recent_item_count: feed_items_for_integration(&feed, "google_workspace"),
    });

    let gmail_adapter = adapter_for("gmail");
    let gmail_default = default_config_for("gmail");
    let gmail_config = gmail_default.as_ref().and_then(|cfg| {
        store
            .configs
            .get("gmail")
            .cloned()
            .or_else(|| Some(cfg.clone()))
    });
    let gmail_cursor = store.cursors.get("gmail").cloned().unwrap_or_default();
    rows.push(IntegrationSyncStatusView {
        integration_id: "gmail".to_string(),
        integration_name: "Gmail".to_string(),
        supported: gmail_adapter.is_some(),
        connected: gmail_connected(ctx).await,
        integration_enabled: integration_dispatch_enabled(ctx, &manager, "gmail").await,
        sync_kind: gmail_adapter
            .map(|item| item.sync_kind)
            .unwrap_or("unsupported")
            .to_string(),
        enabled: gmail_config
            .as_ref()
            .map(|cfg| cfg.enabled)
            .unwrap_or(false),
        poll_interval_secs: gmail_config
            .as_ref()
            .map(|cfg| cfg.poll_interval_secs)
            .unwrap_or_default(),
        importance_threshold: gmail_config
            .as_ref()
            .map(|cfg| cfg.importance_threshold)
            .unwrap_or_default(),
        notify_on_important: gmail_config
            .as_ref()
            .map(|cfg| cfg.notify_on_important)
            .unwrap_or(false),
        push_to_preferred_channel: gmail_config
            .as_ref()
            .map(|cfg| cfg.push_to_preferred_channel)
            .unwrap_or(false),
        last_sync_at: gmail_cursor.last_sync_at.clone(),
        last_success_at: gmail_cursor.last_success_at.clone(),
        last_error: gmail_cursor.last_error.clone(),
        last_item_at: gmail_cursor.last_item_at.clone(),
        recent_item_count: feed_items_for_integration(&feed, "gmail"),
    });

    rows.sort_by(|left, right| left.integration_name.cmp(&right.integration_name));
    rows
}

pub async fn list_feed_items(
    ctx: &IntegrationSyncContext,
    integration_id: Option<&str>,
    limit: usize,
) -> Vec<IntegrationSyncFeedItem> {
    let mut feed = load_feed(ctx).await;
    if let Some(integration_id) = integration_id {
        feed.retain(|item| item.integration_id == integration_id);
    }
    feed.sort_by(|left, right| right.detected_at.cmp(&left.detected_at));
    feed.truncate(limit.clamp(1, MAX_FEED_ITEMS));
    feed
}

pub async fn list_runs(
    ctx: &IntegrationSyncContext,
    integration_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> IntegrationSyncRunsPage {
    let mut runs = load_runs(ctx).await;
    if let Some(integration_id) = integration_id {
        runs.retain(|item| item.integration_id == integration_id);
    }
    runs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    let total = runs.len();
    let stats = summarize_run_stats(&runs);
    let items = runs
        .into_iter()
        .skip(offset)
        .take(limit.clamp(1, MAX_RUN_ITEMS))
        .collect();
    IntegrationSyncRunsPage {
        items,
        total,
        stats,
    }
}

fn merge_feed_items(
    existing: &mut Vec<IntegrationSyncFeedItem>,
    additions: Vec<IntegrationSyncFeedItem>,
) {
    let mut seen = existing
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    for item in additions {
        if seen.insert(item.id.clone()) {
            existing.push(item);
        }
    }
    existing.sort_by(|left, right| right.detected_at.cmp(&left.detected_at));
    existing.truncate(MAX_FEED_ITEMS);
}

fn merge_run_items(
    existing: &mut Vec<IntegrationSyncRunItem>,
    additions: Vec<IntegrationSyncRunItem>,
) {
    let mut seen = existing
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    for item in additions {
        if seen.insert(item.id.clone()) {
            existing.push(item);
        }
    }
    existing.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    existing.truncate(MAX_RUN_ITEMS);
}

pub async fn update_config(
    ctx: &IntegrationSyncContext,
    integration_id: &str,
    request: IntegrationSyncUpdateRequest,
) -> Result<IntegrationSyncStatusView> {
    let Some(_integration_guard) = try_acquire_integration_sync_lock(integration_id).await else {
        return Err(anyhow!(INTEGRATION_SYNC_BUSY_MESSAGE));
    };
    let Some(default_config) = default_config_for(integration_id) else {
        return Err(anyhow!(
            "Background sync is not available for '{}'",
            integration_id
        ));
    };

    {
        let _store_guard = INTEGRATION_SYNC_STORE_LOCK.lock().await;
        let mut store = load_state_store(ctx).await;
        let mut config = store
            .configs
            .get(integration_id)
            .cloned()
            .unwrap_or(default_config);

        if let Some(enabled) = request.enabled {
            config.enabled = enabled;
        }
        if let Some(poll_interval_secs) = request.poll_interval_secs {
            config.poll_interval_secs = poll_interval_secs.clamp(60, 24 * 3600);
        }
        if let Some(importance_threshold) = request.importance_threshold {
            config.importance_threshold = importance_threshold.clamp(0.1, 1.0);
        }
        if let Some(notify_on_important) = request.notify_on_important {
            config.notify_on_important = notify_on_important;
        }
        if let Some(push_to_preferred_channel) = request.push_to_preferred_channel {
            config.push_to_preferred_channel = push_to_preferred_channel;
        }

        store.configs.insert(integration_id.to_string(), config);
        save_state_store(ctx, &store).await?;
    }

    let statuses = list_statuses(ctx).await;
    let status = statuses
        .into_iter()
        .find(|row| row.integration_id == integration_id)
        .ok_or_else(|| anyhow!("Failed to reload sync status for '{}'", integration_id))?;

    if let Some(error) = status.last_error.clone() {
        return Err(anyhow!(error));
    }

    Ok(status)
}

async fn sync_integration_persisted(
    ctx: &IntegrationSyncContext,
    integration_id: &str,
    trigger: &str,
    force: bool,
) -> Result<bool> {
    let Some(_integration_guard) = try_acquire_integration_sync_lock(integration_id).await else {
        if force {
            return Err(anyhow!(INTEGRATION_SYNC_BUSY_MESSAGE));
        }
        tracing::debug!(
            integration_id = integration_id,
            "Integration sync skipped because this integration is already active"
        );
        return Ok(false);
    };
    let manager = integration_manager(ctx);
    let (config, mut store, mut feed, mut runs) = {
        let _store_guard = INTEGRATION_SYNC_STORE_LOCK.lock().await;
        let store = load_state_store(ctx).await;
        let Some(config) = store
            .configs
            .get(integration_id)
            .cloned()
            .or_else(|| default_config_for(integration_id))
        else {
            return Err(anyhow!(
                "Background sync is not available for '{}'",
                integration_id
            ));
        };
        let cursor = store
            .cursors
            .get(integration_id)
            .cloned()
            .unwrap_or_default();
        if !force && !is_due(&cursor, &config) {
            return Ok(false);
        }
        (config, store, load_feed(ctx).await, load_runs(ctx).await)
    };

    let base_feed_ids = feed
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    let base_run_ids = runs
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();

    sync_integration(
        ctx, &manager, &config, &mut store, &mut feed, &mut runs, trigger, force,
    )
    .await;

    let updated_cursor = store.cursors.get(integration_id).cloned();
    let new_feed_items = feed
        .into_iter()
        .filter(|item| !base_feed_ids.contains(&item.id))
        .collect::<Vec<_>>();
    let new_run_items = runs
        .into_iter()
        .filter(|item| !base_run_ids.contains(&item.id))
        .collect::<Vec<_>>();

    {
        let _store_guard = INTEGRATION_SYNC_STORE_LOCK.lock().await;
        let mut latest_store = load_state_store(ctx).await;
        if let Some(cursor) = updated_cursor {
            latest_store
                .cursors
                .insert(integration_id.to_string(), cursor);
        }
        save_state_store(ctx, &latest_store).await?;

        let mut latest_feed = load_feed(ctx).await;
        merge_feed_items(&mut latest_feed, new_feed_items);
        save_feed(ctx, &latest_feed).await?;

        let mut latest_runs = load_runs(ctx).await;
        merge_run_items(&mut latest_runs, new_run_items);
        save_runs(ctx, &latest_runs).await?;
    }

    Ok(true)
}

pub async fn run_due_syncs(ctx: &IntegrationSyncContext) -> Result<()> {
    let started = Instant::now();
    let store = {
        let _store_guard = INTEGRATION_SYNC_STORE_LOCK.lock().await;
        load_state_store(ctx).await
    };
    let manager = integration_manager(ctx);

    let mut supported_ids = manager.ids();
    supported_ids.extend(
        ["google_workspace", "gmail"]
            .iter()
            .map(|id| (*id).to_string()),
    );
    supported_ids.sort();
    supported_ids.dedup();
    tracing::debug!(
        supported_integrations = supported_ids.len(),
        "Integration sync run started"
    );

    let mut due_configs = Vec::new();
    for integration_id in supported_ids {
        let Some(config) = store
            .configs
            .get(&integration_id)
            .cloned()
            .or_else(|| default_config_for(&integration_id))
        else {
            continue;
        };
        if !config.enabled {
            continue;
        }
        if !integration_dispatch_enabled(ctx, &manager, &integration_id).await {
            continue;
        }
        if !integration_connected(ctx, &manager, &integration_id).await {
            continue;
        }
        due_configs.push(config);
    }
    tracing::debug!(
        due_integrations = due_configs.len(),
        "Integration sync due integration scan completed"
    );

    let mut attempted_syncs = 0usize;
    for config in due_configs {
        match sync_integration_persisted(ctx, &config.integration_id, "background", false).await {
            Ok(true) => attempted_syncs = attempted_syncs.saturating_add(1),
            Ok(false) => {}
            Err(error) => tracing::warn!(
                integration_id = config.integration_id.as_str(),
                error = %error,
                "Integration sync skipped"
            ),
        }
    }

    if attempted_syncs > 0 {
        tracing::info!(
            attempted_syncs,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync run completed"
        );
    } else {
        tracing::debug!(
            attempted_syncs,
            duration_ms = started.elapsed().as_millis() as u64,
            "Integration sync run completed"
        );
    }
    Ok(())
}

pub async fn sync_now(
    ctx: &IntegrationSyncContext,
    integration_id: &str,
) -> Result<IntegrationSyncStatusView> {
    if default_config_for(integration_id).is_none() {
        return Err(anyhow!(
            "Background sync is not available for '{}'",
            integration_id
        ));
    }
    sync_integration_persisted(ctx, integration_id, "manual", true).await?;

    let statuses = list_statuses(ctx).await;
    statuses
        .into_iter()
        .find(|row| row.integration_id == integration_id)
        .ok_or_else(|| anyhow!("Failed to reload sync status for '{}'", integration_id))
}

fn build_run_summary(
    integration_name: &str,
    outcome: &SyncRunOutcome,
    connection_error: Option<&str>,
) -> String {
    match outcome.status {
        "blocked" => connection_error
            .map(short_error_summary)
            .unwrap_or_else(|| format!("{} sync did not start.", integration_name)),
        "failed" => outcome
            .error
            .as_deref()
            .map(short_error_summary)
            .unwrap_or_else(|| format!("{} sync failed.", integration_name)),
        _ if outcome.baseline_mode => format!(
            "Seeded {} baseline item{} from {} fetched.",
            outcome.recorded_item_count,
            if outcome.recorded_item_count == 1 {
                ""
            } else {
                "s"
            },
            outcome.fetched_item_count
        ),
        _ if outcome.new_item_count == 0 => {
            format!("No new {} updates detected.", integration_name)
        }
        _ => format!(
            "Detected {} new item{}, recorded {}, important {}.",
            outcome.new_item_count,
            if outcome.new_item_count == 1 { "" } else { "s" },
            outcome.recorded_item_count,
            outcome.important_item_count
        ),
    }
}

fn short_error_summary(error: &str) -> String {
    short_text(error, 160)
}

#[allow(clippy::too_many_arguments)]
fn push_run_record(
    runs: &mut Vec<IntegrationSyncRunItem>,
    config: &IntegrationSyncConfig,
    trigger: &str,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    connected: bool,
    integration_enabled: bool,
    cursor: &IntegrationSyncCursor,
    outcome: &SyncRunOutcome,
) {
    let integration_name = integration_name_from_id(&config.integration_id);
    let summary = build_run_summary(&integration_name, outcome, outcome.error.as_deref());
    runs.push(IntegrationSyncRunItem {
        id: hash_id(&[
            &config.integration_id,
            &started_at.to_rfc3339(),
            trigger,
            outcome.status,
            &summary,
        ]),
        integration_id: config.integration_id.clone(),
        integration_name,
        sync_kind: adapter_for(&config.integration_id)
            .map(|adapter| adapter.sync_kind)
            .unwrap_or("unsupported")
            .to_string(),
        trigger: trigger.to_string(),
        status: outcome.status.to_string(),
        summary,
        error: outcome.error.clone(),
        started_at: started_at.to_rfc3339(),
        completed_at: completed_at.to_rfc3339(),
        duration_ms: completed_at
            .signed_duration_since(started_at)
            .num_milliseconds()
            .max(0) as u64,
        fetched_item_count: outcome.fetched_item_count,
        new_item_count: outcome.new_item_count,
        recorded_item_count: outcome.recorded_item_count,
        important_item_count: outcome.important_item_count,
        baseline_mode: outcome.baseline_mode,
        connected,
        integration_enabled,
        last_item_at: cursor.last_item_at.clone(),
        sample_titles: outcome.sample_titles.clone(),
    });
    runs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    runs.truncate(MAX_RUN_ITEMS);
}

#[allow(clippy::too_many_arguments)]
async fn sync_integration(
    ctx: &IntegrationSyncContext,
    manager: &crate::integrations::IntegrationManager,
    config: &IntegrationSyncConfig,
    store: &mut IntegrationSyncStateStore,
    feed: &mut Vec<IntegrationSyncFeedItem>,
    runs: &mut Vec<IntegrationSyncRunItem>,
    trigger: &str,
    force: bool,
) {
    let started_at = Utc::now();
    let now_rfc3339 = started_at.to_rfc3339();
    let cursor = store
        .cursors
        .entry(config.integration_id.clone())
        .or_default();
    let previous_error = cursor.last_error.clone();

    if !force && !config.enabled {
        tracing::debug!(
            integration_id = config.integration_id.as_str(),
            trigger = trigger,
            "Integration sync skipped disabled config"
        );
        return;
    }

    let integration_enabled =
        integration_dispatch_enabled(ctx, manager, &config.integration_id).await;
    if !integration_enabled {
        cursor.last_sync_at = Some(now_rfc3339.clone());
        let error = "Integration is disabled.".to_string();
        cursor.last_error = Some(error.clone());
        push_run_record(
            runs,
            config,
            trigger,
            started_at,
            Utc::now(),
            false,
            false,
            cursor,
            &SyncRunOutcome {
                status: "blocked",
                error: Some(error),
                fetched_item_count: 0,
                new_item_count: 0,
                recorded_item_count: 0,
                important_item_count: 0,
                baseline_mode: false,
                sample_titles: Vec::new(),
            },
        );
        return;
    }

    let connected = integration_connected(ctx, manager, &config.integration_id).await;
    if !connected {
        cursor.last_sync_at = Some(now_rfc3339.clone());
        let error = "Integration is not connected.".to_string();
        maybe_emit_sync_attention_notification(ctx, config, previous_error.as_deref(), &error)
            .await;
        cursor.last_error = Some(error.clone());
        push_run_record(
            runs,
            config,
            trigger,
            started_at,
            Utc::now(),
            false,
            true,
            cursor,
            &SyncRunOutcome {
                status: "blocked",
                error: Some(error),
                fetched_item_count: 0,
                new_item_count: 0,
                recorded_item_count: 0,
                important_item_count: 0,
                baseline_mode: false,
                sample_titles: Vec::new(),
            },
        );
        return;
    }

    tracing::info!(
        integration_id = config.integration_id.as_str(),
        trigger = trigger,
        timeout_secs = INTEGRATION_SYNC_FETCH_TIMEOUT_SECS,
        "Integration sync fetch started"
    );
    let fetch_result = match tokio::time::timeout(
        StdDuration::from_secs(INTEGRATION_SYNC_FETCH_TIMEOUT_SECS),
        fetch_items(ctx, manager, &config.integration_id, cursor),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow!(
            "Integration fetch timed out after {}s",
            INTEGRATION_SYNC_FETCH_TIMEOUT_SECS
        )),
    };

    match fetch_result {
        Ok(items) => {
            tracing::info!(
                integration_id = config.integration_id.as_str(),
                trigger = trigger,
                fetched_items = items.len(),
                "Integration sync fetch completed"
            );
            let baseline_mode = cursor.seeded_at.is_none();
            let result = apply_items(ctx, config, cursor, feed, items, baseline_mode).await;
            cursor.last_sync_at = Some(now_rfc3339.clone());
            cursor.last_success_at = Some(now_rfc3339.clone());
            cursor.last_error = None;
            if baseline_mode {
                cursor.seeded_at = Some(now_rfc3339);
            }
            push_run_record(
                runs,
                config,
                trigger,
                started_at,
                Utc::now(),
                true,
                true,
                cursor,
                &SyncRunOutcome {
                    status: "completed",
                    error: None,
                    fetched_item_count: result.fetched_item_count,
                    new_item_count: result.new_item_count,
                    recorded_item_count: result.recorded_item_count,
                    important_item_count: result.important_item_count,
                    baseline_mode,
                    sample_titles: result.sample_titles,
                },
            );
        }
        Err(error) => {
            tracing::warn!(
                integration_id = config.integration_id.as_str(),
                trigger = trigger,
                error = %error,
                "Integration sync fetch failed"
            );
            cursor.last_sync_at = Some(now_rfc3339.clone());
            let error_text = short_text(&error.to_string(), 240);
            maybe_emit_sync_attention_notification(
                ctx,
                config,
                previous_error.as_deref(),
                &error_text,
            )
            .await;
            cursor.last_error = Some(error_text.clone());
            push_run_record(
                runs,
                config,
                trigger,
                started_at,
                Utc::now(),
                true,
                true,
                cursor,
                &SyncRunOutcome {
                    status: "failed",
                    error: Some(error_text),
                    fetched_item_count: 0,
                    new_item_count: 0,
                    recorded_item_count: 0,
                    important_item_count: 0,
                    baseline_mode: false,
                    sample_titles: Vec::new(),
                },
            );
        }
    }
}

async fn maybe_emit_sync_attention_notification(
    ctx: &IntegrationSyncContext,
    config: &IntegrationSyncConfig,
    previous_error: Option<&str>,
    error: &str,
) {
    if previous_error == Some(error) {
        return;
    }
    if config.integration_id != "google_workspace" {
        return;
    }
    let lowered = error.to_ascii_lowercase();
    if !lowered.contains("reconnect")
        && !lowered.contains("additional access")
        && !lowered.contains("grant")
    {
        return;
    }
    let Some(shared_agent) = ctx.shared_agent.as_ref() else {
        return;
    };
    let agent = shared_agent.read().await;
    agent
        .emit_notification(
            "Google Workspace needs more access",
            error,
            "warning",
            "integration_sync",
        )
        .await;
}

async fn apply_items(
    ctx: &IntegrationSyncContext,
    config: &IntegrationSyncConfig,
    cursor: &mut IntegrationSyncCursor,
    feed: &mut Vec<IntegrationSyncFeedItem>,
    items: Vec<NormalizedSyncItem>,
    baseline_mode: bool,
) -> ApplyItemsResult {
    let fetched_item_count = items.len();
    let mut existing_ids = cursor
        .recent_source_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let now = Utc::now().to_rfc3339();
    let integration_name = integration_name_from_id(&config.integration_id);
    let mut seen_ids = Vec::new();
    let mut new_items = Vec::new();

    let latest_at = items
        .iter()
        .filter_map(|item| item.occurred_at)
        .max()
        .map(|value| value.to_rfc3339());
    if latest_at.is_some() {
        cursor.last_item_at = latest_at;
    }

    for item in items {
        if existing_ids.insert(item.source_id.clone()) {
            seen_ids.push(item.source_id.clone());
            new_items.push(item);
        }
    }

    let mut important_items = Vec::new();
    let mut result = ApplyItemsResult {
        fetched_item_count,
        new_item_count: new_items.len(),
        ..ApplyItemsResult::default()
    };
    if new_items.is_empty() {
        return result;
    }

    for (index, item) in new_items.into_iter().enumerate() {
        let important = !baseline_mode && item.importance >= config.importance_threshold;
        let outcome = if baseline_mode {
            "baseline"
        } else if important {
            "important"
        } else {
            "recorded"
        };
        if baseline_mode && index >= MAX_BASELINE_ITEMS {
            continue;
        }
        if important {
            important_items.push(item.clone());
        }
        result.recorded_item_count += 1;
        if important {
            result.important_item_count += 1;
        }
        if result.sample_titles.len() < 5 {
            result.sample_titles.push(short_text(&item.title, 80));
        }
        feed.push(IntegrationSyncFeedItem {
            id: hash_id(&[&config.integration_id, &item.source_id, &item.title, &now]),
            integration_id: config.integration_id.clone(),
            integration_name: integration_name.clone(),
            kind: item.kind,
            title: short_text(&item.title, 120),
            summary: short_text(&item.summary, 220),
            url: item.url,
            occurred_at: item.occurred_at.map(|value| value.to_rfc3339()),
            detected_at: now.clone(),
            importance: clamp_importance(item.importance),
            important,
            outcome: outcome.to_string(),
        });
    }

    push_recent_source_ids(cursor, &seen_ids);
    feed.sort_by(|left, right| right.detected_at.cmp(&left.detected_at));
    feed.truncate(MAX_FEED_ITEMS);

    if !important_items.is_empty() && config.notify_on_important {
        let title = format!("Important {} updates", integration_name);
        let mut lines = Vec::new();
        for item in important_items.iter().take(5) {
            let line = if item.summary.is_empty() {
                format!("- {}", item.title)
            } else {
                format!("- {}: {}", item.title, item.summary)
            };
            lines.push(line);
        }
        let body = lines.join("\n");
        if let Some(shared_agent) = ctx.shared_agent.as_ref() {
            let agent = shared_agent.read().await;
            agent
                .emit_notification(&title, &body, "info", "integration_sync")
                .await;
            if config.push_to_preferred_channel {
                let report = format!("{}\n{}", title, body);
                agent.notify_preferred_channel(&report).await;
            }
        }
    }
    result
}

async fn fetch_items(
    ctx: &IntegrationSyncContext,
    manager: &crate::integrations::IntegrationManager,
    integration_id: &str,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    match integration_id {
        "google_workspace" => fetch_google_workspace_items(ctx, manager, cursor).await,
        "gmail" => fetch_gmail_items(ctx, cursor).await,
        "google_calendar" => fetch_calendar_items(manager).await,
        "github" => fetch_github_items(ctx, cursor).await,
        "notion" => fetch_notion_items(manager).await,
        "twitter" => fetch_twitter_items(manager).await,
        "onepassword" => fetch_onepassword_items(manager).await,
        "twilio" => fetch_twilio_items(manager).await,
        "ordering" => fetch_ordering_items(manager).await,
        "garmin" => fetch_garmin_items(manager).await,
        "whoop" => fetch_whoop_items(manager).await,
        "ga4" => fetch_ga4_items(manager).await,
        "gsc" => fetch_gsc_items(manager).await,
        "social_analytics" => fetch_social_analytics_items(manager).await,
        _ => Err(anyhow!(
            "Background sync is not available for '{}'",
            integration_id
        )),
    }
}

async fn fetch_google_workspace_items(
    ctx: &IntegrationSyncContext,
    manager: &crate::integrations::IntegrationManager,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    let bundles = crate::actions::google_workspace::granted_bundles(&ctx.config_dir)?;
    let mut items = Vec::new();
    if bundles.iter().any(|bundle| bundle == "gmail") {
        items.extend(fetch_gmail_items(ctx, cursor).await?);
    }
    if bundles.iter().any(|bundle| bundle == "calendar") {
        items.extend(fetch_calendar_items(manager).await?);
    }
    if bundles.iter().any(|bundle| bundle == "drive") {
        items.extend(fetch_workspace_drive_items(ctx).await?);
    }
    items.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
    items.truncate(40);
    Ok(items)
}

async fn fetch_workspace_drive_items(
    ctx: &IntegrationSyncContext,
) -> Result<Vec<NormalizedSyncItem>> {
    let access_token = crate::actions::google_workspace::ensure_access_token_for_bundles(
        &ctx.config_dir,
        &["drive"],
    )
    .await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut url = reqwest::Url::parse("https://www.googleapis.com/drive/v3/files")?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("pageSize", "20");
        query.append_pair(
            "fields",
            "files(id,name,mimeType,modifiedTime,webViewLink,lastModifyingUser(displayName,emailAddress))",
        );
        query.append_pair("orderBy", "modifiedTime desc");
    }
    let response = send_integration_sync_request(
        "Google Drive activity",
        client.get(url).bearer_auth(access_token),
    )
    .await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Google Drive activity failed: {}",
            response.status()
        ));
    }
    let data: serde_json::Value = response.json().await?;
    let files = data
        .get("files")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(files
        .into_iter()
        .map(|file| {
            let title = file
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("Drive file")
                .to_string();
            let modified = file
                .get("modifiedTime")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let occurred_at = parse_datetime(modified);
            let modifier = file
                .get("lastModifyingUser")
                .and_then(|value| {
                    value
                        .get("displayName")
                        .and_then(|name| name.as_str())
                        .or_else(|| value.get("emailAddress").and_then(|email| email.as_str()))
                })
                .unwrap_or("someone");
            let mime = file
                .get("mimeType")
                .and_then(|value| value.as_str())
                .unwrap_or("file");
            NormalizedSyncItem {
                source_id: file
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or(&title)
                    .to_string(),
                kind: "file".to_string(),
                title: format!("Drive: {}", title),
                summary: format!("{} updated by {} at {}", mime, modifier, modified),
                url: file
                    .get("webViewLink")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                occurred_at,
                importance: clamp_importance(
                    0.34 + recency_importance_boost(occurred_at) + text_importance_boost(&title),
                ),
            }
        })
        .collect())
}

async fn fetch_notion_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute("notion", "search", &serde_json::json!({ "query": "" }))
        .await?;
    let pages = payload
        .get("pages")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(pages
        .into_iter()
        .map(|page| {
            let title = page
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("Untitled page");
            let edited = page
                .get("last_edited_time")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let occurred_at = parse_datetime(edited);
            let summary = if edited.is_empty() {
                "Recently edited page".to_string()
            } else {
                format!("Last edited {}", edited)
            };
            NormalizedSyncItem {
                source_id: format!(
                    "{}:{}",
                    page.get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("page"),
                    edited
                ),
                kind: "page".to_string(),
                title: title.to_string(),
                summary,
                url: page
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                occurred_at,
                importance: clamp_importance(
                    0.34 + recency_importance_boost(occurred_at) + text_importance_boost(title),
                ),
            }
        })
        .collect())
}

async fn fetch_twitter_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute(
            "twitter",
            "list_tweets",
            &serde_json::json!({ "max_results": 20 }),
        )
        .await?;
    let tweets = payload
        .get("tweets")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(tweets
        .into_iter()
        .map(|tweet| {
            let text = tweet
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("Tweet");
            let occurred_at = tweet
                .get("created_at")
                .and_then(|value| value.as_str())
                .and_then(parse_datetime);
            NormalizedSyncItem {
                source_id: tweet
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("tweet")
                    .to_string(),
                kind: "tweet".to_string(),
                title: short_text(text, 90),
                summary: "Published tweet".to_string(),
                url: tweet
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(|value| format!("https://twitter.com/i/web/status/{}", value)),
                occurred_at,
                importance: clamp_importance(
                    0.24 + recency_importance_boost(occurred_at) + text_importance_boost(text),
                ),
            }
        })
        .collect())
}

async fn fetch_onepassword_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute("onepassword", "list_vaults", &serde_json::json!({}))
        .await?;
    let vaults = payload
        .get("vaults")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(vaults
        .into_iter()
        .map(|vault| {
            let name = vault
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("Vault");
            NormalizedSyncItem {
                source_id: vault
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("vault")
                    .to_string(),
                kind: "vault".to_string(),
                title: name.to_string(),
                summary: vault
                    .get("description")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Available vault")
                    .to_string(),
                url: None,
                occurred_at: None,
                importance: clamp_importance(0.25 + text_importance_boost(name)),
            }
        })
        .collect())
}

async fn fetch_twilio_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let messages = manager
        .execute("twilio", "list_messages", &serde_json::json!({}))
        .await?;
    let calls = manager
        .execute("twilio", "list_calls", &serde_json::json!({}))
        .await?;

    let mut items = Vec::new();
    for message in messages
        .get("messages")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let body = message
            .get("body")
            .and_then(|value| value.as_str())
            .unwrap_or("SMS message");
        let status = message
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let occurred_at = message
            .get("date_sent")
            .and_then(|value| value.as_str())
            .and_then(parse_datetime);
        items.push(NormalizedSyncItem {
            source_id: message
                .get("sid")
                .and_then(|value| value.as_str())
                .unwrap_or("message")
                .to_string(),
            kind: "message".to_string(),
            title: short_text(body, 90),
            summary: format!(
                "{} -> {} ({})",
                message
                    .get("from")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?"),
                message
                    .get("to")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?"),
                status
            ),
            url: None,
            occurred_at,
            importance: clamp_importance(
                0.42 + recency_importance_boost(occurred_at)
                    + if status.eq_ignore_ascii_case("failed") {
                        0.24
                    } else {
                        0.0
                    }
                    + text_importance_boost(body),
            ),
        });
    }
    for call in calls
        .get("calls")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let status = call
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let occurred_at = call
            .get("date_created")
            .and_then(|value| value.as_str())
            .and_then(parse_datetime);
        items.push(NormalizedSyncItem {
            source_id: call
                .get("sid")
                .and_then(|value| value.as_str())
                .unwrap_or("call")
                .to_string(),
            kind: "call".to_string(),
            title: format!(
                "Call {} -> {}",
                call.get("from")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?"),
                call.get("to")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
            ),
            summary: format!("Status: {}", status),
            url: None,
            occurred_at,
            importance: clamp_importance(
                0.4 + recency_importance_boost(occurred_at)
                    + if matches!(
                        status.to_ascii_lowercase().as_str(),
                        "busy" | "failed" | "no-answer" | "canceled"
                    ) {
                        0.28
                    } else {
                        0.0
                    },
            ),
        });
    }
    Ok(items)
}

async fn fetch_ordering_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute("ordering", "list_orders", &serde_json::json!({}))
        .await?;
    let orders = payload
        .get("orders")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(orders
        .into_iter()
        .map(|order| {
            let order_id = order
                .get("order_id")
                .and_then(|value| value.as_str())
                .unwrap_or("order");
            let status = order
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let occurred_at = order
                .get("created_at")
                .and_then(|value| value.as_str())
                .and_then(parse_datetime);
            NormalizedSyncItem {
                source_id: order_id.to_string(),
                kind: "order".to_string(),
                title: format!("Order {}", order_id),
                summary: format!(
                    "Status: {} | Total: {}",
                    status,
                    order
                        .get("total_price")
                        .and_then(|value| value.as_str())
                        .unwrap_or("?")
                ),
                url: None,
                occurred_at,
                importance: clamp_importance(
                    0.48 + recency_importance_boost(occurred_at)
                        + if matches!(
                            status.to_ascii_lowercase().as_str(),
                            "pending" | "unfulfilled" | "open"
                        ) {
                            0.18
                        } else {
                            0.0
                        },
                ),
            }
        })
        .collect())
}

async fn fetch_garmin_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute("garmin", "activities", &serde_json::json!({ "limit": 20 }))
        .await?;
    let activities = payload
        .get("activities")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(activities
        .into_iter()
        .map(|activity| {
            let start = activity
                .get("startTimeLocal")
                .or_else(|| activity.get("startTimeGMT"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let activity_type = activity
                .get("activityType")
                .and_then(|value| value.as_str())
                .unwrap_or("activity");
            let title = format!("Garmin {}", activity_type);
            let occurred_at = parse_datetime(start);
            NormalizedSyncItem {
                source_id: activity
                    .get("activityId")
                    .and_then(|value| value.as_i64())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| hash_id(&[&title, start])),
                kind: "activity".to_string(),
                title: title.clone(),
                summary: format!(
                    "{} | {}",
                    short_text(
                        activity
                            .get("activityName")
                            .and_then(|value| value.as_str())
                            .unwrap_or("Workout"),
                        80
                    ),
                    start
                ),
                url: None,
                occurred_at,
                importance: clamp_importance(
                    0.28 + recency_importance_boost(occurred_at) + text_importance_boost(&title),
                ),
            }
        })
        .collect())
}

async fn fetch_whoop_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute("whoop", "workouts", &serde_json::json!({ "limit": 20 }))
        .await?;
    let workouts = payload.as_array().cloned().unwrap_or_default();
    Ok(workouts
        .into_iter()
        .map(|workout| {
            let title = workout
                .get("sport_name")
                .or_else(|| workout.get("sport"))
                .and_then(|value| value.as_str())
                .unwrap_or("Workout");
            let occurred_at = workout
                .get("start")
                .or_else(|| workout.get("start_time"))
                .and_then(|value| value.as_str())
                .and_then(parse_datetime);
            NormalizedSyncItem {
                source_id: workout
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("workout")
                    .to_string(),
                kind: "workout".to_string(),
                title: title.to_string(),
                summary: short_text(
                    workout
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("Workout recorded"),
                    80,
                ),
                url: None,
                occurred_at,
                importance: clamp_importance(
                    0.28 + recency_importance_boost(occurred_at) + text_importance_boost(title),
                ),
            }
        })
        .collect())
}

async fn fetch_ga4_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute(
            "ga4",
            "run_report",
            &serde_json::json!({
                "dimensions": ["date"],
                "metrics": ["sessions", "activeUsers"],
                "date_ranges": [{"startDate":"2daysAgo","endDate":"today"}],
                "limit": 3
            }),
        )
        .await?;
    let rows = payload
        .get("report")
        .and_then(|value| value.get("rows"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut items = Vec::new();
    for row in rows {
        let dimensions = row
            .get("dimensionValues")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let metrics = row
            .get("metricValues")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let date = dimensions
            .first()
            .and_then(|value| value.get("value"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let sessions = metrics
            .first()
            .and_then(|value| value.get("value"))
            .and_then(|value| value.as_str())
            .unwrap_or("0");
        let active_users = metrics
            .get(1)
            .and_then(|value| value.get("value"))
            .and_then(|value| value.as_str())
            .unwrap_or("0");
        let occurred_at = parse_datetime(date);
        items.push(NormalizedSyncItem {
            source_id: format!("ga4:{}", date),
            kind: "analytics".to_string(),
            title: format!("GA4 traffic {}", date),
            summary: format!("Sessions: {} | Active users: {}", sessions, active_users),
            url: None,
            occurred_at,
            importance: clamp_importance(
                0.36 + recency_importance_boost(occurred_at)
                    + if sessions == "0" { 0.28 } else { 0.0 },
            ),
        });
    }
    Ok(items)
}

async fn fetch_gsc_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute(
            "gsc",
            "query",
            &serde_json::json!({
                "dimensions": ["date"],
                "start_date": "7daysAgo",
                "end_date": "today",
                "row_limit": 7
            }),
        )
        .await?;
    let rows = payload
        .get("result")
        .and_then(|value| value.get("rows"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut items = Vec::new();
    for row in rows {
        let keys = row
            .get("keys")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let date = keys.first().and_then(|value| value.as_str()).unwrap_or("");
        let occurred_at = parse_datetime(date);
        items.push(NormalizedSyncItem {
            source_id: format!("gsc:{}", date),
            kind: "analytics".to_string(),
            title: format!("Search Console {}", date),
            summary: format!(
                "Clicks: {} | Impressions: {}",
                row.get("clicks")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0),
                row.get("impressions")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0)
            ),
            url: None,
            occurred_at,
            importance: clamp_importance(
                0.36 + recency_importance_boost(occurred_at)
                    + if row
                        .get("clicks")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(0.0)
                        <= 0.0
                    {
                        0.24
                    } else {
                        0.0
                    },
            ),
        });
    }
    Ok(items)
}

async fn fetch_social_analytics_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute(
            "social_analytics",
            "summary",
            &serde_json::json!({ "days": 2, "post_limit": 25 }),
        )
        .await?;
    let sources = payload
        .get("sources")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut items = Vec::new();
    for source in sources {
        let platform = source
            .get("platform")
            .and_then(|value| value.as_str())
            .unwrap_or("social");
        let summary = short_text(&source.to_string(), 180);
        items.push(NormalizedSyncItem {
            source_id: hash_id(&[platform, &summary]),
            kind: "analytics".to_string(),
            title: format!("{} summary updated", platform.to_ascii_uppercase()),
            summary,
            url: None,
            occurred_at: Some(Utc::now()),
            importance: clamp_importance(0.34 + text_importance_boost(platform)),
        });
    }
    Ok(items)
}

async fn fetch_calendar_items(
    manager: &crate::integrations::IntegrationManager,
) -> Result<Vec<NormalizedSyncItem>> {
    let payload = manager
        .execute("google_calendar", "this_week", &serde_json::json!({}))
        .await?;
    let events = payload.as_array().cloned().unwrap_or_default();
    Ok(events
        .into_iter()
        .map(|event| {
            let title = event
                .get("summary")
                .or_else(|| event.get("title"))
                .and_then(|value| value.as_str())
                .unwrap_or("Calendar event");
            let start_text = event
                .get("start")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let occurred_at = parse_datetime(start_text);
            let time_boost = occurred_at.map_or(0.0, |at| {
                let delta = at - Utc::now();
                if delta <= Duration::hours(6) && delta >= Duration::minutes(-30) {
                    0.32
                } else if delta <= Duration::hours(24) && delta >= Duration::hours(-4) {
                    0.18
                } else {
                    0.0
                }
            });
            NormalizedSyncItem {
                source_id: format!(
                    "{}:{}",
                    event
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("event"),
                    start_text
                ),
                kind: "event".to_string(),
                title: title.to_string(),
                summary: format!(
                    "{}{}",
                    start_text,
                    event
                        .get("location")
                        .and_then(|value| value.as_str())
                        .map(|value| format!(" @ {}", value))
                        .unwrap_or_default()
                ),
                url: event
                    .get("html_link")
                    .or_else(|| event.get("htmlLink"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                occurred_at,
                importance: clamp_importance(0.34 + time_boost + text_importance_boost(title)),
            }
        })
        .collect())
}

async fn fetch_github_items(
    ctx: &IntegrationSyncContext,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    let token = crate::integrations::github::GitHubConnector::load_token_from(&ctx.config_dir)
        .ok_or_else(|| anyhow!("GitHub token not configured"))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut successful_fetches = 0usize;

    match fetch_github_notification_items(&client, &token, cursor).await {
        Ok(mut fetched) => {
            successful_fetches += 1;
            items.append(&mut fetched);
        }
        Err(error) => errors.push(error.to_string()),
    }

    match fetch_github_repository_activity_items(&client, &token, cursor, "pushed").await {
        Ok(mut fetched) => {
            successful_fetches += 1;
            items.append(&mut fetched);
        }
        Err(error) => errors.push(error.to_string()),
    }

    match fetch_github_repository_activity_items(&client, &token, cursor, "updated").await {
        Ok(mut fetched) => {
            successful_fetches += 1;
            items.append(&mut fetched);
        }
        Err(error) => errors.push(error.to_string()),
    }

    match fetch_github_issue_activity_items(&client, &token, cursor).await {
        Ok(mut fetched) => {
            successful_fetches += 1;
            items.append(&mut fetched);
        }
        Err(error) => errors.push(error.to_string()),
    }

    match fetch_github_security_alert_items(&client, &token, cursor).await {
        Ok(mut fetched) => {
            successful_fetches += 1;
            items.append(&mut fetched);
        }
        Err(error) => errors.push(error.to_string()),
    }

    match fetch_github_public_event_items(&client, &token, cursor).await {
        Ok(mut fetched) => {
            successful_fetches += 1;
            items.append(&mut fetched);
        }
        Err(error) => errors.push(error.to_string()),
    }

    if successful_fetches == 0 {
        return Err(anyhow!(
            "GitHub activity fetch failed: {}",
            errors.join("; ")
        ));
    }
    if !errors.is_empty() {
        tracing::warn!(
            errors = ?errors,
            "GitHub activity sync completed with partial API coverage"
        );
    }

    items.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
    items.truncate(120);
    Ok(items)
}

async fn fetch_github_notification_items(
    client: &reqwest::Client,
    token: &str,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    let mut url = github_api_url(&["notifications"], &[])?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("all", "false");
        query.append_pair("participating", "false");
        query.append_pair("per_page", "25");
        if let Some(since) = cursor.last_success_at.as_deref() {
            query.append_pair("since", since);
        }
    }
    let notifications = github_get_json_array(client, token, url, "notifications").await?;
    Ok(notifications
        .into_iter()
        .map(|item| {
            let subject = item
                .get("subject")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let title = subject
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("GitHub notification");
            let reason = item
                .get("reason")
                .and_then(|value| value.as_str())
                .unwrap_or("notification");
            let repo = item
                .get("repository")
                .and_then(|value| value.get("full_name"))
                .and_then(|value| value.as_str())
                .unwrap_or("repository");
            let updated_at = item
                .get("updated_at")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let occurred_at = parse_datetime(updated_at);
            let raw_id = item
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or(title);
            let reason_boost = match reason {
                "security_alert" => 0.36,
                "review_requested" => 0.32,
                "assign" => 0.28,
                "mention" | "team_mention" => 0.26,
                "author" | "comment" => 0.18,
                _ => 0.1,
            };
            NormalizedSyncItem {
                source_id: format!("github:notification:{}:{}", raw_id, updated_at),
                kind: subject
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("notification")
                    .to_ascii_lowercase(),
                title: title.to_string(),
                summary: format!("{} | {}", repo, reason.replace('_', " ")),
                url: subject
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                occurred_at,
                importance: clamp_importance(
                    0.36 + reason_boost
                        + recency_importance_boost(occurred_at)
                        + text_importance_boost(title),
                ),
            }
        })
        .collect())
}

async fn fetch_github_repository_activity_items(
    client: &reqwest::Client,
    token: &str,
    cursor: &IntegrationSyncCursor,
    sort: &str,
) -> Result<Vec<NormalizedSyncItem>> {
    let url = github_api_url(
        &["user", "repos"],
        &[
            ("visibility", "all".to_string()),
            (
                "affiliation",
                "owner,collaborator,organization_member".to_string(),
            ),
            ("sort", sort.to_string()),
            ("direction", "desc".to_string()),
            ("per_page", GITHUB_SYNC_REPO_LIMIT.to_string()),
        ],
    )?;
    let repos = github_get_json_array(client, token, url, "repository activity").await?;
    let timestamp_field = if sort == "updated" {
        "updated_at"
    } else {
        "pushed_at"
    };
    let verb = if sort == "updated" {
        "updated"
    } else {
        "pushed"
    };

    let mut items = Vec::new();
    for repo in repos {
        let full_name = repo
            .get("full_name")
            .and_then(|value| value.as_str())
            .unwrap_or("repository");
        let timestamp = repo
            .get(timestamp_field)
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let occurred_at = parse_datetime(timestamp);
        if timestamp.is_empty() || !github_after_last_success(cursor, occurred_at) {
            continue;
        }
        let description = repo
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let visibility = repo
            .get("visibility")
            .and_then(|value| value.as_str())
            .unwrap_or("repository");
        let private_label = if repo
            .get("private")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "private"
        } else {
            visibility
        };
        items.push(NormalizedSyncItem {
            source_id: format!(
                "github:repo:{}:{}:{}",
                verb,
                repo.get("id")
                    .and_then(|value| value.as_i64())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| full_name.to_string()),
                timestamp
            ),
            kind: format!("repo_{}", verb),
            title: format!("{} {}", full_name, verb),
            summary: if description.trim().is_empty() {
                format!("{} repository {}", private_label, timestamp)
            } else {
                format!(
                    "{} repository | {} | {}",
                    private_label,
                    short_text(description, 90),
                    timestamp
                )
            },
            url: repo
                .get("html_url")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .or_else(|| github_repo_url(full_name)),
            occurred_at,
            importance: clamp_importance(
                0.34 + recency_importance_boost(occurred_at) + text_importance_boost(full_name),
            ),
        });
    }
    Ok(items)
}

async fn fetch_github_issue_activity_items(
    client: &reqwest::Client,
    token: &str,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    let mut url = github_api_url(
        &["issues"],
        &[
            ("filter", "all".to_string()),
            ("state", "all".to_string()),
            ("sort", "updated".to_string()),
            ("direction", "desc".to_string()),
            ("per_page", "25".to_string()),
        ],
    )?;
    if let Some(since) = cursor.last_success_at.as_deref() {
        url.query_pairs_mut().append_pair("since", since);
    }
    let issues = github_get_json_array(client, token, url, "issue activity").await?;
    let mut items = Vec::new();
    for issue in issues {
        let title = issue
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or("GitHub issue");
        let updated_at = issue
            .get("updated_at")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let occurred_at = parse_datetime(updated_at);
        if !github_after_last_success(cursor, occurred_at) {
            continue;
        }
        let repository = issue
            .get("repository")
            .and_then(|value| value.get("full_name"))
            .and_then(|value| value.as_str())
            .unwrap_or("repository");
        let number = issue
            .get("number")
            .and_then(|value| value.as_i64())
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let is_pr = issue.get("pull_request").is_some();
        let kind = if is_pr { "pull_request" } else { "issue" };
        items.push(NormalizedSyncItem {
            source_id: format!(
                "github:{}:{}:{}",
                kind,
                issue
                    .get("id")
                    .and_then(|value| value.as_i64())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| format!("{}#{}", repository, number)),
                updated_at
            ),
            kind: kind.to_string(),
            title: format!(
                "{} #{} {}",
                if is_pr { "PR" } else { "Issue" },
                number,
                title
            ),
            summary: format!("{} | updated {}", repository, updated_at),
            url: issue
                .get("html_url")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .or_else(|| github_repo_url(repository)),
            occurred_at,
            importance: clamp_importance(
                0.4 + recency_importance_boost(occurred_at) + text_importance_boost(title),
            ),
        });
    }
    Ok(items)
}

async fn fetch_github_security_alert_items(
    client: &reqwest::Client,
    token: &str,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    let repos = github_get_json_array(
        client,
        token,
        github_api_url(
            &["user", "repos"],
            &[
                ("visibility", "all".to_string()),
                (
                    "affiliation",
                    "owner,collaborator,organization_member".to_string(),
                ),
                ("sort", "updated".to_string()),
                ("direction", "desc".to_string()),
                ("per_page", GITHUB_SYNC_ALERT_REPO_LIMIT.to_string()),
            ],
        )?,
        "alert repository discovery",
    )
    .await?;

    let mut items = Vec::new();
    let mut attempted = 0usize;
    for repo in repos {
        let full_name = repo
            .get("full_name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let Some((owner, repo_name)) = full_name.split_once('/') else {
            continue;
        };
        for alert_surface in ["dependabot", "code-scanning", "secret-scanning"] {
            attempted += 1;
            let path = match alert_surface {
                "dependabot" => vec!["repos", owner, repo_name, "dependabot", "alerts"],
                "code-scanning" => vec!["repos", owner, repo_name, "code-scanning", "alerts"],
                _ => vec!["repos", owner, repo_name, "secret-scanning", "alerts"],
            };
            let query = match alert_surface {
                "dependabot" => vec![
                    ("state", "open".to_string()),
                    ("per_page", GITHUB_SYNC_ALERT_LIMIT.to_string()),
                ],
                _ => vec![
                    ("state", "open".to_string()),
                    ("sort", "updated".to_string()),
                    ("direction", "desc".to_string()),
                    ("per_page", GITHUB_SYNC_ALERT_LIMIT.to_string()),
                ],
            };
            let url = github_api_url(&path, &query)?;
            match github_get_json_array(client, token, url, alert_surface).await {
                Ok(alerts) => {
                    for alert in alerts {
                        if let Some(item) =
                            github_alert_to_sync_item(alert_surface, full_name, &alert, cursor)
                        {
                            items.push(item);
                        }
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        repository = full_name,
                        surface = alert_surface,
                        error = %error,
                        "GitHub security alert surface unavailable"
                    );
                }
            }
        }
    }

    if attempted > 0 || !items.is_empty() {
        Ok(items)
    } else {
        Err(anyhow!(
            "No GitHub repositories were available for alert polling"
        ))
    }
}

fn github_alert_to_sync_item(
    surface: &str,
    repository: &str,
    alert: &serde_json::Value,
    cursor: &IntegrationSyncCursor,
) -> Option<NormalizedSyncItem> {
    let updated_at = alert
        .get("updated_at")
        .or_else(|| alert.get("fixed_at"))
        .or_else(|| alert.get("dismissed_at"))
        .or_else(|| alert.get("resolved_at"))
        .or_else(|| alert.get("created_at"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let occurred_at = parse_datetime(updated_at);
    if updated_at.is_empty() || !github_after_last_success(cursor, occurred_at) {
        return None;
    }

    let number = alert
        .get("number")
        .or_else(|| alert.get("id"))
        .and_then(|value| {
            value
                .as_i64()
                .map(|number| number.to_string())
                .or_else(|| value.as_str().map(|raw| raw.to_string()))
        })
        .unwrap_or_else(|| hash_id(&[surface, repository, updated_at]));
    let state = alert
        .get("state")
        .and_then(|value| value.as_str())
        .unwrap_or("open");
    let title = match surface {
        "dependabot" => {
            let package = alert
                .get("dependency")
                .and_then(|value| value.get("package"))
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str())
                .unwrap_or("dependency");
            let advisory = alert
                .get("security_advisory")
                .and_then(|value| value.get("summary"))
                .and_then(|value| value.as_str())
                .unwrap_or("Dependabot alert");
            format!(
                "Dependabot alert in {}: {} ({})",
                repository, advisory, package
            )
        }
        "code-scanning" => {
            let rule = alert
                .get("rule")
                .and_then(|value| {
                    value
                        .get("name")
                        .and_then(|name| name.as_str())
                        .or_else(|| value.get("id").and_then(|id| id.as_str()))
                })
                .unwrap_or("code scanning alert");
            format!("Code scanning alert in {}: {}", repository, rule)
        }
        _ => {
            let secret_type = alert
                .get("secret_type_display_name")
                .or_else(|| alert.get("secret_type"))
                .and_then(|value| value.as_str())
                .unwrap_or("secret");
            format!("Secret scanning alert in {}: {}", repository, secret_type)
        }
    };

    Some(NormalizedSyncItem {
        source_id: format!(
            "github:alert:{}:{}:{}:{}",
            surface, repository, number, updated_at
        ),
        kind: format!("{}_alert", surface.replace('-', "_")),
        title: short_text(&title, 120),
        summary: format!("{} | {} | updated {}", surface, state, updated_at),
        url: alert
            .get("html_url")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| github_repo_url(repository)),
        occurred_at,
        importance: clamp_importance(
            0.78 + recency_importance_boost(occurred_at) + text_importance_boost(&title),
        ),
    })
}

async fn fetch_github_public_event_items(
    client: &reqwest::Client,
    token: &str,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    let profile =
        github_get_json_value(client, token, github_api_url(&["user"], &[])?, "user").await?;
    let login = profile
        .get("login")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("GitHub user payload did not include login"))?;
    let mut events = Vec::new();
    for (label, segments) in [
        ("user events", vec!["users", login, "events"]),
        ("received events", vec!["users", login, "received_events"]),
    ] {
        let url = github_api_url(
            &segments,
            &[("per_page", GITHUB_SYNC_EVENT_LIMIT.to_string())],
        )?;
        match github_get_json_array(client, token, url, label).await {
            Ok(mut fetched) => events.append(&mut fetched),
            Err(error) => tracing::warn!(error = %error, "GitHub event surface fetch failed"),
        }
    }

    let mut items = Vec::new();
    for event in events {
        let occurred_at = event
            .get("created_at")
            .and_then(|value| value.as_str())
            .and_then(parse_datetime);
        if !github_after_last_success(cursor, occurred_at) {
            continue;
        }
        if let Some(item) = github_event_to_sync_item(&event, occurred_at) {
            items.push(item);
        }
    }
    Ok(items)
}

fn github_event_to_sync_item(
    event: &serde_json::Value,
    occurred_at: Option<DateTime<Utc>>,
) -> Option<NormalizedSyncItem> {
    let event_id = event.get("id").and_then(|value| value.as_str())?;
    let event_type = event
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("GitHubEvent");
    let repo = event
        .get("repo")
        .and_then(|value| value.get("name"))
        .and_then(|value| value.as_str())
        .unwrap_or("repository");
    let actor = event
        .get("actor")
        .and_then(|value| {
            value
                .get("display_login")
                .and_then(|login| login.as_str())
                .or_else(|| value.get("login").and_then(|login| login.as_str()))
        })
        .unwrap_or("someone");
    let payload = event
        .get("payload")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let action = payload
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    let (title, summary, url) = match event_type {
        "PushEvent" => {
            let commit_count = payload
                .get("commits")
                .and_then(|value| value.as_array())
                .map(|value| value.len())
                .unwrap_or(0);
            let raw_ref = payload
                .get("ref")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let branch = raw_ref.strip_prefix("refs/heads/").unwrap_or(raw_ref);
            (
                format!("Push to {}", repo),
                format!(
                    "{} pushed {} commit{}{}",
                    actor,
                    commit_count,
                    if commit_count == 1 { "" } else { "s" },
                    if branch.is_empty() {
                        String::new()
                    } else {
                        format!(" to {}", branch)
                    }
                ),
                if branch.is_empty() {
                    github_repo_url(repo)
                } else {
                    Some(format!("https://github.com/{}/commits/{}", repo, branch))
                },
            )
        }
        "PullRequestEvent" => {
            let pr = payload
                .get("pull_request")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let pr_title = pr
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("pull request");
            (
                format!("PR {} {}", action, pr_title).trim().to_string(),
                format!("{} | {}", repo, event_type),
                pr.get("html_url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
                    .or_else(|| github_repo_url(repo)),
            )
        }
        "IssuesEvent" => {
            let issue = payload
                .get("issue")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let issue_title = issue
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("issue");
            (
                format!("Issue {} {}", action, issue_title)
                    .trim()
                    .to_string(),
                format!("{} | {}", repo, event_type),
                issue
                    .get("html_url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
                    .or_else(|| github_repo_url(repo)),
            )
        }
        "IssueCommentEvent" => {
            let issue = payload
                .get("issue")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let issue_title = issue
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("issue");
            (
                format!("Comment on {}", issue_title),
                format!("{} | {}", repo, event_type),
                issue
                    .get("html_url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
                    .or_else(|| github_repo_url(repo)),
            )
        }
        "ReleaseEvent" => {
            let release = payload
                .get("release")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let release_name = release
                .get("name")
                .and_then(|value| value.as_str())
                .or_else(|| release.get("tag_name").and_then(|value| value.as_str()))
                .unwrap_or("release");
            (
                format!("Release {} {}", action, release_name)
                    .trim()
                    .to_string(),
                format!("{} | {}", repo, event_type),
                release
                    .get("html_url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
                    .or_else(|| github_repo_url(repo)),
            )
        }
        _ => {
            let cleaned = event_type.trim_end_matches("Event");
            (
                if action.is_empty() {
                    format!("{} in {}", cleaned, repo)
                } else {
                    format!("{} {} in {}", cleaned, action, repo)
                },
                format!("{} | {}", actor, event_type),
                github_repo_url(repo),
            )
        }
    };

    Some(NormalizedSyncItem {
        source_id: format!("github:event:{}", event_id),
        kind: event_type
            .trim_end_matches("Event")
            .replace('_', "-")
            .to_ascii_lowercase(),
        title: short_text(&title, 120),
        summary: short_text(&summary, 220),
        url,
        occurred_at,
        importance: clamp_importance(
            0.36 + recency_importance_boost(occurred_at) + text_importance_boost(&title),
        ),
    })
}

async fn fetch_gmail_items(
    ctx: &IntegrationSyncContext,
    cursor: &IntegrationSyncCursor,
) -> Result<Vec<NormalizedSyncItem>> {
    #[derive(Debug, Deserialize)]
    struct GmailListResponse {
        messages: Option<Vec<GmailMessageRef>>,
    }

    #[derive(Debug, Deserialize)]
    struct GmailMessageRef {
        id: String,
    }

    #[derive(Debug, Deserialize)]
    struct GmailMessage {
        id: String,
        #[serde(default, rename = "labelIds")]
        label_ids: Vec<String>,
        #[serde(default, rename = "internalDate")]
        internal_date: String,
        #[serde(default)]
        payload: GmailPayload,
    }

    #[derive(Debug, Default, Deserialize)]
    struct GmailPayload {
        #[serde(default)]
        headers: Vec<GmailHeader>,
    }

    #[derive(Debug, Deserialize)]
    struct GmailHeader {
        name: String,
        value: String,
    }

    fn header_value(headers: &[GmailHeader], name: &str) -> String {
        headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.clone())
            .unwrap_or_default()
    }

    let token = crate::actions::gmail::ensure_access_token(&ctx.config_dir).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut url = reqwest::Url::parse("https://gmail.googleapis.com/gmail/v1/users/me/messages")?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("maxResults", "20");
        let q = if let Some(since) = cursor.last_success_at.as_deref() {
            if let Some(ts) = parse_datetime(since) {
                format!("is:unread after:{}", ts.timestamp())
            } else {
                "is:unread newer_than:7d".to_string()
            }
        } else {
            "is:unread newer_than:7d".to_string()
        };
        query.append_pair("q", &q);
        query.append_pair("labelIds", "INBOX");
    }
    let response =
        send_integration_sync_request("Gmail list", client.get(url).bearer_auth(&token)).await?;
    if !response.status().is_success() {
        return Err(anyhow!("Gmail list failed: {}", response.status()));
    }
    let list = response.json::<GmailListResponse>().await?;
    let ids = list.messages.unwrap_or_default();
    let mut items = Vec::new();
    for message_ref in ids {
        let response = send_integration_sync_request(
            "Gmail message metadata",
            client.get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From&metadataHeaders=Date",
                message_ref.id
            ))
            .bearer_auth(&token),
        )
        .await?;
        if !response.status().is_success() {
            continue;
        }
        let message = response.json::<GmailMessage>().await?;
        let subject = header_value(&message.payload.headers, "Subject");
        let from = header_value(&message.payload.headers, "From");
        let date = header_value(&message.payload.headers, "Date");
        let occurred_at = parse_datetime(&date).or_else(|| {
            message
                .internal_date
                .parse::<i64>()
                .ok()
                .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        });
        let label_boost = if message
            .label_ids
            .iter()
            .any(|label| label.eq_ignore_ascii_case("IMPORTANT"))
        {
            0.22
        } else {
            0.0
        };
        items.push(NormalizedSyncItem {
            source_id: message.id,
            kind: "email".to_string(),
            title: if subject.trim().is_empty() {
                "Unread email".to_string()
            } else {
                subject.clone()
            },
            summary: format!("From {} | {}", short_text(&from, 70), short_text(&date, 50)),
            url: None,
            occurred_at,
            importance: clamp_importance(
                0.4 + label_boost
                    + recency_importance_boost(occurred_at)
                    + text_importance_boost(&format!("{} {}", subject, from)),
            ),
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_exists_for_supported_integrations() {
        assert!(default_config_for("gmail").is_some());
        assert!(default_config_for("github").is_some());
        assert!(default_config_for("google_places").is_none());
    }

    #[test]
    fn parse_datetime_handles_multiple_formats() {
        assert!(parse_datetime("2026-03-27T10:11:12Z").is_some());
        assert!(parse_datetime("Fri, 27 Mar 2026 10:11:12 +0000").is_some());
        assert!(parse_datetime("2026-03-27").is_some());
        assert!(parse_datetime("20260327").is_some());
    }

    #[test]
    fn recent_source_ids_are_deduped_and_capped() {
        let mut cursor = IntegrationSyncCursor {
            recent_source_ids: vec!["b".to_string(), "c".to_string()],
            ..Default::default()
        };
        push_recent_source_ids(
            &mut cursor,
            &["a".to_string(), "b".to_string(), "d".to_string()],
        );
        assert_eq!(cursor.recent_source_ids[0], "a");
        assert_eq!(cursor.recent_source_ids[1], "b");
        assert!(cursor.recent_source_ids.contains(&"c".to_string()));
        assert!(cursor.recent_source_ids.contains(&"d".to_string()));
    }

    #[tokio::test]
    async fn integration_sync_locks_are_scoped_per_integration() {
        let gmail_guard = try_acquire_integration_sync_lock("gmail")
            .await
            .expect("gmail lock should be available");
        let github_guard = try_acquire_integration_sync_lock("github")
            .await
            .expect("github lock should be independently available");

        assert!(try_acquire_integration_sync_lock("gmail").await.is_none());

        drop(github_guard);
        drop(gmail_guard);
    }
}
