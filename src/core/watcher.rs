//! Background watcher system — poll-until-condition-then-act
//!
//! Allows the agent to spawn short-lived background watchers that:
//! 1. Poll an action (e.g. gmail_scan) at a regular interval
//! 2. Check a condition against the result (e.g. "not empty", "contains keyword")
//! 3. When triggered: execute a chain of follow-up actions via the agent
//! 4. Self-terminate after trigger or timeout
//!
//! Watchers are persisted to disk so they survive container restarts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Default watch duration: 24 hours
pub const DEFAULT_TIMEOUT_SECS: u64 = 24 * 60 * 60;

/// Max allowed timeout: 9999 days
pub const MAX_TIMEOUT_SECS: u64 = 9999 * 24 * 60 * 60;

/// Cap persisted watcher payloads so a noisy poller cannot bloat state/UI.
const MAX_STORED_RESULT_CHARS: usize = 16_000;
const MAX_STORED_NOTIFICATION_MESSAGE_CHARS: usize = 8_000;
const MAX_NOTIFICATION_ATTEMPTS: usize = 12;

/// Status of a watcher
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WatcherStatus {
    /// Actively polling
    Active,
    /// Temporarily suspended by user
    Paused,
    /// Condition was met — follow-up actions queued
    Triggered,
    /// Timed out without finding a match
    TimedOut,
    /// Cancelled by user
    Cancelled,
    /// Error during polling
    Failed { error: String },
}

/// Outcome of the most recent poll attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WatcherPollOutcome {
    NoMatch,
    Matched,
    Error,
}

/// Delivery attempt made on behalf of a watcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherNotificationAttempt {
    pub attempted_at: DateTime<Utc>,
    pub channel: String,
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Condition to evaluate against poll results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchCondition {
    /// Result is not empty / not "No messages found" etc.
    NotEmpty,
    /// Result contains a keyword (case-insensitive)
    Contains { keyword: String },
    /// Result matches a regex pattern
    Matches { pattern: String },
    /// Custom condition described in natural language (evaluated by LLM)
    Custom { description: String },
}

impl WatchCondition {
    /// Evaluate the condition against a poll result
    pub fn evaluate(&self, result: &str) -> bool {
        let trimmed = result.trim();
        match self {
            WatchCondition::NotEmpty => {
                !trimmed.is_empty()
                    && !trimmed.eq_ignore_ascii_case("no messages found.")
                    && !trimmed.eq_ignore_ascii_case("no results")
                    && !trimmed.eq_ignore_ascii_case("no results found")
                    && !trimmed.starts_with("Error")
            }
            WatchCondition::Contains { keyword } => {
                trimmed.to_lowercase().contains(&keyword.to_lowercase())
            }
            WatchCondition::Matches { pattern } => regex::Regex::new(pattern)
                .map(|re| re.is_match(trimmed))
                .unwrap_or(false),
            WatchCondition::Custom { .. } => {
                // Custom conditions need LLM evaluation — treated as NotEmpty
                // for the poll loop. The agent re-evaluates with LLM after trigger.
                !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("no messages found.")
            }
        }
    }
}

/// A background watcher definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    /// Unique watcher ID
    pub id: Uuid,
    /// Human-readable description of what this watcher does
    pub description: String,
    /// Action to poll (e.g. "gmail_scan")
    pub poll_action: String,
    /// Arguments for the poll action
    pub poll_arguments: serde_json::Value,
    /// Condition that triggers the watcher
    pub condition: WatchCondition,
    /// What to do when triggered — described in natural language for the agent
    pub on_trigger: String,
    /// Polling interval in seconds (default: 60)
    pub interval_secs: u64,
    /// Maximum time to watch in seconds (default: 24 hours)
    pub timeout_secs: u64,
    /// Channel to notify when triggered or timed out
    pub notify_channel: String,
    /// Current status
    pub status: WatcherStatus,
    /// When the watcher was created
    pub created_at: DateTime<Utc>,
    /// When the watcher last polled
    pub last_poll_at: Option<DateTime<Utc>>,
    /// Number of polls executed
    pub poll_count: u32,
    /// The result that triggered the watcher (if triggered)
    pub trigger_result: Option<String>,
    /// The most recent successful poll payload, whether or not it matched.
    #[serde(default)]
    pub last_result: Option<String>,
    /// The most recent poll error, if the latest poll failed.
    #[serde(default)]
    pub last_error: Option<String>,
    /// The outcome of the latest poll attempt.
    #[serde(default)]
    pub last_poll_outcome: Option<WatcherPollOutcome>,
    /// Recent watcher-originated notification deliveries.
    #[serde(default)]
    pub notification_attempts: Vec<WatcherNotificationAttempt>,
}

