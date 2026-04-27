use super::*;

pub(super) const AUTONOMY_LAST_BRIEF_KEY: &str = "autonomy_last_brief_v1";
pub(super) const AUTONOMY_ANALYSIS_LAST_RUN_KEY: &str = "autonomy_analysis_last_run_v1";
pub(super) const AUTONOMY_ATTENTION_STATE_KEY: &str = "autonomy_attention_state_v1";
pub(super) const AUTONOMY_CHAT_SUGGESTIONS_KEY: &str = "autonomy_chat_suggestions_v1";
pub(super) const AUTONOMY_CHAT_SUGGESTION_SCAN_STATE_KEY: &str =
    "autonomy_chat_suggestion_scan_state_v1";
pub(super) const DAILY_BRIEF_ENABLED_KEY: &str = "daily_brief_enabled";
pub(super) const DAILY_BRIEF_TIME_KEY: &str = "daily_brief_time";
pub(super) const DAILY_BRIEF_CHANNEL_KEY: &str = "daily_brief_channel";
pub(super) const DEFAULT_DAILY_BRIEF_TIME: &str = "09:00";
pub(super) const PUBLIC_SELECTED_APP_KEY: &str = "public_selected_app_id";
pub(super) const ROUTING_POLICY_LINEAGE_REL_PATH: &str =
    ".agentark/self_evolve/routing_policy_lineage.jsonl";
pub(super) const PROMPT_BUNDLE_LINEAGE_REL_PATH: &str =
    ".agentark/self_evolve/prompt_bundle_lineage.jsonl";
pub(super) const CLASSIFIER_PROMPT_BUNDLE_LINEAGE_REL_PATH: &str =
    ".agentark/self_evolve/classifier_prompt_bundle_lineage.jsonl";
pub(super) const SPECIALIST_PROMPT_BUNDLE_LINEAGE_REL_PATH: &str =
    ".agentark/self_evolve/specialist_prompt_bundle_lineage.jsonl";
pub(super) const PROMPT_REPLAY_EVAL_SAMPLE_LIMIT: u64 = 5000;
pub(super) const EVOLUTION_DEV_DEFAULT_LIMIT: u64 = 250;
pub(super) const EVOLUTION_DEV_MAX_LIMIT: u64 = 500;
pub(super) const EVOLUTION_DEV_RECENT_RUN_RESPONSE_LIMIT: usize = 250;
pub(super) const PROMPT_OPTIMIZATION_REVIEW_STATE_KEY: &str = "prompt_optimization_review_state_v1";
pub(super) const CHAT_SUGGESTION_SCAN_INTERVAL_HOURS: i64 = 12;
pub(super) const CHAT_SUGGESTION_SCAN_DEFER_MINUTES: i64 = 30;
pub(super) const CHAT_SUGGESTION_SCAN_FETCH_LIMIT: u64 = 48;
pub(super) const CHAT_SUGGESTION_SCAN_BATCH_LIMIT: usize = 12;
pub(super) const CHAT_SUGGESTION_RECENT_MESSAGES_PER_CHAT: usize = 8;
pub(super) const CHAT_SUGGESTION_OPEN_LIMIT: usize = 24;
pub(super) const CHAT_SUGGESTION_RETAINED_HISTORY: usize = 80;
pub(super) const CHAT_SUGGESTION_RETAINED_WATERMARKS: usize = 512;
const CHAT_SUGGESTION_SEMANTIC_DEDUP_TIMEOUT_SECS: u64 = 8;
pub(super) const OPTIONAL_BACKGROUND_POLL_SECS: u64 = 60;
// Supervision budget for optional HTTP-side maintenance loops only.
// This does not apply to foreground chat/task execution.
pub(super) const OPTIONAL_BACKGROUND_JOB_TIMEOUT_SECS: u64 = 90;
pub(super) const OPTIONAL_BACKGROUND_MAX_TIMEOUT_BACKOFF_SECS: u64 = 15 * 60;

pub(super) fn parse_bool_pref(raw: Option<Vec<u8>>) -> bool {
    raw.and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub(super) fn normalize_daily_brief_time(value: &str) -> Option<String> {
    let parsed = chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M").ok()?;
    Some(format!("{:02}:{:02}", parsed.hour(), parsed.minute()))
}

pub(super) fn notification_represents_missing_input(
    notification: &crate::storage::entities::notification::Model,
) -> bool {
    let source = notification.source.to_ascii_lowercase();
    if source == "autonomy_attention" || source.contains("approval") {
        return false;
    }
    let title = notification.title.to_ascii_lowercase();
    let body = notification.body.to_ascii_lowercase();
    source == "workflow_inputs"
        || title.contains("missing input")
        || body.contains("missing input")
        || title.contains("required input")
        || body.contains("required input")
}

pub(super) fn daily_brief_time_from_cron(cron: &str) -> Option<String> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    let (hour_raw, minute_raw) = match parts.as_slice() {
        [_, minute, hour, _, _, _] => (*hour, *minute),
        [minute, hour, _, _, _] => (*hour, *minute),
        _ => return None,
    };
    let hour = hour_raw.parse::<u32>().ok()?;
    let minute = minute_raw.parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{:02}:{:02}", hour, minute))
}

