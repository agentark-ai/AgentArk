//! Background watcher system — poll-until-condition-then-act
//!
//! Allows the agent to spawn background watchers that:
//! 1. Poll an action (e.g. gmail_scan) at a regular interval
//! 2. Check a condition against the result (e.g. "not empty", "contains keyword")
//! 3. When triggered: execute a chain of follow-up actions via the agent
//! 4. Self-terminate after trigger or timeout, unless configured to repeat
//!
//! Watchers are persisted in the database so they survive container restarts.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};
use uuid::Uuid;

/// Default watch duration: 24 hours
pub const DEFAULT_TIMEOUT_SECS: u64 = 24 * 60 * 60;

/// Max allowed timeout: 9999 days
pub const MAX_TIMEOUT_SECS: u64 = 9999 * 24 * 60 * 60;

/// Cap persisted watcher payloads so a noisy poller cannot bloat state/UI.
const MAX_STORED_RESULT_CHARS: usize = 16_000;
const MAX_STORED_NOTIFICATION_MESSAGE_CHARS: usize = 8_000;
const MAX_NOTIFICATION_ATTEMPTS: usize = 12;
const MAX_CONSECUTIVE_FAILURES_BEFORE_PAUSE: u32 = 5;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatchConditionLogic {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatchConditionEvaluationMode {
    #[default]
    CurrentState,
    Change,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchConditionOperator {
    Exists,
    NotExists,
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    NotContains,
    NonEmpty,
    Empty,
    True,
    False,
    Regex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatchJsonPredicate {
    pub path: String,
    pub operator: WatchConditionOperator,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchConditionMatcher {
    NotEmpty,
    TextContains {
        text: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    Regex {
        pattern: String,
    },
    JsonPredicate {
        path: String,
        operator: WatchConditionOperator,
        #[serde(default)]
        value: Option<serde_json::Value>,
    },
    JsonLogic {
        #[serde(default)]
        logic: WatchConditionLogic,
        rules: Vec<WatchJsonPredicate>,
    },
    Llm,
}

/// Condition contract authored by the model for a watcher.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatchCondition {
    pub description: String,
    #[serde(default)]
    pub evaluation_mode: WatchConditionEvaluationMode,
    #[serde(flatten)]
    pub matcher: WatchConditionMatcher,
}

impl WatchCondition {
    pub fn summary(&self) -> String {
        let description = self.description.trim();
        if !description.is_empty() {
            return match self.evaluation_mode {
                WatchConditionEvaluationMode::CurrentState => description.to_string(),
                WatchConditionEvaluationMode::Change => {
                    format!(
                        "{} (compare against the previous successful poll)",
                        description
                    )
                }
            };
        }

        let summary = match &self.matcher {
            WatchConditionMatcher::NotEmpty => {
                "Trigger when the poll result is not empty".to_string()
            }
            WatchConditionMatcher::TextContains { text, .. } => {
                format!("Trigger when the poll result contains \"{}\"", text)
            }
            WatchConditionMatcher::Regex { pattern } => {
                format!("Trigger when the poll result matches {}", pattern)
            }
            WatchConditionMatcher::JsonPredicate {
                path,
                operator,
                value,
            } => match value {
                Some(value) => format!(
                    "Trigger when {} {} {}",
                    if path.trim().is_empty() {
                        "$"
                    } else {
                        path.trim()
                    },
                    watch_condition_operator_label(operator),
                    value
                ),
                None => format!(
                    "Trigger when {} {}",
                    if path.trim().is_empty() {
                        "$"
                    } else {
                        path.trim()
                    },
                    watch_condition_operator_label(operator)
                ),
            },
            WatchConditionMatcher::JsonLogic { logic, rules } => format!(
                "Trigger when {} of {} structured rule(s) match",
                match logic {
                    WatchConditionLogic::All => "all",
                    WatchConditionLogic::Any => "any",
                },
                rules.len()
            ),
            WatchConditionMatcher::Llm => {
                "Trigger when the model judges the condition satisfied".to_string()
            }
        };
        match self.evaluation_mode {
            WatchConditionEvaluationMode::CurrentState => summary,
            WatchConditionEvaluationMode::Change => {
                format!("{} and differs from the previous successful poll", summary)
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.description.trim().is_empty() {
            return Err("watcher condition requires a non-empty `description`".to_string());
        }

        match &self.matcher {
            WatchConditionMatcher::NotEmpty | WatchConditionMatcher::Llm => Ok(()),
            WatchConditionMatcher::TextContains { text, .. } => {
                if text.trim().is_empty() {
                    Err("watcher text condition requires non-empty `text`".to_string())
                } else {
                    Ok(())
                }
            }
            WatchConditionMatcher::Regex { pattern } => {
                if pattern.trim().is_empty() {
                    return Err("watcher regex condition requires non-empty `pattern`".to_string());
                }
                regex::Regex::new(pattern)
                    .map(|_| ())
                    .map_err(|error| format!("watcher regex condition is invalid: {}", error))
            }
            WatchConditionMatcher::JsonPredicate {
                path,
                operator,
                value,
            } => validate_watch_json_predicate(path, operator, value),
            WatchConditionMatcher::JsonLogic { rules, .. } => {
                if rules.is_empty() {
                    return Err(
                        "watcher json_logic condition requires at least one rule".to_string()
                    );
                }
                for rule in rules {
                    validate_watch_json_predicate(&rule.path, &rule.operator, &rule.value)?;
                }
                Ok(())
            }
        }
    }
}

fn watch_condition_operator_label(operator: &WatchConditionOperator) -> &'static str {
    match operator {
        WatchConditionOperator::Exists => "exists",
        WatchConditionOperator::NotExists => "not_exists",
        WatchConditionOperator::Eq => "eq",
        WatchConditionOperator::Ne => "ne",
        WatchConditionOperator::Gt => "gt",
        WatchConditionOperator::Gte => "gte",
        WatchConditionOperator::Lt => "lt",
        WatchConditionOperator::Lte => "lte",
        WatchConditionOperator::Contains => "contains",
        WatchConditionOperator::NotContains => "not_contains",
        WatchConditionOperator::NonEmpty => "non_empty",
        WatchConditionOperator::Empty => "empty",
        WatchConditionOperator::True => "true",
        WatchConditionOperator::False => "false",
        WatchConditionOperator::Regex => "regex",
    }
}

fn watch_operator_requires_value(operator: &WatchConditionOperator) -> bool {
    matches!(
        operator,
        WatchConditionOperator::Eq
            | WatchConditionOperator::Ne
            | WatchConditionOperator::Gt
            | WatchConditionOperator::Gte
            | WatchConditionOperator::Lt
            | WatchConditionOperator::Lte
            | WatchConditionOperator::Contains
            | WatchConditionOperator::NotContains
            | WatchConditionOperator::Regex
    )
}

fn validate_watch_json_predicate(
    path: &str,
    operator: &WatchConditionOperator,
    value: &Option<serde_json::Value>,
) -> Result<(), String> {
    if path.trim().is_empty()
        && !matches!(
            operator,
            WatchConditionOperator::Exists
                | WatchConditionOperator::NotExists
                | WatchConditionOperator::NonEmpty
                | WatchConditionOperator::Empty
        )
    {
        return Err("watcher json predicate requires a non-empty `path`".to_string());
    }

    if watch_operator_requires_value(operator) && value.is_none() {
        return Err(format!(
            "watcher json predicate operator {:?} requires a `value`",
            operator
        ));
    }

    if matches!(operator, WatchConditionOperator::Regex) {
        let pattern = value
            .as_ref()
            .and_then(|value| value.as_str())
            .ok_or_else(|| "watcher regex predicate requires string `value`".to_string())?;
        regex::Regex::new(pattern)
            .map(|_| ())
            .map_err(|error| format!("watcher regex predicate is invalid: {}", error))?;
    }

    Ok(())
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
    /// Keep polling after a matched condition and send a notification for each match.
    #[serde(default)]
    pub repeat_on_match: bool,
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
    /// Consecutive poll failures, used for backoff and automatic pausing.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Earliest instant when the watcher is allowed to poll again.
    #[serde(default)]
    pub next_poll_not_before: Option<DateTime<Utc>>,
    /// The outcome of the latest poll attempt.
    #[serde(default)]
    pub last_poll_outcome: Option<WatcherPollOutcome>,
    /// Recent watcher-originated notification deliveries.
    #[serde(default)]
    pub notification_attempts: Vec<WatcherNotificationAttempt>,
}

/// Manages all active watchers with persistent storage
#[derive(Clone)]
pub struct WatcherManager {
    watchers: Arc<RwLock<HashMap<Uuid, Watcher>>>,
    storage: Option<crate::storage::Storage>,
    change_notify: Arc<Notify>,
}

fn add_secs(base: DateTime<Utc>, secs: u64, fallback: DateTime<Utc>) -> DateTime<Utc> {
    base.checked_add_signed(chrono::Duration::seconds(secs.min(i64::MAX as u64) as i64))
        .unwrap_or(fallback)
}

fn watcher_next_wakeup_at(watcher: &Watcher, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if watcher.status != WatcherStatus::Active {
        return None;
    }

    let timeout_at = add_secs(watcher.created_at, watcher.timeout_secs, now);
    if timeout_at <= now {
        return Some(now);
    }

    let interval_due_at = watcher
        .last_poll_at
        .map(|last| add_secs(last, watcher.interval_secs, timeout_at))
        .unwrap_or(now);
    let poll_due_at = watcher
        .next_poll_not_before
        .map(|not_before| interval_due_at.max(not_before))
        .unwrap_or(interval_due_at);

    Some(poll_due_at.min(timeout_at))
}

fn watcher_retention_reference_at(watcher: &Watcher) -> DateTime<Utc> {
    let mut latest = watcher.created_at;
    if let Some(last_poll_at) = watcher.last_poll_at {
        latest = latest.max(last_poll_at);
    }
    for attempt in &watcher.notification_attempts {
        latest = latest.max(attempt.attempted_at);
    }
    latest
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
    super::task::normalize_signature_text(value)
}

fn watcher_topic_signature(poll_arguments: &serde_json::Value, description: &str) -> String {
    let cleaned = strip_automation_meta(poll_arguments);
    let arguments_signature = super::task::canonical_signature_value(&cleaned);
    let description_signature = normalize_signature_text(description);
    match (arguments_signature.as_str(), description_signature.as_str()) {
        ("{}" | "null", description) => description.to_string(),
        (arguments, "") => arguments.to_string(),
        (arguments, description) => format!("{}|{}", description, arguments),
    }
}

fn watcher_origin_scope_signature(
    poll_arguments: &serde_json::Value,
) -> Option<(String, String, String)> {
    let origin = crate::core::automation::origin_from_arguments(poll_arguments);
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
    let poll_arguments = arguments
        .get("poll_arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    format!(
        "{}|{}",
        poll_action,
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
        _data_dir: Option<&std::path::Path>,
        storage: Option<crate::storage::Storage>,
    ) -> Self {
        let watchers = if let Some(storage_ref) = storage.as_ref() {
            match storage_ref.list_watchers().await {
                Ok(loaded) => loaded
                    .into_iter()
                    .map(|watcher| (watcher.id, watcher))
                    .collect::<HashMap<_, _>>(),
                Err(e) => {
                    tracing::warn!("Failed to load watchers from DB: {}", e);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            watchers: Arc::new(RwLock::new(watchers)),
            storage,
            change_notify: Arc::new(Notify::new()),
        }
    }

    fn notify_changed(&self) {
        self.change_notify.notify_one();
    }

    #[cfg(test)]
    /// Legacy no-op file persistence helper retained only until fresh-install cleanup finishes.
    fn save_sync(_path: &std::path::Path, _watchers: &HashMap<Uuid, Watcher>) {
        // Only persist Active watchers — completed ones get cleaned up
        // Fresh-install-only builds do not persist watcher state to files.
    }

    /// Persist the in-memory watcher snapshot to the database.
    async fn persist(&self) {
        let watchers = {
            let watchers = self.watchers.read().await;
            watchers.values().cloned().collect::<Vec<_>>()
        };
        if let Some(storage) = self.storage.as_ref() {
            for watcher in watchers {
                if let Err(e) = storage.upsert_watcher(&watcher).await {
                    tracing::warn!("Failed to persist watcher '{}' to DB: {}", watcher.id, e);
                }
            }
        }
    }

    async fn persist_one(&self, id: Uuid) {
        let watcher = self.watchers.read().await.get(&id).cloned();
        self.persist_watcher_snapshot(id, watcher).await;
    }

    async fn persist_watcher_snapshot(&self, id: Uuid, watcher: Option<Watcher>) {
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        match watcher {
            Some(watcher) => {
                if let Err(error) = storage.upsert_watcher(&watcher).await {
                    tracing::warn!("Failed to persist watcher '{}' to DB: {}", id, error);
                }
            }
            _ => {
                if let Err(error) = storage.delete_watcher(&id.to_string()).await {
                    tracing::warn!("Failed to delete watcher '{}' from DB: {}", id, error);
                }
            }
        }
    }

    async fn delete_persisted(&self, id: Uuid) {
        self.persist_watcher_snapshot(id, None).await;
    }

    /// Add a new watcher and return its ID
    pub async fn add(&self, watcher: Watcher) -> Uuid {
        let id = watcher.id;
        self.watchers.write().await.insert(id, watcher);
        self.persist_one(id).await;
        self.notify_changed();
        id
    }

    pub fn watchers_are_semantically_similar(existing: &Watcher, candidate: &Watcher) -> bool {
        if !existing
            .poll_action
            .eq_ignore_ascii_case(candidate.poll_action.as_str())
        {
            return false;
        }

        let existing_topic =
            watcher_topic_signature(&existing.poll_arguments, &existing.description);
        let candidate_topic =
            watcher_topic_signature(&candidate.poll_arguments, &candidate.description);
        existing_topic == candidate_topic
            && watcher_origin_scope_signature(&existing.poll_arguments)
                == watcher_origin_scope_signature(&candidate.poll_arguments)
    }

    fn replace_watcher(existing: &mut Watcher, watcher: Watcher) {
        existing.description = watcher.description;
        existing.poll_action = watcher.poll_action;
        existing.poll_arguments = watcher.poll_arguments;
        existing.condition = watcher.condition;
        existing.on_trigger = watcher.on_trigger;
        existing.interval_secs = watcher.interval_secs;
        existing.timeout_secs = watcher.timeout_secs;
        existing.notify_channel = watcher.notify_channel;
        existing.repeat_on_match = watcher.repeat_on_match;
        existing.status = WatcherStatus::Active;
        existing.created_at = Utc::now();
        existing.last_poll_at = None;
        existing.poll_count = 0;
        existing.trigger_result = None;
        existing.last_result = None;
        existing.last_error = None;
        existing.consecutive_failures = 0;
        existing.next_poll_not_before = None;
        existing.last_poll_outcome = None;
        existing.notification_attempts.clear();
    }

    pub async fn upsert_similar(
        &self,
        watcher: Watcher,
        target_watcher_id: Option<Uuid>,
    ) -> Result<(Uuid, bool, usize), String> {
        let mut watchers = self.watchers.write().await;
        if let Some(target_id) = target_watcher_id {
            let existing = watchers.get_mut(&target_id).ok_or_else(|| {
                format!(
                    "Watcher `{}` was not found. Use `list_watchers` to choose an active watcher.",
                    target_id
                )
            })?;
            Self::replace_watcher(existing, watcher);
            drop(watchers);
            self.persist_one(target_id).await;
            self.notify_changed();
            return Ok((target_id, true, 0));
        }

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
                Self::replace_watcher(existing, watcher);
            }
            for duplicate_id in matching_ids.iter().skip(1) {
                watchers.remove(duplicate_id);
            }
            drop(watchers);
            self.persist_one(keeper_id).await;
            for duplicate_id in matching_ids.iter().skip(1) {
                self.delete_persisted(*duplicate_id).await;
            }
            self.notify_changed();
            return Ok((keeper_id, true, matching_ids.len().saturating_sub(1)));
        }

        let id = watcher.id;
        watchers.insert(id, watcher);
        drop(watchers);
        self.persist_one(id).await;
        self.notify_changed();
        Ok((id, false, 0))
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
                if let Some(not_before) = w.next_poll_not_before {
                    if not_before > now {
                        return false;
                    }
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

    /// Earliest poll, retry, or timeout event for active watchers.
    pub async fn next_wakeup_at(&self) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        let watchers = self.watchers.read().await;
        watchers
            .values()
            .filter_map(|watcher| watcher_next_wakeup_at(watcher, now))
            .min()
    }

    /// Wait until watcher definitions or scheduling state changes.
    pub async fn wait_for_change(&self) {
        self.change_notify.notified().await;
    }

    /// Mark a watcher poll as started before external work so cancellation or
    /// process restart does not leave it immediately due again.
    pub async fn begin_poll(&self, id: Uuid) -> Option<Watcher> {
        let started = {
            let mut watchers = self.watchers.write().await;
            let watcher = watchers.get_mut(&id)?;
            if watcher.status != WatcherStatus::Active {
                return None;
            }
            let now = Utc::now();
            watcher.last_poll_at = Some(now);
            watcher.poll_count = watcher.poll_count.saturating_add(1);
            watcher.last_poll_outcome = None;
            watcher.clone()
        };
        self.persist_one(id).await;
        Some(started)
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
            w.consecutive_failures = 0;
            w.next_poll_not_before = None;
            w.last_poll_outcome = Some(if matched {
                WatcherPollOutcome::Matched
            } else {
                WatcherPollOutcome::NoMatch
            });
        }
        self.persist_one(id).await;
    }

    /// Update a watcher after a failed poll attempt.
    pub async fn record_poll_error(&self, id: Uuid, poll_count: u32, error: String) {
        if let Some(w) = self.watchers.write().await.get_mut(&id) {
            let now = Utc::now();
            w.last_poll_at = Some(now);
            w.poll_count = poll_count;
            w.consecutive_failures = w.consecutive_failures.saturating_add(1);
            let mut error_text = truncate_for_storage(&error, MAX_STORED_RESULT_CHARS);
            let transient_error = watcher_poll_error_is_transient(&error_text);
            if w.consecutive_failures >= MAX_CONSECUTIVE_FAILURES_BEFORE_PAUSE && !transient_error {
                w.status = WatcherStatus::Paused;
                w.next_poll_not_before = None;
                error_text = truncate_for_storage(
                    &format!(
                        "{} Polling paused after {} consecutive failures.",
                        error_text, w.consecutive_failures
                    ),
                    MAX_STORED_RESULT_CHARS,
                );
            } else {
                let backoff_secs =
                    watcher_failure_backoff_secs(w.interval_secs, w.consecutive_failures);
                w.next_poll_not_before = Some(now + chrono::Duration::seconds(backoff_secs as i64));
                if transient_error {
                    error_text = truncate_for_storage(
                        &format!(
                            "{} Transient availability error detected; watcher will retry with backoff.",
                            error_text
                        ),
                        MAX_STORED_RESULT_CHARS,
                    );
                }
            }
            w.last_error = Some(error_text);
            w.last_poll_outcome = Some(WatcherPollOutcome::Error);
        }
        self.persist_one(id).await;
    }

    /// Mark a watcher as triggered
    pub async fn mark_triggered(&self, id: Uuid, result: String) {
        if let Some(w) = self.watchers.write().await.get_mut(&id) {
            w.trigger_result = Some(truncate_for_storage(&result, MAX_STORED_RESULT_CHARS));
            if !w.repeat_on_match {
                w.status = WatcherStatus::Triggered;
            }
        }
        self.persist_one(id).await;
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
        self.persist_one(id).await;
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
            for watcher in &expired {
                self.persist_one(watcher.id).await;
            }
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
            self.persist_one(id).await;
            self.notify_changed();
        }
        cancelled
    }

    /// Pause an active watcher by ID
    pub async fn pause(&self, id: Uuid) -> bool {
        let paused = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if w.status == WatcherStatus::Active {
                w.status = WatcherStatus::Paused;
                w.next_poll_not_before = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if paused {
            self.persist_one(id).await;
            self.notify_changed();
        }
        paused
    }

    /// Resume a paused watcher by ID
    pub async fn resume(&self, id: Uuid) -> bool {
        let resumed = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if w.status == WatcherStatus::Paused {
                w.status = WatcherStatus::Active;
                w.consecutive_failures = 0;
                w.next_poll_not_before = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if resumed {
            self.persist_one(id).await;
            self.notify_changed();
        }
        resumed
    }

    /// Pause all active watchers and return how many changed state.
    pub async fn pause_all(&self) -> usize {
        let mut changed = 0usize;
        {
            let mut watchers = self.watchers.write().await;
            for watcher in watchers.values_mut() {
                if watcher.status == WatcherStatus::Active {
                    watcher.status = WatcherStatus::Paused;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.persist().await;
            self.notify_changed();
        }
        changed
    }

    /// Resume all paused watchers and return how many changed state.
    pub async fn resume_all(&self) -> usize {
        let mut changed = 0usize;
        {
            let mut watchers = self.watchers.write().await;
            for watcher in watchers.values_mut() {
                if watcher.status == WatcherStatus::Paused {
                    watcher.status = WatcherStatus::Active;
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.persist().await;
            self.notify_changed();
        }
        changed
    }

    /// Mark a watcher due for the next Sentinel watcher tick.
    pub async fn run_now(&self, id: Uuid) -> bool {
        let ran = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if w.status == WatcherStatus::Active {
                w.last_poll_at = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if ran {
            self.persist_one(id).await;
            self.notify_changed();
        }
        ran
    }

    /// Extend a watcher lifetime by extra seconds, clamped to the global max.
    pub async fn extend_timeout(&self, id: Uuid, extra_secs: u64) -> Option<u64> {
        if extra_secs == 0 {
            return self.get(id).await.map(|watcher| watcher.timeout_secs);
        }
        let updated = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if matches!(w.status, WatcherStatus::Active | WatcherStatus::Paused) {
                w.timeout_secs = w
                    .timeout_secs
                    .saturating_add(extra_secs)
                    .min(MAX_TIMEOUT_SECS);
                Some(w.timeout_secs)
            } else {
                None
            }
        } else {
            None
        };
        if updated.is_some() {
            self.persist_one(id).await;
            self.notify_changed();
        }
        updated
    }

    /// Set a watcher lifetime to effectively indefinite by pinning it to the max timeout.
    pub async fn extend_until_stopped(&self, id: Uuid) -> Option<u64> {
        let updated = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if matches!(w.status, WatcherStatus::Active | WatcherStatus::Paused) {
                w.timeout_secs = MAX_TIMEOUT_SECS;
                Some(w.timeout_secs)
            } else {
                None
            }
        } else {
            None
        };
        if updated.is_some() {
            self.persist_one(id).await;
            self.notify_changed();
        }
        updated
    }

    /// Rebind a watcher to a different background session without changing its runtime behavior.
    pub async fn set_background_session_id(&self, id: Uuid, session_id: Option<&str>) -> bool {
        let updated = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            w.poll_arguments = super::background_session::set_background_session_id_in_automation(
                &w.poll_arguments,
                session_id,
            );
            true
        } else {
            false
        };
        if updated {
            self.persist_one(id).await;
            self.notify_changed();
        }
        updated
    }

    /// Rebind a watcher's notification route without changing its poll target.
    pub async fn set_notify_channel(&self, id: Uuid, notify_channel: &str) -> bool {
        let normalized = notify_channel.trim();
        if normalized.is_empty() {
            return false;
        }
        let updated = if let Some(w) = self.watchers.write().await.get_mut(&id) {
            if w.notify_channel == normalized {
                false
            } else {
                w.notify_channel = normalized.to_string();
                true
            }
        } else {
            false
        };
        if updated {
            self.persist_one(id).await;
            self.notify_changed();
        }
        updated
    }

    /// Delete a watcher by ID
    pub async fn delete(&self, id: Uuid) -> bool {
        let deleted = self.watchers.write().await.remove(&id).is_some();
        if deleted {
            self.delete_persisted(id).await;
            self.notify_changed();
        }
        deleted
    }

    /// Clean up completed/failed/timed-out watchers (older than 1 hour)
    pub async fn cleanup(&self) {
        let cutoff = Utc::now() - chrono::Duration::hours(1);
        let removed_ids = {
            let mut watchers = self.watchers.write().await;
            let removed_ids = watchers
                .iter()
                .filter_map(|(id, watcher)| {
                    (!matches!(
                        watcher.status,
                        WatcherStatus::Active | WatcherStatus::Paused
                    ) && watcher_retention_reference_at(watcher) <= cutoff)
                        .then_some(*id)
                })
                .collect::<Vec<_>>();
            for id in &removed_ids {
                watchers.remove(id);
            }
            removed_ids
        };
        for id in removed_ids {
            self.delete_persisted(id).await;
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

fn watcher_failure_backoff_secs(interval_secs: u64, consecutive_failures: u32) -> u64 {
    let multiplier = 2u64.saturating_pow(consecutive_failures.saturating_sub(1).min(6));
    interval_secs
        .max(30)
        .saturating_mul(multiplier)
        .min(60 * 60)
}

fn watcher_poll_error_is_transient(error: &str) -> bool {
    let haystack = error.trim().to_ascii_lowercase();
    !haystack.is_empty()
        && [
            "timeout",
            "temporar",
            "rate limit",
            "429",
            "unavailable",
            "busy",
            "pending",
            "connection reset",
            "connection refused",
            "network",
            "retry later",
            "gateway",
            "refused",
        ]
        .iter()
        .any(|token| haystack.contains(token))
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
            condition: WatchCondition {
                description: "Trigger when results contain breaking".to_string(),
                evaluation_mode: WatchConditionEvaluationMode::CurrentState,
                matcher: WatchConditionMatcher::TextContains {
                    text: "breaking".to_string(),
                    case_sensitive: false,
                },
            },
            on_trigger: "Notify me".to_string(),
            interval_secs: 60,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            notify_channel: "telegram".to_string(),
            repeat_on_match: false,
            status: WatcherStatus::Active,
            created_at: Utc::now(),
            last_poll_at: None,
            poll_count: 0,
            trigger_result: None,
            last_result: None,
            last_error: None,
            consecutive_failures: 0,
            next_poll_not_before: None,
            last_poll_outcome: None,
            notification_attempts: Vec::new(),
        }
    }

    #[test]
    fn watcher_similarity_requires_exact_structural_signature() {
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
        let exact = watcher_with_origin(
            "Monitor current external updates every minute and notify on Telegram when materially important developments occur.",
            "Monitor current external updates every minute and notify on Telegram when materially important developments occur.",
            "conv-1",
            "proj-1",
        );

        assert!(!WatcherManager::watchers_are_semantically_similar(
            &existing, &candidate
        ));
        assert!(WatcherManager::watchers_are_semantically_similar(
            &existing, &exact
        ));
    }

    #[test]
    fn next_wakeup_is_now_for_never_polled_active_watcher() {
        let now = Utc::now();
        let mut watcher = watcher_with_origin("watch", "query", "conv", "proj");
        watcher.created_at = now;
        watcher.last_poll_at = None;

        assert_eq!(watcher_next_wakeup_at(&watcher, now), Some(now));
    }

    #[test]
    fn next_wakeup_honors_backoff_after_interval() {
        let now = Utc::now();
        let mut watcher = watcher_with_origin("watch", "query", "conv", "proj");
        watcher.created_at = now - chrono::Duration::minutes(10);
        watcher.last_poll_at = Some(now - chrono::Duration::seconds(30));
        watcher.interval_secs = 60;
        watcher.next_poll_not_before = Some(now + chrono::Duration::seconds(120));

        assert_eq!(
            watcher_next_wakeup_at(&watcher, now),
            Some(now + chrono::Duration::seconds(120))
        );
    }

    #[test]
    fn next_wakeup_uses_timeout_when_timeout_is_earliest() {
        let now = Utc::now();
        let mut watcher = watcher_with_origin("watch", "query", "conv", "proj");
        watcher.created_at = now - chrono::Duration::seconds(90);
        watcher.timeout_secs = 100;
        watcher.last_poll_at = Some(now);
        watcher.interval_secs = 60;

        assert_eq!(
            watcher_next_wakeup_at(&watcher, now),
            Some(now + chrono::Duration::seconds(10))
        );
    }

    #[test]
    fn next_wakeup_ignores_paused_watchers() {
        let now = Utc::now();
        let mut watcher = watcher_with_origin("watch", "query", "conv", "proj");
        watcher.status = WatcherStatus::Paused;

        assert_eq!(watcher_next_wakeup_at(&watcher, now), None);
    }

    #[test]
    fn legacy_save_sync_noop_is_still_callable_in_tests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watchers.json");
        WatcherManager::save_sync(&path, &HashMap::new());
        assert!(!path.exists());
    }
}