/// Manages all active watchers with persistent storage
pub struct WatcherManager {
    watchers: Arc<RwLock<HashMap<Uuid, Watcher>>>,
    storage: Option<crate::storage::Storage>,
    storage_path: Option<PathBuf>,
}

fn strip_automation_meta(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (key, inner) in map {
                if key == "_automation" {
                    continue;
                }
                next.insert(key.clone(), strip_automation_meta(inner));
            }
            serde_json::Value::Object(next)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(strip_automation_meta).collect())
        }
        _ => value.clone(),
    }
}

fn normalize_signature_text(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .take(40)
        .collect::<Vec<_>>()
        .join(" ")
}

fn watcher_topic_signature(poll_arguments: &serde_json::Value, description: &str) -> String {
    let cleaned = strip_automation_meta(poll_arguments);
    let preferred = ["query", "url", "topic", "target", "app_id", "id"]
        .iter()
        .find_map(|key| cleaned.get(*key).and_then(|value| value.as_str()))
        .unwrap_or(description);
    normalize_signature_text(preferred)
}

fn normalized_topic_tokens(value: &str) -> BTreeSet<String> {
    normalize_signature_text(value)
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_string())
        .collect()
}

fn topics_are_similar(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left.contains(right) || right.contains(left) {
        return true;
    }

    let left_tokens = normalized_topic_tokens(left);
    let right_tokens = normalized_topic_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return false;
    }

    let shared = left_tokens.intersection(&right_tokens).count();
    let largest = left_tokens.len().max(right_tokens.len());
    shared >= 4 && (shared as f32 / largest as f32) >= 0.6
}

fn topics_overlap_lightly(left: &str, right: &str) -> bool {
    let left_tokens = normalized_topic_tokens(left);
    let right_tokens = normalized_topic_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return false;
    }
    left_tokens.intersection(&right_tokens).count() >= 2
}

fn watcher_origin_scope_signature(
    poll_arguments: &serde_json::Value,
) -> Option<(String, String, String)> {
    let origin = super::automation::origin_from_arguments(poll_arguments);
    let channel = origin
        .channel
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let conversation_id = origin
        .conversation_id
        .unwrap_or_default()
        .trim()
        .to_string();
    let project_id = origin.project_id.unwrap_or_default().trim().to_string();
    if channel.is_empty() && conversation_id.is_empty() && project_id.is_empty() {
        None
    } else {
        Some((channel, conversation_id, project_id))
    }
}

pub fn watcher_request_signature_from_arguments(arguments: &serde_json::Value) -> String {
    let description = arguments
        .get("description")
        .and_then(|value| value.as_str())
        .unwrap_or("background watcher");
    let poll_action = arguments
        .get("poll_action")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let notify_channel = arguments
        .get("notify_channel")
        .and_then(|value| value.as_str())
        .unwrap_or("telegram")
        .trim()
        .to_ascii_lowercase();
    let poll_arguments = arguments
        .get("poll_arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    format!(
        "{}|{}|{}",
        poll_action,
        notify_channel,
        watcher_topic_signature(&poll_arguments, description)
    )
}