pub(super) fn daily_brief_cron_from_time(value: &str) -> Option<String> {
    let normalized = normalize_daily_brief_time(value)?;
    let (hour_raw, minute_raw) = normalized.split_once(':')?;
    let hour = hour_raw.parse::<u32>().ok()?;
    let minute = minute_raw.parse::<u32>().ok()?;
    Some(format!("0 {} {} * * *", minute, hour))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AutonomyBriefingResponse {
    pub(super) generated_at: String,
    pub(super) scope: String,
    pub(super) top_risks: Vec<serde_json::Value>,
    pub(super) top_opportunities: Vec<serde_json::Value>,
    pub(super) recommended_actions: Vec<RecommendedAction>,
    pub(super) trust_summary: serde_json::Value,
    pub(super) suggested_automations: Vec<ChatAutomationSuggestion>,
    pub(super) suggestion_scan: ChatSuggestionScanState,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AutonomyExecuteActionRequest {
    pub(super) action: RecommendedAction,
    #[serde(default)]
    pub(super) dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ChatSuggestionConversationWatermark {
    pub(super) conversation_id: String,
    pub(super) last_scanned_updated_at: String,
    #[serde(default)]
    pub(super) last_user_message_id: Option<String>,
    #[serde(default)]
    pub(super) last_user_message_at: Option<String>,
    pub(super) scanned_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ChatSuggestionScanState {
    #[serde(default)]
    pub(super) last_started_at: Option<String>,
    #[serde(default)]
    pub(super) last_completed_at: Option<String>,
    #[serde(default)]
    pub(super) next_due_at: Option<String>,
    #[serde(default)]
    pub(super) last_status: Option<String>,
    #[serde(default)]
    pub(super) last_error: Option<String>,
    #[serde(default)]
    pub(super) defer_count: u32,
    #[serde(default)]
    pub(super) cursor_updated_at: Option<String>,
    #[serde(default)]
    pub(super) cursor_conversation_id: Option<String>,
    #[serde(default)]
    pub(super) last_examined_chats: usize,
    #[serde(default)]
    pub(super) last_created_suggestions: usize,
    #[serde(default)]
    pub(super) last_low_signal_skips: usize,
    #[serde(default)]
    pub(super) last_artifact_skips: usize,
    #[serde(default)]
    pub(super) last_backlog_hint: usize,
    #[serde(default)]
    pub(super) tracked_chats: usize,
    #[serde(default)]
    pub(super) conversation_watermarks: Vec<ChatSuggestionConversationWatermark>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ChatAutomationSuggestion {
    pub(super) id: String,
    pub(super) status: String,
    pub(super) kind: String,
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) rationale: String,
    pub(super) confidence: f32,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) conversation_id: String,
    pub(super) conversation_title: String,
    pub(super) conversation_channel: String,
    pub(super) source_message_id: String,
    pub(super) source_snippet: String,
    pub(super) fingerprint: String,
    pub(super) goal_title: String,
    #[serde(default)]
    pub(super) goal_detail: Option<String>,
    #[serde(default)]
    pub(super) accepted_goal_id: Option<String>,
    #[serde(default)]
    pub(super) dismissed_at: Option<String>,
    #[serde(default)]
    pub(super) accepted_at: Option<String>,
    #[serde(default)]
    pub(super) accepted_trace_id: Option<String>,
    #[serde(default)]
    pub(super) run_status: Option<String>,
    #[serde(default)]
    pub(super) last_run_error: Option<String>,
    #[serde(default)]
    pub(super) last_run_started_at: Option<String>,
    #[serde(default)]
    pub(super) last_run_completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) accepted_outcomes: Vec<suggestions::ChatSuggestionOutcome>,
}

pub(super) fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

pub(super) fn parse_utc_rfc3339(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    parse_rfc3339_utc(value)
}

pub(super) async fn sleep_or_http_shutdown(
    duration: std::time::Duration,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = shutdown_rx.changed() => false,
        _ = tokio::time::sleep(duration) => true,
    }
}

pub(super) fn next_background_sleep_duration(
    due_at: Option<chrono::DateTime<chrono::Utc>>,
) -> std::time::Duration {
    let poll = std::time::Duration::from_secs(OPTIONAL_BACKGROUND_POLL_SECS);
    let Some(due_at) = due_at else {
        return poll;
    };
    let now = chrono::Utc::now();
    if due_at <= now {
        return std::time::Duration::from_secs(0);
    }
    (due_at - now).to_std().unwrap_or(poll).min(poll)
}

pub(super) fn optional_background_timeout_backoff(timeout_streak: u32) -> std::time::Duration {
    let exponent = timeout_streak.saturating_sub(1).min(4);
    let multiplier = 1u64 << exponent;
    std::time::Duration::from_secs(
        OPTIONAL_BACKGROUND_POLL_SECS
            .saturating_mul(multiplier)
            .min(OPTIONAL_BACKGROUND_MAX_TIMEOUT_BACKOFF_SECS),
    )
}

pub(super) async fn load_autonomy_settings_from_storage(
    storage: &crate::storage::Storage,
) -> AutonomySettings {
    let mut settings = storage
        .get(crate::core::AUTONOMY_SETTINGS_STORAGE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<AutonomySettings>(&raw).ok())
        .unwrap_or_default();
    settings.enforce_dependencies();
    settings
}

pub(super) async fn save_autonomy_settings_to_storage(
    storage: &crate::storage::Storage,
    settings: &AutonomySettings,
) -> std::result::Result<(), String> {
    let mut settings = settings.clone();
    settings.enforce_dependencies();
    let raw = serde_json::to_vec(&settings).map_err(|e| e.to_string())?;
    storage
        .set(crate::core::AUTONOMY_SETTINGS_STORAGE_KEY, &raw)
        .await
        .map_err(|e| e.to_string())?;

    if crate::core::autonomy::autonomy_background_paused(&settings) {
        let paused_since_missing = match storage
            .get(crate::core::autonomy::AUTONOMY_PAUSED_SINCE_KEY)
            .await
        {
            Ok(Some(raw)) => String::from_utf8(raw)
                .ok()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true),
            Ok(None) => true,
            Err(error) => {
                tracing::debug!(
                    "Failed to read autonomy pause state while saving settings: {}",
                    error
                );
                true
            }
        };
        if paused_since_missing {
            let now = chrono::Utc::now().timestamp().to_string();
            if let Err(error) = storage
                .set(
                    crate::core::autonomy::AUTONOMY_PAUSED_SINCE_KEY,
                    now.as_bytes(),
                )
                .await
            {
                tracing::debug!(
                    "Failed to persist autonomy pause start while saving settings: {}",
                    error
                );
            }
        }
    } else {
        let _ = storage
            .delete(crate::core::autonomy::AUTONOMY_PAUSED_SINCE_KEY)
            .await;
        let _ = storage
            .delete(crate::core::autonomy::AUTONOMY_PAUSE_NUDGE_LAST_SENT_AT_KEY)
            .await;
    }

    Ok(())
}

pub(super) fn normalize_chat_suggestion_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn chat_suggestion_due_at(now: chrono::DateTime<chrono::Utc>) -> String {
    (now + chrono::Duration::hours(CHAT_SUGGESTION_SCAN_INTERVAL_HOURS)).to_rfc3339()
}

pub(super) fn chat_suggestion_deferred_due_at(
    now: chrono::DateTime<chrono::Utc>,
    defer_count: u32,
) -> String {
    let steps = defer_count.saturating_sub(1) as i64;
    let delay_minutes = (CHAT_SUGGESTION_SCAN_DEFER_MINUTES + steps * 15).min(180);
    (now + chrono::Duration::minutes(delay_minutes)).to_rfc3339()
}

pub(super) fn suggestion_kind_title(kind: &str) -> &'static str {
    match kind {
        "watcher" => "Watcher",
        "workflow" => "Workflow",
        "task" => "Task",
        "app" => "App",
        _ => "Automation",
    }
}

pub(super) fn chat_suggestion_display_status(raw: &str) -> &'static str {
    match raw {
        "completed" => "Ready",
        "deferred_busy" => "Deferred",
        "no_user_chat" => "Waiting for chat",
        "no_candidates" => "Idle",
        "running" => "Scanning",
        "error" => "Needs attention",
        _ => "Scheduled",
    }
}