pub fn watcher_tool_call_signature_from_arguments(arguments: &serde_json::Value) -> String {
    let base = watcher_request_signature_from_arguments(arguments);
    let interval_secs = arguments
        .get("interval_secs")
        .and_then(|value| value.as_u64())
        .unwrap_or(60);
    let timeout_signature = if arguments
        .get("until_stopped")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        "until_stopped".to_string()
    } else if let Some(days) = arguments
        .get("timeout_days")
        .and_then(|value| value.as_u64())
    {
        format!("days:{}", days)
    } else if let Some(hours) = arguments
        .get("timeout_hours")
        .and_then(|value| value.as_u64())
    {
        format!("hours:{}", hours)
    } else if let Some(secs) = arguments
        .get("timeout_secs")
        .and_then(|value| value.as_u64())
    {
        format!("secs:{}", secs)
    } else {
        format!("secs:{}", DEFAULT_TIMEOUT_SECS)
    };
    format!("{}|interval:{}|{}", base, interval_secs, timeout_signature)
}

impl WatcherManager {
    pub async fn new(
        data_dir: Option<&std::path::Path>,
        storage: Option<crate::storage::Storage>,
    ) -> Self {
        let storage_path = data_dir.map(|d| d.join("watchers.json"));

        // Load persisted watchers, preferring DB-backed state.
        let mut restored_from_legacy_file = false;
        let watchers = if let Some(storage_ref) = storage.as_ref() {
            match storage_ref.list_watchers().await {
                Ok(loaded) => {
                    let restored = loaded
                        .into_iter()
                        .filter(|watcher| {
                            matches!(
                                watcher.status,
                                WatcherStatus::Active | WatcherStatus::Paused
                            )
                        })
                        .map(|watcher| (watcher.id, watcher))
                        .collect::<HashMap<_, _>>();
                    if !restored.is_empty() {
                        restored
                    } else if let Some(ref path) = storage_path {
                        match std::fs::read_to_string(path) {
                            Ok(contents) => {
                                match serde_json::from_str::<HashMap<Uuid, Watcher>>(&contents) {
                                    Ok(mut legacy) => {
                                        let now = Utc::now();
                                        legacy.retain(|_, watcher| {
                                            if !matches!(
                                                watcher.status,
                                                WatcherStatus::Active | WatcherStatus::Paused
                                            ) {
                                                return false;
                                            }
                                            if watcher.status == WatcherStatus::Paused {
                                                return true;
                                            }
                                            let elapsed =
                                                (now - watcher.created_at).num_seconds() as u64;
                                            elapsed < watcher.timeout_secs
                                        });
                                        restored_from_legacy_file = !legacy.is_empty();
                                        legacy
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to parse watchers.json: {}", e);
                                        HashMap::new()
                                    }
                                }
                            }
                            Err(_) => HashMap::new(),
                        }
                    } else {
                        HashMap::new()
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load watchers from DB: {}", e);
                    HashMap::new()
                }
            }
        } else if let Some(ref path) = storage_path {
            match std::fs::read_to_string(path) {
                Ok(contents) => {
                    match serde_json::from_str::<HashMap<Uuid, Watcher>>(&contents) {
                        Ok(mut loaded) => {
                            // Only restore watchers that can continue later.
                            let now = Utc::now();
                            loaded.retain(|_, w| {
                                if !matches!(
                                    w.status,
                                    WatcherStatus::Active | WatcherStatus::Paused
                                ) {
                                    return false;
                                }
                                if w.status == WatcherStatus::Paused {
                                    return true;
                                }
                                let elapsed = (now - w.created_at).num_seconds() as u64;
                                elapsed < w.timeout_secs
                            });
                            let count = loaded.len();
                            if count > 0 {
                                tracing::info!("Restored {} active watcher(s) from disk", count);
                            }
                            restored_from_legacy_file = true;
                            loaded
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse watchers.json: {}", e);
                            HashMap::new()
                        }
                    }
                }
                Err(_) => HashMap::new(), // File doesn't exist yet
            }
        } else {
            HashMap::new()
        };

        let manager = Self {
            watchers: Arc::new(RwLock::new(watchers)),
            storage,
            storage_path,
        };

        if restored_from_legacy_file {
            manager.persist().await;
        }

        manager
    }

    /// Persist current watchers to disk
    fn save_sync(path: &std::path::Path, watchers: &HashMap<Uuid, Watcher>) {
        // Only persist Active watchers — completed ones get cleaned up
        let active: HashMap<&Uuid, &Watcher> = watchers
            .iter()
            .filter(|(_, w)| matches!(w.status, WatcherStatus::Active | WatcherStatus::Paused))
            .collect();

        if let Ok(json) = serde_json::to_string_pretty(&active) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(path, json) {
                tracing::warn!("Failed to save watchers: {}", e);
            }
        }
    }