pub(super) async fn load_chat_suggestions(
    storage: &crate::storage::Storage,
) -> Vec<ChatAutomationSuggestion> {
    match storage.get(AUTONOMY_CHAT_SUGGESTIONS_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<Vec<ChatAutomationSuggestion>>(&raw).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

pub(super) async fn save_chat_suggestions(
    storage: &crate::storage::Storage,
    suggestions: &[ChatAutomationSuggestion],
) {
    if let Ok(bytes) = serde_json::to_vec(suggestions) {
        let _ = storage.set(AUTONOMY_CHAT_SUGGESTIONS_KEY, &bytes).await;
    }
}

pub(super) async fn load_chat_suggestion_scan_state(
    storage: &crate::storage::Storage,
) -> ChatSuggestionScanState {
    match storage.get(AUTONOMY_CHAT_SUGGESTION_SCAN_STATE_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<ChatSuggestionScanState>(&raw).unwrap_or_default()
        }
        _ => ChatSuggestionScanState::default(),
    }
}

pub(super) async fn save_chat_suggestion_scan_state(
    storage: &crate::storage::Storage,
    state: &ChatSuggestionScanState,
) {
    if let Ok(bytes) = serde_json::to_vec(state) {
        let _ = storage
            .set(AUTONOMY_CHAT_SUGGESTION_SCAN_STATE_KEY, &bytes)
            .await;
    }
}

pub(super) fn upsert_chat_suggestion_watermark(
    state: &mut ChatSuggestionScanState,
    conversation_id: &str,
    conversation_updated_at: &str,
    user_message_id: Option<&str>,
    user_message_at: Option<&str>,
    scanned_at: &str,
) {
    if let Some(existing) = state
        .conversation_watermarks
        .iter_mut()
        .find(|entry| entry.conversation_id == conversation_id)
    {
        existing.last_scanned_updated_at = conversation_updated_at.to_string();
        existing.last_user_message_id = user_message_id.map(ToString::to_string);
        existing.last_user_message_at = user_message_at.map(ToString::to_string);
        existing.scanned_at = scanned_at.to_string();
    } else {
        state
            .conversation_watermarks
            .push(ChatSuggestionConversationWatermark {
                conversation_id: conversation_id.to_string(),
                last_scanned_updated_at: conversation_updated_at.to_string(),
                last_user_message_id: user_message_id.map(ToString::to_string),
                last_user_message_at: user_message_at.map(ToString::to_string),
                scanned_at: scanned_at.to_string(),
            });
    }

    state.conversation_watermarks.sort_by(|a, b| {
        parse_rfc3339_utc(&b.scanned_at)
            .cmp(&parse_rfc3339_utc(&a.scanned_at))
            .then_with(|| a.conversation_id.cmp(&b.conversation_id))
    });
    if state.conversation_watermarks.len() > CHAT_SUGGESTION_RETAINED_WATERMARKS {
        state
            .conversation_watermarks
            .truncate(CHAT_SUGGESTION_RETAINED_WATERMARKS);
    }
    state.tracked_chats = state.conversation_watermarks.len();
}

pub(super) fn prune_chat_suggestion_history(
    mut suggestions: Vec<ChatAutomationSuggestion>,
) -> Vec<ChatAutomationSuggestion> {
    suggestions.sort_by(|a, b| {
        parse_rfc3339_utc(&b.updated_at)
            .cmp(&parse_rfc3339_utc(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut open = 0usize;
    let mut retained = Vec::new();
    for suggestion in suggestions {
        if suggestion.status == "open" {
            if open >= CHAT_SUGGESTION_OPEN_LIMIT {
                continue;
            }
            open += 1;
        }
        retained.push(suggestion);
        if retained.len() >= CHAT_SUGGESTION_RETAINED_HISTORY {
            break;
        }
    }
    retained
}

fn chat_suggestion_is_open_for_dedup(suggestion: &ChatAutomationSuggestion) -> bool {
    suggestion.status == "open"
}

fn chat_suggestion_can_block_duplicate(suggestion: &ChatAutomationSuggestion) -> bool {
    matches!(
        suggestion.status.as_str(),
        "open" | "running" | "accepted" | "completed" | "dismissed"
    )
}

fn chat_suggestion_semantic_payload(suggestion: &ChatAutomationSuggestion) -> serde_json::Value {
    serde_json::json!({
        "id": suggestion.id,
        "kind": suggestion.kind,
        "title": suggestion.title,
        "detail": suggestion.detail,
        "rationale": suggestion.rationale,
        "goal_title": suggestion.goal_title,
        "goal_detail": suggestion.goal_detail,
    })
}

fn chat_suggestion_representative_rank(suggestion: &ChatAutomationSuggestion) -> (u8, f32, i64) {
    let status_rank = match suggestion.status.as_str() {
        "running" | "accepted" | "completed" | "dismissed" => 0,
        "open" => 1,
        _ => 2,
    };
    let updated_at = parse_rfc3339_utc(&suggestion.updated_at)
        .map(|value| value.timestamp())
        .unwrap_or(0);
    (status_rank, suggestion.confidence, updated_at)
}

fn merge_open_chat_suggestion_representative(
    representative: &mut ChatAutomationSuggestion,
    duplicate: &ChatAutomationSuggestion,
    now: &str,
) {
    if representative.status != "open" {
        return;
    }

    let candidate_rank = chat_suggestion_representative_rank(duplicate);
    let current_rank = chat_suggestion_representative_rank(representative);
    let candidate_is_better = candidate_rank.0 < current_rank.0
        || (candidate_rank.0 == current_rank.0
            && (candidate_rank.1 > current_rank.1
                || ((candidate_rank.1 - current_rank.1).abs() <= f32::EPSILON
                    && candidate_rank.2 >= current_rank.2)));

    if candidate_is_better {
        representative.kind = duplicate.kind.clone();
        representative.title = duplicate.title.clone();
        representative.detail = duplicate.detail.clone();
        representative.rationale = duplicate.rationale.clone();
        representative.confidence = representative.confidence.max(duplicate.confidence);
        representative.conversation_id = duplicate.conversation_id.clone();
        representative.conversation_title = duplicate.conversation_title.clone();
        representative.conversation_channel = duplicate.conversation_channel.clone();
        representative.source_message_id = duplicate.source_message_id.clone();
        representative.source_snippet = duplicate.source_snippet.clone();
        representative.goal_title = duplicate.goal_title.clone();
        representative.goal_detail = duplicate.goal_detail.clone();
    } else {
        representative.confidence = representative.confidence.max(duplicate.confidence);
    }

    representative.updated_at = now.to_string();
}

fn parse_chat_suggestion_duplicate_groups(
    text: &str,
    valid_ids: &std::collections::HashSet<String>,
) -> Vec<Vec<String>> {
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

fn apply_chat_suggestion_duplicate_groups(
    suggestions: &mut Vec<ChatAutomationSuggestion>,
    groups: &[Vec<String>],
    now: &str,
) -> usize {
    if suggestions.len() < 2 || groups.is_empty() {
        return 0;
    }

    let mut index_by_id = std::collections::HashMap::new();
    for (idx, suggestion) in suggestions.iter().enumerate() {
        index_by_id.insert(suggestion.id.clone(), idx);
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
            let left_rank = chat_suggestion_representative_rank(&suggestions[*left]);
            let right_rank = chat_suggestion_representative_rank(&suggestions[*right]);
            left_rank
                .0
                .cmp(&right_rank.0)
                .then_with(|| right_rank.1.total_cmp(&left_rank.1))
                .then_with(|| right_rank.2.cmp(&left_rank.2))
                .then_with(|| suggestions[*left].id.cmp(&suggestions[*right].id))
        }) else {
            continue;
        };

        let representative_id = suggestions[representative_idx].id.clone();
        for duplicate_idx in indices {
            if duplicate_idx == representative_idx {
                continue;
            }
            if !chat_suggestion_is_open_for_dedup(&suggestions[duplicate_idx]) {
                continue;
            }
            remove_ids.insert(suggestions[duplicate_idx].id.clone());
            let duplicate = suggestions[duplicate_idx].clone();
            if let Some(current_rep_idx) = suggestions
                .iter()
                .position(|suggestion| suggestion.id == representative_id)
            {
                merge_open_chat_suggestion_representative(
                    &mut suggestions[current_rep_idx],
                    &duplicate,
                    now,
                );
            }
        }
    }

    if remove_ids.is_empty() {
        return 0;
    }
    let before = suggestions.len();
    suggestions.retain(|suggestion| !remove_ids.contains(&suggestion.id));
    before.saturating_sub(suggestions.len())
}

fn build_chat_suggestion_dedup_prompt(suggestions: &[ChatAutomationSuggestion]) -> Option<String> {
    let items = suggestions
        .iter()
        .filter(|suggestion| chat_suggestion_can_block_duplicate(suggestion))
        .map(chat_suggestion_semantic_payload)
        .collect::<Vec<_>>();
    if items.len() < 2 {
        return None;
    }

    Some(format!(
        "You are deduplicating proactive automation follow-up suggestions before they are shown to the user.\n\
         Cluster suggestions only when they have the same underlying user intent and approving one would make the others redundant.\n\
         Judge by intended outcome, target, constraints, schedule, recipient, and artifact/workflow being requested. \
         Wording, order, casing, punctuation, grammar, abbreviations, and typos are irrelevant. \
         Do not use keyword matching or phrase templates; decide from meaning.\n\
         Keep suggestions separate when they pursue materially different deliverables, targets, constraints, schedules, recipients, or external systems.\n\n\
         Suggestions:\n{}\n\n\
         Return strict JSON only with this shape: {{\"duplicate_groups\":[[\"id-a\",\"id-b\"]]}}. \
         Include only groups with two or more ids. Return an empty array when nothing is redundant.",
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
    ))
}

pub(super) async fn collapse_semantically_equivalent_chat_suggestions(
    llm: &crate::core::LlmClient,
    suggestions: &mut Vec<ChatAutomationSuggestion>,
    now: &str,
) -> usize {
    let valid_ids = suggestions
        .iter()
        .filter(|suggestion| chat_suggestion_can_block_duplicate(suggestion))
        .map(|suggestion| suggestion.id.clone())
        .collect::<std::collections::HashSet<_>>();
    if valid_ids.len() < 2 {
        return 0;
    }
    let Some(prompt) = build_chat_suggestion_dedup_prompt(suggestions) else {
        return 0;
    };
    let system = "You are a strict semantic deduplication checker for automation follow-ups. Return JSON only.";
    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(CHAT_SUGGESTION_SEMANTIC_DEDUP_TIMEOUT_SECS),
        llm.chat_with_system_bounded(system, &prompt, 360),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::debug!("chat_suggestion_dedup: LLM clustering unavailable: {}", error);
            return 0;
        }
        Err(_) => {
            tracing::debug!(
                "chat_suggestion_dedup: LLM clustering timed out after {}s",
                CHAT_SUGGESTION_SEMANTIC_DEDUP_TIMEOUT_SECS
            );
            return 0;
        }
    };
    let groups = parse_chat_suggestion_duplicate_groups(&response.content, &valid_ids);
    apply_chat_suggestion_duplicate_groups(suggestions, &groups, now)
}

pub(super) fn chat_suggestion_scan_is_due(
    state: &ChatSuggestionScanState,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    match state.next_due_at.as_deref().and_then(parse_rfc3339_utc) {
        Some(next_due) => now >= next_due,
        None => true,
    }
}

pub(super) async fn server_busy_for_chat_suggestions(state: &AppState) -> bool {
    if server_under_load(state).await {
        return true;
    }
    let active_traces = {
        let history = state.trace_history.read().await;
        history
            .iter()
            .filter(|trace| trace.completed_at.is_none())
            .count()
    };
    active_traces > 0
}

pub(super) async fn conversation_has_recent_app_artifact(
    storage: &crate::storage::Storage,
    conversation_id: &str,
) -> bool {
    let key = Agent::conversation_recent_artifact_key(conversation_id);
    let artifact = storage
        .get(&key)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok());
    let has_recent_app = artifact.as_ref().is_some_and(|artifact| match artifact {
        serde_json::Value::Array(items) => items.iter().any(|item| {
            item.get("artifact_type")
                .and_then(|value| value.as_str())
                .is_some_and(|artifact_type| artifact_type.eq_ignore_ascii_case("app"))
        }),
        serde_json::Value::Object(map) => map
            .get("artifact_type")
            .and_then(|value| value.as_str())
            .is_some_and(|artifact_type| artifact_type.eq_ignore_ascii_case("app")),
        _ => false,
    });
    has_recent_app
        || storage
            .get(&Agent::conversation_last_deployed_app_key(conversation_id))
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
            .is_some()
}

pub(super) fn looks_like_low_signal_message(input: &str) -> bool {
    normalize_chat_suggestion_text(input).trim().is_empty()
}

pub(super) fn conversation_has_signal(
    messages: &[crate::storage::entities::message::Model],
) -> bool {
    messages.iter().any(|message| {
        message.role.eq_ignore_ascii_case("user")
            && !looks_like_low_signal_message(&message.content)
    })
}

pub(super) fn extract_latest_signal_user_message(
    messages: &[crate::storage::entities::message::Model],
) -> Option<crate::storage::entities::message::Model> {
    messages
        .iter()
        .rev()
        .find(|message| {
            message.role.eq_ignore_ascii_case("user")
                && !looks_like_low_signal_message(&message.content)
        })
        .cloned()
}

fn compact_chat_suggestion_text(input: &str, max_chars: usize) -> String {
    let compact = normalize_chat_suggestion_text(input)
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'))
        .to_string();
    let mut out = compact.chars().take(max_chars).collect::<String>();
    if compact.chars().count() > max_chars {
        out = out.trim_end().to_string();
        out.push_str("...");
    }
    out
}

fn normalize_chat_suggestion_kind(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "watcher" => Some("watcher"),
        "workflow" => Some("workflow"),
        "task" => Some("task"),
        "app" => Some("app"),
        _ => None,
    }
}

fn chat_suggestion_inference_json(raw: &str) -> Option<serde_json::Value> {
    let trimmed = raw.trim();
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

fn chat_suggestion_inference_text(
    payload: &serde_json::Value,
    key: &str,
    max_chars: usize,
) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| compact_chat_suggestion_text(value, max_chars))
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn build_chat_automation_suggestion_from_inference(
    conversation: &crate::storage::entities::conversation::Model,
    source_message: &crate::storage::entities::message::Model,
    payload: &serde_json::Value,
) -> Option<ChatAutomationSuggestion> {
    if !payload
        .get("should_suggest")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let snippet = compact_chat_suggestion_text(&source_message.content, 420);
    if snippet.is_empty() {
        return None;
    }
    let kind = payload
        .get("kind")
        .and_then(|value| value.as_str())
        .and_then(normalize_chat_suggestion_kind)?;
    let confidence = payload
        .get("confidence")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0) as f32;
    if confidence < 0.65 {
        return None;
    }
    let title = chat_suggestion_inference_text(payload, "title", 110)?;
    let detail = chat_suggestion_inference_text(payload, "detail", 260)?;
    let rationale = chat_suggestion_inference_text(payload, "rationale", 220)?;
    let kind_label = suggestion_kind_title(kind);
    let goal_title = chat_suggestion_inference_text(payload, "goal_title", 140)
        .unwrap_or_else(|| format!("Launch {} from chat signal", kind_label.to_ascii_lowercase()));
    let goal_detail = chat_suggestion_inference_text(payload, "goal_detail", 420).or_else(|| {
        Some(format!(
            "Source conversation: {}. Original signal: {}",
            conversation.title.trim(),
            compact_chat_suggestion_text(&source_message.content, 220)
        ))
    });
    let now = chrono::Utc::now().to_rfc3339();
    let fingerprint = {
        let mut hasher = Sha256::new();
        hasher.update(b"chat_automation_suggestion_v1");
        hasher.update([0u8]);
        hasher.update(conversation.id.as_bytes());
        hasher.update([0u8]);
        hasher.update(source_message.id.as_bytes());
        hasher.update([0u8]);
        hasher.update(kind.as_bytes());
        hasher.update([0u8]);
        hasher.update(title.as_bytes());
        hasher.update([0u8]);
        hasher.update(detail.as_bytes());
        hex::encode(hasher.finalize())
            .chars()
            .take(24)
            .collect::<String>()
    };

    Some(ChatAutomationSuggestion {
        id: uuid::Uuid::new_v4().to_string(),
        status: "open".to_string(),
        kind: kind.to_string(),
        title,
        detail,
        rationale,
        confidence,
        created_at: now.clone(),
        updated_at: now,
        conversation_id: conversation.id.clone(),
        conversation_title: if conversation.title.trim().is_empty() {
            "Untitled conversation".to_string()
        } else {
            conversation.title.clone()
        },
        conversation_channel: conversation.channel.clone(),
        source_message_id: source_message.id.clone(),
        source_snippet: snippet,
        fingerprint,
        goal_title,
        goal_detail,
        accepted_goal_id: None,
        dismissed_at: None,
        accepted_at: None,
        accepted_trace_id: None,
        run_status: None,
        last_run_error: None,
        last_run_started_at: None,
        last_run_completed_at: None,
        accepted_outcomes: Vec::new(),
    })
}