    /// Save watchers to disk (only Active ones)
    async fn persist(&self) {
        let watchers = self.watchers.read().await;
        if let Some(storage) = self.storage.as_ref() {
            let active = watchers
                .values()
                .filter(|watcher| {
                    matches!(
                        watcher.status,
                        WatcherStatus::Active | WatcherStatus::Paused
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if let Err(e) = storage.replace_active_watchers(&active).await {
                tracing::warn!("Failed to persist watchers to DB: {}", e);
            }
        } else if let Some(ref path) = self.storage_path {
            Self::save_sync(path, &watchers);
        }
    }

    /// Add a new watcher and return its ID
    pub async fn add(&self, watcher: Watcher) -> Uuid {
        let id = watcher.id;
        self.watchers.write().await.insert(id, watcher);
        self.persist().await;
        id
    }

    pub fn semantic_signature(watcher: &Watcher) -> String {
        format!(
            "{}|{}|{}",
            watcher.poll_action.trim().to_ascii_lowercase(),
            watcher.notify_channel.trim().to_ascii_lowercase(),
            watcher_topic_signature(&watcher.poll_arguments, &watcher.description)
        )
    }

    pub fn watchers_are_semantically_similar(existing: &Watcher, candidate: &Watcher) -> bool {
        if !existing
            .poll_action
            .eq_ignore_ascii_case(candidate.poll_action.as_str())
        {
            return false;
        }
        if !existing
            .notify_channel
            .eq_ignore_ascii_case(candidate.notify_channel.as_str())
        {
            return false;
        }

        let existing_topic =
            watcher_topic_signature(&existing.poll_arguments, &existing.description);
        let candidate_topic =
            watcher_topic_signature(&candidate.poll_arguments, &candidate.description);
        if topics_are_similar(&existing_topic, &candidate_topic) {
            return true;
        }

        match (
            watcher_origin_scope_signature(&existing.poll_arguments),
            watcher_origin_scope_signature(&candidate.poll_arguments),
        ) {
            (Some(left_scope), Some(right_scope)) if left_scope == right_scope => {
                topics_overlap_lightly(&existing_topic, &candidate_topic)
            }
            _ => false,
        }
    }

    pub async fn upsert_similar(&self, watcher: Watcher) -> (Uuid, bool, usize) {
        let mut watchers = self.watchers.write().await;
        let matching_ids = watchers
            .iter()
            .filter(|(_, existing)| {
                matches!(
                    existing.status,
                    WatcherStatus::Active | WatcherStatus::Paused | WatcherStatus::Failed { .. }
                ) && Self::watchers_are_semantically_similar(existing, &watcher)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        if let Some(keeper_id) = matching_ids.first().copied() {
            if let Some(existing) = watchers.get_mut(&keeper_id) {
                existing.description = watcher.description;
                existing.poll_action = watcher.poll_action;
                existing.poll_arguments = watcher.poll_arguments;
                existing.condition = watcher.condition;
                existing.on_trigger = watcher.on_trigger;
                existing.interval_secs = watcher.interval_secs;
                existing.timeout_secs = watcher.timeout_secs;
                existing.notify_channel = watcher.notify_channel;
                existing.status = WatcherStatus::Active;
                existing.created_at = Utc::now();
                existing.last_poll_at = None;
                existing.poll_count = 0;
                existing.trigger_result = None;
                existing.last_result = None;
                existing.last_error = None;
                existing.last_poll_outcome = None;
                existing.notification_attempts.clear();
            }
            for duplicate_id in matching_ids.iter().skip(1) {
                watchers.remove(duplicate_id);
            }
            drop(watchers);
            self.persist().await;
            return (keeper_id, true, matching_ids.len().saturating_sub(1));
        }

        let id = watcher.id;
        watchers.insert(id, watcher);
        drop(watchers);
        self.persist().await;
        (id, false, 0)
    }

    /// Get all watchers
    pub async fn list(&self) -> Vec<Watcher> {
        self.watchers.read().await.values().cloned().collect()
    }

    /// Get a watcher by ID.
    pub async fn get(&self, id: Uuid) -> Option<Watcher> {
        self.watchers.read().await.get(&id).cloned()
    }

    /// Get active watchers that need polling
    pub async fn get_due_watchers(&self) -> Vec<Watcher> {
        let now = Utc::now();
        let watchers = self.watchers.read().await;
        watchers
            .values()
            .filter(|w| {
                if w.status != WatcherStatus::Active {
                    return false;
                }
                // Check timeout
                let elapsed = (now - w.created_at).num_seconds() as u64;
                if elapsed >= w.timeout_secs {
                    return false; // Will be timed out in tick()
                }
                // Check if enough time has passed since last poll
                match w.last_poll_at {
                    Some(last) => (now - last).num_seconds() as u64 >= w.interval_secs,
                    None => true, // Never polled — poll immediately
                }
            })
            .cloned()
            .collect()
    }

    /// Update a watcher after a successful poll.
    pub async fn record_poll_success(
        &self,
        id: Uuid,
        poll_count: u32,
        result: String,
        matched: bool,
    ) {
        if let Some(w) = self.watchers.write().await.get_mut(&id) {
            w.last_poll_at = Some(Utc::now());
            w.poll_count = poll_count;
            w.last_result = Some(truncate_for_storage(&result, MAX_STORED_RESULT_CHARS));
            w.last_error = None;
            w.last_poll_outcome = Some(if matched {
                WatcherPollOutcome::Matched
            } else {
                WatcherPollOutcome::NoMatch
            });
        }
        self.persist().await;
    }

    /// Update a watcher after a failed poll attempt.
    pub async fn record_poll_error(&self, id: Uuid, poll_count: u32, error: String) {
        if let Some(w) = self.watchers.write().await.get_mut(&id) {
            w.last_poll_at = Some(Utc::now());
            w.poll_count = poll_count;
            w.last_error = Some(truncate_for_storage(&error, MAX_STORED_RESULT_CHARS));
            w.last_poll_outcome = Some(WatcherPollOutcome::Error);
        }
        self.persist().await;
    }

    /// Mark a watcher as triggered
    pub async fn mark_triggered(&self, id: Uuid, result: String) {
        if let Some(w) = self.watchers.write().await.get_mut(&id) {
            w.status = WatcherStatus::Triggered;
            w.trigger_result = Some(truncate_for_storage(&result, MAX_STORED_RESULT_CHARS));
        }
        self.persist().await;
    }

    /// Record a watcher notification delivery attempt.
    pub async fn push_notification_attempt(
        &self,
        id: Uuid,
        mut attempt: WatcherNotificationAttempt,
    ) {
        if let Some(w) = self.watchers.write().await.get_mut(&id) {
            attempt.message =
                truncate_for_storage(&attempt.message, MAX_STORED_NOTIFICATION_MESSAGE_CHARS);
            attempt.error = attempt
                .error
                .as_ref()
                .map(|value| truncate_for_storage(value, MAX_STORED_RESULT_CHARS));
            w.notification_attempts.push(attempt);
            if w.notification_attempts.len() > MAX_NOTIFICATION_ATTEMPTS {
                let overflow = w.notification_attempts.len() - MAX_NOTIFICATION_ATTEMPTS;
                w.notification_attempts.drain(0..overflow);
            }
        }
        self.persist().await;
    }

    /// Mark timed-out watchers
    pub async fn expire_watchers(&self) -> Vec<Watcher> {
        let now = Utc::now();
        let mut expired = Vec::new();
        let mut watchers = self.watchers.write().await;
        for w in watchers.values_mut() {
            if w.status == WatcherStatus::Active {
                let elapsed = (now - w.created_at).num_seconds() as u64;
                if elapsed >= w.timeout_secs {
                    w.status = WatcherStatus::TimedOut;
                    expired.push(w.clone());
                }
            }
        }
        drop(watchers);
        if !expired.is_empty() {
            self.persist().await;
        }
        expired
    }

    /// Cancel a watcher by ID
    pub async fn cancel(&self, id: Uuid) -> bool {
        let cancelled = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if matches!(w.status, WatcherStatus::Active | WatcherStatus::Paused) {
                w.status = WatcherStatus::Cancelled;
                true
            } else {
                false
            }
        } else {
            false
        };
        if cancelled {
            self.persist().await;
        }
        cancelled
    }

    /// Pause an active watcher by ID
    pub async fn pause(&self, id: Uuid) -> bool {
        let paused = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if w.status == WatcherStatus::Active {
                w.status = WatcherStatus::Paused;
                true
            } else {
                false
            }
        } else {
            false
        };
        if paused {
            self.persist().await;
        }
        paused
    }

    /// Resume a paused watcher by ID
    pub async fn resume(&self, id: Uuid) -> bool {
        let resumed = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if w.status == WatcherStatus::Paused {
                w.status = WatcherStatus::Active;
                true
            } else {
                false
            }
        } else {
            false
        };
        if resumed {
            self.persist().await;
        }
        resumed
    }

    /// Delete a watcher by ID
    pub async fn delete(&self, id: Uuid) -> bool {
        let deleted = self.watchers.write().await.remove(&id).is_some();
        if deleted {
            self.persist().await;
        }
        deleted
    }

    /// Clean up completed/failed/timed-out watchers (older than 1 hour)
    pub async fn cleanup(&self) {
        let cutoff = Utc::now() - chrono::Duration::hours(1);
        let mut watchers = self.watchers.write().await;
        let before = watchers.len();
        watchers.retain(|_, w| {
            matches!(w.status, WatcherStatus::Active | WatcherStatus::Paused)
                || w.created_at > cutoff
        });
        let removed = before - watchers.len();
        drop(watchers);
        if removed > 0 {
            self.persist().await;
        }
    }
}

fn truncate_for_storage(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n\n[truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watcher_with_origin(
        description: &str,
        query: &str,
        conversation_id: &str,
        project_id: &str,
    ) -> Watcher {
        Watcher {
            id: Uuid::new_v4(),
            description: description.to_string(),
            poll_action: "web_search".to_string(),
            poll_arguments: serde_json::json!({
                "query": query,
                "_automation": {
                    "origin": {
                        "channel": "web",
                        "conversation_id": conversation_id,
                        "project_id": project_id,
                        "source": "watcher"
                    }
                }
            }),
            condition: WatchCondition::Contains {
                keyword: "breaking".to_string(),
            },
            on_trigger: "Notify me".to_string(),
            interval_secs: 60,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            notify_channel: "telegram".to_string(),
            status: WatcherStatus::Active,
            created_at: Utc::now(),
            last_poll_at: None,
            poll_count: 0,
            trigger_result: None,
            last_result: None,
            last_error: None,
            last_poll_outcome: None,
            notification_attempts: Vec::new(),
        }
    }

    #[test]
    fn watchers_are_semantically_similar_for_same_origin_fix_attempts() {
        let existing = watcher_with_origin(
            "Monitor current external updates every minute and notify on Telegram when materially important developments occur.",
            "Monitor current external updates every minute and notify on Telegram when materially important developments occur.",
            "conv-1",
            "proj-1",
        );
        let candidate = watcher_with_origin(
            "Update the existing monitor so it uses a real query and still alerts Telegram for materially important developments.",
            "Update the existing monitor so it uses a real query and still alerts Telegram for materially important developments.",
            "conv-1",
            "proj-1",
        );

        assert!(WatcherManager::watchers_are_semantically_similar(
            &existing, &candidate
        ));
    }
}