pub(super) async fn infer_chat_automation_suggestion(
    agent: &Agent,
    conversation: &crate::storage::entities::conversation::Model,
    source_message: &crate::storage::entities::message::Model,
    recent_messages: &[crate::storage::entities::message::Model],
) -> Option<ChatAutomationSuggestion> {
    if compact_chat_suggestion_text(&source_message.content, 1).is_empty() {
        return None;
    }
    let recent_dialogue = recent_messages
        .iter()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| {
            format!(
                "{}: {}",
                message.role.trim(),
                compact_chat_suggestion_text(&message.content, 700)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let response_shape = r#"{"should_suggest":false,"kind":null,"title":"","detail":"","rationale":"","confidence":0.0,"goal_title":"","goal_detail":""}"#;
    let prompt = format!(
        "Conversation title: {conversation_title}\n\
Conversation channel: {conversation_channel}\n\n\
Recent dialogue:\n{recent_dialogue}\n\n\
Candidate user message:\n{candidate}\n\n\
Return JSON only with this shape:\n{response_shape}\n\n\
Decide whether this message reveals a durable opportunity for AgentArk to proactively prepare future work before the user asks again. \
Use semantic meaning and conversational context. Do not use fixed phrases, keyword matching, regexes, punctuation, casing, or expected wording. \
When should_suggest is true, choose exactly one kind from watcher, workflow, task, or app. \
Suggest only when there is a concrete future action AgentArk can prepare as an approval-gated watcher, workflow, task, or app. \
Do not suggest for ordinary questions, one-off explanations, generic greetings, or messages without a concrete follow-up opportunity. \
Keep title, detail, rationale, goal_title, and goal_detail concise and user-facing. \
Sensitive external effects must be framed as drafts or approval-gated work.",
        conversation_title = conversation.title,
        conversation_channel = conversation.channel,
        recent_dialogue = if recent_dialogue.trim().is_empty() {
            "(none)"
        } else {
            recent_dialogue.as_str()
        },
        candidate = compact_chat_suggestion_text(&source_message.content, 1200),
        response_shape = response_shape,
    );

    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(6),
        agent.llm.chat_with_system_bounded(
            "You infer proactive automation opportunities from chat semantics. Return strict JSON only.",
            &prompt,
            320,
        ),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::debug!("Chat suggestion inference failed: {}", error);
            return None;
        }
        Err(_) => {
            tracing::debug!("Chat suggestion inference timed out");
            return None;
        }
    };
    let payload = chat_suggestion_inference_json(&response.content)?;
    build_chat_automation_suggestion_from_inference(conversation, source_message, &payload)
}

pub(super) fn build_chat_suggestion_execution_prompt(
    suggestion: &ChatAutomationSuggestion,
) -> String {
    let kind_label = suggestion_kind_title(&suggestion.kind);
    let focus = suggestion
        .goal_detail
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&suggestion.title);
    let execution_directive = match suggestion.kind.as_str() {
        "app" => {
            "Build and deploy a concrete starter app now if feasible. Prefer a working thin slice over a plan-only response."
        }
        "watcher" => "Create a concrete watcher now. Do not just describe the watcher.",
        "workflow" => {
            "Create a concrete automation now, preferably as a watcher, scheduled task, or goal loop."
        }
        "task" => "Create a concrete task or goal now rather than leaving this as an idea.",
        _ => "Execute the best concrete automation now rather than only describing it.",
    };

    format!(
        "A Mission Control suggestion was inferred from a prior user chat, and the user has now explicitly clicked Accept.\n\
You should execute this accepted suggestion now.\n\
Do not merely save it as a draft goal unless you are blocked by missing information.\n\
If the suggestion is best fulfilled by building/deploying an app, do that so the trace includes real build/runtime details.\n\
If the suggestion is better fulfilled as a watcher, scheduled task, or goal workflow, create that concrete automation instead.\n\
If required inputs are missing, do the safest concrete version you can and clearly say what remains missing.\n\n\
Accepted suggestion type: {kind_label}\n\
Suggestion title: {title}\n\
Suggestion detail: {detail}\n\
Rationale: {rationale}\n\
Original user snippet: {snippet}\n\
Conversation title: {conversation_title}\n\
Requested focus: {focus}\n\n\
Execution directive: {execution_directive}\n\
Return a concise final outcome after the actual work is done.",
        kind_label = kind_label,
        title = suggestion.title,
        detail = suggestion.detail,
        rationale = suggestion.rationale,
        snippet = suggestion.source_snippet,
        conversation_title = suggestion.conversation_title,
        focus = focus,
        execution_directive = execution_directive,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_suggestion(id: &str, title: &str, detail: &str, confidence: f32) -> ChatAutomationSuggestion {
        ChatAutomationSuggestion {
            id: id.to_string(),
            status: "open".to_string(),
            kind: "app".to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            rationale: "The conversation points to a reusable deliverable.".to_string(),
            confidence,
            created_at: "2026-04-26T00:00:00Z".to_string(),
            updated_at: "2026-04-26T00:00:00Z".to_string(),
            conversation_id: format!("conversation-{id}"),
            conversation_title: "Chat".to_string(),
            conversation_channel: "web".to_string(),
            source_message_id: format!("message-{id}"),
            source_snippet: title.to_string(),
            fingerprint: format!("fingerprint-{id}"),
            goal_title: title.to_string(),
            goal_detail: Some(detail.to_string()),
            accepted_goal_id: None,
            dismissed_at: None,
            accepted_at: None,
            accepted_trace_id: None,
            run_status: None,
            last_run_error: None,
            last_run_started_at: None,
            last_run_completed_at: None,
            accepted_outcomes: Vec::new(),
        }
    }

    #[test]
    fn parse_chat_suggestion_duplicate_groups_ignores_invalid_or_singleton_ids() {
        let valid_ids = ["a".to_string(), "b".to_string(), "c".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let groups = parse_chat_suggestion_duplicate_groups(
            r#"{"duplicate_groups":[["a","b","missing"],["c"],["b","a"]]}"#,
            &valid_ids,
        );

        assert_eq!(groups, vec![vec!["a".to_string(), "b".to_string()]]);
    }

    #[test]
    fn duplicate_groups_collapse_open_semantic_variants_to_one_representative() {
        let mut suggestions = vec![
            test_suggestion(
                "a",
                "Build landing page for baby Software",
                "Create an enterprise landing page with hero, products, services, and trust signals.",
                0.81,
            ),
            test_suggestion(
                "b",
                "Baby Software Landing Page",
                "Create a complete enterprise landing page for baby Software with product cards and case studies.",
                0.93,
            ),
            test_suggestion(
                "c",
                "Production Landing Page Approval Workflow",
                "Prepare a ready-to-deploy landing page draft pending approval.",
                0.88,
            ),
            test_suggestion(
                "d",
                "Generate Baby Software Landing Page",
                "Create a ready-to-deploy HTML/CSS/JS landing page with hero and trust signals.",
                0.87,
            ),
        ];

        let removed = apply_chat_suggestion_duplicate_groups(
            &mut suggestions,
            &[vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ]],
            "2026-04-26T01:00:00Z",
        );

        assert_eq!(removed, 3);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].id, "b");
        assert_eq!(suggestions[0].status, "open");
        assert_eq!(suggestions[0].confidence, 0.93);
    }

    #[test]
    fn duplicate_groups_keep_distinct_suggestions_outside_group() {
        let mut suggestions = vec![
            test_suggestion("a", "Create invoice workflow", "Draft monthly invoice automation.", 0.9),
            test_suggestion("b", "Monitor deployment health", "Watch production app health checks.", 0.9),
            test_suggestion("c", "Deployment health watcher", "Set up a health-check watcher.", 0.92),
        ];

        let removed = apply_chat_suggestion_duplicate_groups(
            &mut suggestions,
            &[vec!["b".to_string(), "c".to_string()]],
            "2026-04-26T01:00:00Z",
        );

        assert_eq!(removed, 1);
        assert_eq!(suggestions.len(), 2);
        assert!(suggestions.iter().any(|suggestion| suggestion.id == "a"));
        assert!(suggestions.iter().any(|suggestion| suggestion.id == "c"));
    }
}
