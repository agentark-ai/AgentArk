use super::*;

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const CHAT_SUGGESTION_RESOLUTION_TIMEOUT_SECS: u64 = 6;
const CHAT_SUGGESTION_RESOLUTION_RECHECK_SECS: i64 = 10 * 60;
const CHAT_SUGGESTION_RESOLUTION_MAX_ARTIFACTS: usize = 16;
const CHAT_SUGGESTION_RESOLUTION_MAX_TEXT_CHARS: usize = 700;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ChatSuggestionOutcome {
    pub kind: String,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SuggestionRunSnapshot {
    task_ids: HashMap<String, crate::storage::entities::task::Model>,
    watcher_ids: HashMap<String, crate::core::watcher::Watcher>,
    app_ids: HashMap<String, serde_json::Value>,
}

fn normalize_task_status(raw: &str) -> String {
    let parsed = serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string());
    let trimmed = parsed.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if ch.is_uppercase() && idx > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn watcher_status_label(status: &crate::core::watcher::WatcherStatus) -> String {
    match status {
        crate::core::watcher::WatcherStatus::Active => "active".to_string(),
        crate::core::watcher::WatcherStatus::Paused => "paused".to_string(),
        crate::core::watcher::WatcherStatus::Triggered => "triggered".to_string(),
        crate::core::watcher::WatcherStatus::TimedOut => "timed_out".to_string(),
        crate::core::watcher::WatcherStatus::Cancelled => "cancelled".to_string(),
        crate::core::watcher::WatcherStatus::Failed { .. } => "failed".to_string(),
    }
}

fn outcome_kind_priority(suggestion_kind: &str, outcome_kind: &str) -> usize {
    match suggestion_kind {
        "app" => match outcome_kind {
            "app" => 0,
            "watcher" => 1,
            "task" => 2,
            _ => 3,
        },
        "watcher" => match outcome_kind {
            "watcher" => 0,
            "task" => 1,
            "app" => 2,
            _ => 3,
        },
        "task" => match outcome_kind {
            "task" => 0,
            "watcher" => 1,
            "app" => 2,
            _ => 3,
        },
        "workflow" => match outcome_kind {
            "watcher" => 0,
            "task" => 1,
            "app" => 2,
            _ => 3,
        },
        _ => 3,
    }
}

#[derive(Debug, Clone)]
struct DurableSuggestionArtifact {
    key: String,
    kind: String,
    id: String,
    title: String,
    detail: String,
    status: String,
    created_at: Option<String>,
    updated_at: Option<String>,
    conversation_id: Option<String>,
    view: Option<String>,
    url: Option<String>,
    source_suggestion_ids: HashSet<String>,
}

fn compact_resolution_text(input: &str, max_chars: usize) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = compact.chars().take(max_chars).collect::<String>();
    if compact.chars().count() > max_chars {
        out = out.trim_end().to_string();
        out.push_str("...");
    }
    out
}

fn json_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn collect_matching_suggestion_ids(
    value: &serde_json::Value,
    open_suggestion_ids: &HashSet<String>,
    found: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if open_suggestion_ids.contains(trimmed) {
                found.insert(trimmed.to_string());
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_matching_suggestion_ids(item, open_suggestion_ids, found);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_matching_suggestion_ids(value, open_suggestion_ids, found);
            }
        }
        _ => {}
    }
}

fn artifact_view_for_kind(kind: &str) -> Option<String> {
    match kind {
        "task" => Some("tasks".to_string()),
        "watcher" => Some("watchers".to_string()),
        "app" => Some("apps".to_string()),
        "background_session" => Some("background-work".to_string()),
        _ => None,
    }
}

fn normalized_task_model_status(raw: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        match value {
            serde_json::Value::String(text) => return normalize_task_status(&text),
            serde_json::Value::Object(map) => {
                if let Some((key, _)) = map.into_iter().next() {
                    return normalize_task_status(&key);
                }
            }
            _ => {}
        }
    }
    normalize_task_status(raw)
}

fn task_status_can_satisfy_suggestion(status: &str) -> bool {
    !matches!(status, "failed" | "cancelled")
}

fn watcher_status_can_satisfy_suggestion(status: &str) -> bool {
    !matches!(status, "failed" | "cancelled" | "timed_out")
}

fn background_status_can_satisfy_suggestion(status: &str) -> bool {
    !matches!(status, "draft" | "failed" | "cancelled")
}

fn artifact_prompt_payload(artifact: &DurableSuggestionArtifact) -> serde_json::Value {
    serde_json::json!({
        "artifact_key": artifact.key,
        "kind": artifact.kind,
        "id": artifact.id,
        "title": compact_resolution_text(&artifact.title, 220),
        "detail": compact_resolution_text(&artifact.detail, CHAT_SUGGESTION_RESOLUTION_MAX_TEXT_CHARS),
        "status": artifact.status,
        "created_at": artifact.created_at,
        "updated_at": artifact.updated_at,
        "conversation_id": artifact.conversation_id,
    })
}

fn artifact_as_outcome(
    artifact: &DurableSuggestionArtifact,
    primary: bool,
) -> ChatSuggestionOutcome {
    ChatSuggestionOutcome {
        kind: artifact.kind.clone(),
        id: artifact.id.clone(),
        title: artifact.title.clone(),
        detail: (!artifact.detail.trim().is_empty()).then(|| artifact.detail.clone()),
        status: Some(artifact.status.clone()),
        url: artifact.url.clone(),
        view: artifact.view.clone(),
        created_at: artifact
            .created_at
            .clone()
            .or_else(|| artifact.updated_at.clone()),
        primary,
    }
}

fn suggestion_resolution_signature(
    suggestion: &ChatAutomationSuggestion,
    artifacts: &[DurableSuggestionArtifact],
) -> String {
    let mut rows = artifacts
        .iter()
        .map(|artifact| {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                artifact.key,
                artifact.status,
                artifact.title,
                artifact.detail,
                artifact.created_at.as_deref().unwrap_or_default(),
                artifact.updated_at.as_deref().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"chat_suggestion_resolution_v1");
    for value in [
        suggestion.kind.as_str(),
        suggestion.title.as_str(),
        suggestion.detail.as_str(),
        suggestion.rationale.as_str(),
        suggestion.goal_title.as_str(),
        suggestion.goal_detail.as_deref().unwrap_or_default(),
        suggestion.source_snippet.as_str(),
    ] {
        hasher.update([0]);
        hasher.update(
            compact_resolution_text(value, CHAT_SUGGESTION_RESOLUTION_MAX_TEXT_CHARS).as_bytes(),
        );
    }
    for row in rows {
        hasher.update([0]);
        hasher.update(row.as_bytes());
    }
    hex::encode(hasher.finalize())
        .chars()
        .take(24)
        .collect::<String>()
}

fn recently_checked_resolution_signature(
    suggestion: &ChatAutomationSuggestion,
    signature: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if suggestion.resolution_check_signature.as_deref() != Some(signature) {
        return false;
    }
    suggestion
        .resolution_checked_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .is_some_and(|checked_at| {
            now.signed_duration_since(checked_at).num_seconds()
                < CHAT_SUGGESTION_RESOLUTION_RECHECK_SECS
        })
}

fn mark_resolution_checked(
    suggestion: &mut ChatAutomationSuggestion,
    signature: &str,
    now_text: &str,
) {
    suggestion.resolution_checked_at = Some(now_text.to_string());
    suggestion.resolution_check_signature = Some(signature.to_string());
}

fn mark_suggestion_resolved_by_artifacts(
    suggestion: &mut ChatAutomationSuggestion,
    artifacts: &[DurableSuggestionArtifact],
    summary: &str,
    signature: &str,
    now_text: &str,
) {
    let mut outcomes = artifacts
        .iter()
        .enumerate()
        .map(|(idx, artifact)| artifact_as_outcome(artifact, idx == 0))
        .collect::<Vec<_>>();
    outcomes.sort_by(|left, right| {
        outcome_kind_priority(&suggestion.kind, &left.kind)
            .cmp(&outcome_kind_priority(&suggestion.kind, &right.kind))
            .then_with(|| left.title.cmp(&right.title))
    });
    if let Some(first) = outcomes.first_mut() {
        first.primary = true;
    }

    suggestion.status = "completed".to_string();
    suggestion.run_status = Some("already_handled".to_string());
    suggestion.last_run_error = None;
    suggestion.last_run_completed_at = Some(now_text.to_string());
    suggestion.updated_at = now_text.to_string();
    suggestion.resolved_at = Some(now_text.to_string());
    suggestion.resolution_summary = Some(compact_resolution_text(summary, 260));
    suggestion.accepted_outcomes = outcomes;
    mark_resolution_checked(suggestion, signature, now_text);
}

fn artifact_scope_matches_suggestion(
    artifact: &DurableSuggestionArtifact,
    suggestion: &ChatAutomationSuggestion,
) -> bool {
    if artifact
        .source_suggestion_ids
        .contains(suggestion.id.trim())
    {
        return true;
    }
    let suggestion_conversation = suggestion.conversation_id.trim();
    !suggestion_conversation.is_empty()
        && artifact
            .conversation_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|conversation_id| conversation_id == suggestion_conversation)
}

fn dedupe_resolution_artifacts(artifacts: &mut Vec<DurableSuggestionArtifact>) {
    let mut seen = HashSet::new();
    artifacts.retain(|artifact| seen.insert(artifact.key.clone()));
}

fn artifact_recency_key(artifact: &DurableSuggestionArtifact) -> i64 {
    artifact
        .updated_at
        .as_deref()
        .or(artifact.created_at.as_deref())
        .and_then(parse_rfc3339_utc)
        .map(|value| value.timestamp())
        .unwrap_or(0)
}

fn sort_resolution_artifacts(
    artifacts: &mut [DurableSuggestionArtifact],
    suggestion: &ChatAutomationSuggestion,
) {
    artifacts.sort_by(|left, right| {
        let left_direct = left.source_suggestion_ids.contains(suggestion.id.trim());
        let right_direct = right.source_suggestion_ids.contains(suggestion.id.trim());
        right_direct
            .cmp(&left_direct)
            .then_with(|| {
                outcome_kind_priority(&suggestion.kind, &left.kind)
                    .cmp(&outcome_kind_priority(&suggestion.kind, &right.kind))
            })
            .then_with(|| artifact_recency_key(right).cmp(&artifact_recency_key(left)))
            .then_with(|| left.key.cmp(&right.key))
    });
}

async fn load_recent_conversation_artifacts(
    storage: &crate::storage::Storage,
    conversation_id: &str,
    open_suggestion_ids: &HashSet<String>,
) -> Vec<DurableSuggestionArtifact> {
    let key = crate::core::Agent::conversation_recent_artifact_key(conversation_id);
    let Some(raw) = storage.get(&key).await.ok().flatten() else {
        return Vec::new();
    };
    let value =
        serde_json::from_slice::<serde_json::Value>(&raw).unwrap_or(serde_json::Value::Null);
    let items = match value {
        serde_json::Value::Array(items) => items,
        object @ serde_json::Value::Object(_) => vec![object],
        _ => Vec::new(),
    };

    items
        .into_iter()
        .filter_map(|item| {
            let kind = json_string_field(&item, "artifact_type")?;
            let id = json_string_field(&item, "artifact_id")?;
            let title = json_string_field(&item, "title").unwrap_or_else(|| kind.clone());
            let detail = json_string_field(&item, "summary").unwrap_or_default();
            let updated_at = json_string_field(&item, "updated_at");
            let url = json_string_field(&item, "url");
            let mut source_suggestion_ids = HashSet::new();
            collect_matching_suggestion_ids(&item, open_suggestion_ids, &mut source_suggestion_ids);
            Some(DurableSuggestionArtifact {
                key: format!("{}:{}", kind, id),
                kind: kind.clone(),
                id,
                title,
                detail,
                status: "exists".to_string(),
                created_at: None,
                updated_at,
                conversation_id: Some(conversation_id.trim().to_string()),
                view: artifact_view_for_kind(&kind),
                url,
                source_suggestion_ids,
            })
        })
        .collect()
}

async fn load_durable_suggestion_artifacts(
    agent: &crate::core::Agent,
    open_suggestion_ids: &HashSet<String>,
) -> Vec<DurableSuggestionArtifact> {
    let mut artifacts = Vec::new();

    for task in agent.storage.get_tasks().await.unwrap_or_default() {
        let status = normalized_task_model_status(&task.status);
        if !task_status_can_satisfy_suggestion(&status) {
            continue;
        }
        let arguments =
            serde_json::from_str::<serde_json::Value>(&task.arguments).unwrap_or_default();
        let origin = crate::core::automation::origin_from_arguments(&arguments);
        let mut source_suggestion_ids = HashSet::new();
        collect_matching_suggestion_ids(
            &arguments,
            open_suggestion_ids,
            &mut source_suggestion_ids,
        );
        artifacts.push(DurableSuggestionArtifact {
            key: format!("task:{}", task.id),
            kind: "task".to_string(),
            id: task.id,
            title: task.description,
            detail: format!("Action: {}", task.action),
            status,
            created_at: Some(task.created_at),
            updated_at: Some(task.updated_at),
            conversation_id: origin.conversation_id,
            view: Some("tasks".to_string()),
            url: None,
            source_suggestion_ids,
        });
    }

    for watcher in agent.watcher_manager.list().await {
        let status = watcher_status_label(&watcher.status);
        if !watcher_status_can_satisfy_suggestion(&status) {
            continue;
        }
        let origin = crate::core::automation::origin_from_arguments(&watcher.poll_arguments);
        let mut source_suggestion_ids = HashSet::new();
        collect_matching_suggestion_ids(
            &watcher.poll_arguments,
            open_suggestion_ids,
            &mut source_suggestion_ids,
        );
        artifacts.push(DurableSuggestionArtifact {
            key: format!("watcher:{}", watcher.id),
            kind: "watcher".to_string(),
            id: watcher.id.to_string(),
            title: watcher.description,
            detail: format!(
                "Polls {} every {}s. Trigger: {}",
                watcher.poll_action, watcher.interval_secs, watcher.on_trigger
            ),
            status,
            created_at: Some(watcher.created_at.to_rfc3339()),
            updated_at: watcher
                .last_poll_at
                .map(|value| value.to_rfc3339())
                .or_else(|| Some(watcher.created_at.to_rfc3339())),
            conversation_id: origin.conversation_id,
            view: Some("watchers".to_string()),
            url: None,
            source_suggestion_ids,
        });
    }

    for session in agent.background_sessions.list().await {
        let status = session.status.label().to_string();
        if !background_status_can_satisfy_suggestion(&status) {
            continue;
        }
        let payload = serde_json::to_value(&session).unwrap_or(serde_json::Value::Null);
        let mut source_suggestion_ids = HashSet::new();
        collect_matching_suggestion_ids(&payload, open_suggestion_ids, &mut source_suggestion_ids);
        let detail = [
            session.objective.as_str(),
            session.summary.as_deref().unwrap_or_default(),
            session.current_focus.as_deref().unwrap_or_default(),
            session.waiting_on.as_deref().unwrap_or_default(),
            session.next_expected_action.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        artifacts.push(DurableSuggestionArtifact {
            key: format!("background_session:{}", session.id),
            kind: "background_session".to_string(),
            id: session.id,
            title: session.title,
            detail,
            status,
            created_at: Some(session.created_at.to_rfc3339()),
            updated_at: Some(session.updated_at.to_rfc3339()),
            conversation_id: session.conversation_id,
            view: Some("background-work".to_string()),
            url: None,
            source_suggestion_ids,
        });
    }

    for app in agent.app_registry.list().await {
        let Some(id) = json_string_field(&app, "id") else {
            continue;
        };
        if app
            .get("enabled")
            .and_then(|value| value.as_bool())
            .is_some_and(|enabled| !enabled)
        {
            continue;
        }
        let mut source_suggestion_ids = HashSet::new();
        collect_matching_suggestion_ids(&app, open_suggestion_ids, &mut source_suggestion_ids);
        let title = json_string_field(&app, "title").unwrap_or_else(|| "Deployed app".to_string());
        let status = json_string_field(&app, "runtime_mode").unwrap_or_else(|| {
            app.get("running")
                .and_then(|value| value.as_bool())
                .map(|running| if running { "running" } else { "stopped" })
                .unwrap_or("unknown")
                .to_string()
        });
        artifacts.push(DurableSuggestionArtifact {
            key: format!("app:{}", id),
            kind: "app".to_string(),
            id,
            title,
            detail: status.clone(),
            status,
            created_at: json_string_field(&app, "created_at"),
            updated_at: json_string_field(&app, "updated_at"),
            conversation_id: json_string_field(&app, "conversation_id"),
            view: Some("apps".to_string()),
            url: json_string_field(&app, "access_url").or_else(|| json_string_field(&app, "url")),
            source_suggestion_ids,
        });
    }

    artifacts
}

fn parse_resolution_json(text: &str) -> Option<serde_json::Value> {
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

async fn semantically_resolved_artifacts(
    llm: &crate::core::LlmClient,
    suggestion: &ChatAutomationSuggestion,
    artifacts: &[DurableSuggestionArtifact],
) -> Option<(Vec<DurableSuggestionArtifact>, String)> {
    if artifacts.is_empty() {
        return None;
    }
    let artifact_keys = artifacts
        .iter()
        .map(|artifact| artifact.key.clone())
        .collect::<HashSet<_>>();
    let prompt = format!(
        "Decide whether an open proactive automation suggestion is already handled by durable AgentArk work.\n\
Judge by underlying intent and meaning, not words. Casing, punctuation, grammar, abbreviations, typos, order, and phrasing are irrelevant.\n\
An artifact handles the suggestion only if it already satisfies, implements, or is the pending/running durable record for the same requested outcome, target, trigger/cadence, recipient/delivery route, and important constraints.\n\
Do not mark resolved merely because the same app, integration, or broad topic appears. Keep it open when target, condition, schedule, delivery, or deliverable materially differs.\n\
Failed, cancelled, or unrelated artifacts do not resolve a suggestion.\n\n\
Suggestion:\n{}\n\n\
Candidate durable artifacts from the same structured context:\n{}\n\n\
Return strict JSON only: {{\"resolved\":true|false,\"artifact_keys\":[\"kind:id\"],\"summary\":\"short reason\"}}.",
        serde_json::to_string_pretty(&serde_json::json!({
            "kind": suggestion.kind,
            "title": compact_resolution_text(&suggestion.title, 220),
            "detail": compact_resolution_text(&suggestion.detail, 500),
            "rationale": compact_resolution_text(&suggestion.rationale, 500),
            "goal_title": compact_resolution_text(&suggestion.goal_title, 220),
            "goal_detail": suggestion
                .goal_detail
                .as_deref()
                .map(|value| compact_resolution_text(value, 500)),
            "source_snippet": compact_resolution_text(&suggestion.source_snippet, 500),
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        serde_json::to_string_pretty(
            &artifacts
                .iter()
                .map(artifact_prompt_payload)
                .collect::<Vec<_>>()
        )
        .unwrap_or_else(|_| "[]".to_string())
    );

    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(CHAT_SUGGESTION_RESOLUTION_TIMEOUT_SECS),
        llm.chat_with_system_bounded(
            "You are a strict semantic reconciliation judge for durable automation state. Return JSON only.",
            &prompt,
            260,
        ),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::debug!("chat_suggestion_resolution: semantic judge unavailable: {}", error);
            return None;
        }
        Err(_) => {
            tracing::debug!(
                "chat_suggestion_resolution: semantic judge timed out after {}s",
                CHAT_SUGGESTION_RESOLUTION_TIMEOUT_SECS
            );
            return None;
        }
    };

    let payload = parse_resolution_json(&response.content)?;
    if !payload
        .get("resolved")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let selected_keys = payload
        .get("artifact_keys")
        .or_else(|| payload.get("artifacts"))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|key| artifact_keys.contains(*key))
                .map(ToString::to_string)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    if selected_keys.is_empty() {
        return None;
    }
    let selected = artifacts
        .iter()
        .filter(|artifact| selected_keys.contains(&artifact.key))
        .cloned()
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return None;
    }
    let summary = payload
        .get("summary")
        .and_then(|value| value.as_str())
        .map(|value| compact_resolution_text(value, 260))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Existing durable work already handles this suggestion.".to_string());
    Some((selected, summary))
}

pub(super) async fn reconcile_open_chat_suggestions_with_durable_work(
    agent: &crate::core::Agent,
    suggestions: &mut Vec<ChatAutomationSuggestion>,
) -> usize {
    let open_suggestion_ids = suggestions
        .iter()
        .filter(|suggestion| suggestion.status == "open")
        .map(|suggestion| suggestion.id.clone())
        .collect::<HashSet<_>>();
    if open_suggestion_ids.is_empty() {
        return 0;
    }

    let base_artifacts = load_durable_suggestion_artifacts(agent, &open_suggestion_ids).await;
    let base_artifacts_by_key = base_artifacts
        .iter()
        .map(|artifact| (artifact.key.clone(), artifact.clone()))
        .collect::<HashMap<_, _>>();
    let mut recent_artifacts_by_conversation: HashMap<String, Vec<DurableSuggestionArtifact>> =
        HashMap::new();
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let mut changed = 0usize;
    let mut resolved = 0usize;

    for suggestion in suggestions.iter_mut().filter(|item| item.status == "open") {
        let mut artifacts = base_artifacts
            .iter()
            .filter(|artifact| artifact_scope_matches_suggestion(artifact, suggestion))
            .cloned()
            .collect::<Vec<_>>();

        let conversation_id = suggestion.conversation_id.trim().to_string();
        if !conversation_id.is_empty() {
            if !recent_artifacts_by_conversation.contains_key(&conversation_id) {
                let recent = load_recent_conversation_artifacts(
                    &agent.storage,
                    &conversation_id,
                    &open_suggestion_ids,
                )
                .await;
                recent_artifacts_by_conversation.insert(conversation_id.clone(), recent);
            }
            if let Some(recent) = recent_artifacts_by_conversation.get(&conversation_id) {
                artifacts.extend(recent.iter().filter_map(|artifact| {
                    base_artifacts_by_key.get(&artifact.key).map(|base| {
                        let mut merged = base.clone();
                        merged.conversation_id = Some(conversation_id.clone());
                        if merged.title.trim().is_empty() {
                            merged.title = artifact.title.clone();
                        }
                        if merged.detail.trim().is_empty() {
                            merged.detail = artifact.detail.clone();
                        }
                        if merged.url.is_none() {
                            merged.url = artifact.url.clone();
                        }
                        merged
                            .source_suggestion_ids
                            .extend(artifact.source_suggestion_ids.iter().cloned());
                        merged
                    })
                }));
            }
        }

        dedupe_resolution_artifacts(&mut artifacts);
        if artifacts.is_empty() {
            continue;
        }
        sort_resolution_artifacts(&mut artifacts, suggestion);
        if artifacts.len() > CHAT_SUGGESTION_RESOLUTION_MAX_ARTIFACTS {
            artifacts.truncate(CHAT_SUGGESTION_RESOLUTION_MAX_ARTIFACTS);
        }
        let signature = suggestion_resolution_signature(suggestion, &artifacts);

        let direct_matches = artifacts
            .iter()
            .filter(|artifact| {
                artifact
                    .source_suggestion_ids
                    .contains(suggestion.id.trim())
            })
            .cloned()
            .collect::<Vec<_>>();
        if !direct_matches.is_empty() {
            mark_suggestion_resolved_by_artifacts(
                suggestion,
                &direct_matches,
                "Existing durable work carries this suggestion's structured source identity.",
                &signature,
                &now_text,
            );
            resolved += 1;
            changed += 1;
            continue;
        }

        if recently_checked_resolution_signature(suggestion, &signature, now) {
            continue;
        }

        if let Some((selected, summary)) =
            semantically_resolved_artifacts(&agent.llm, suggestion, &artifacts).await
        {
            mark_suggestion_resolved_by_artifacts(
                suggestion, &selected, &summary, &signature, &now_text,
            );
            resolved += 1;
            changed += 1;
        } else {
            mark_resolution_checked(suggestion, &signature, &now_text);
            changed += 1;
        }
    }

    if resolved > 0 {
        *suggestions = prune_chat_suggestion_history(std::mem::take(suggestions));
    }
    changed
}

pub(super) async fn capture_run_snapshot(state: &AppState) -> SuggestionRunSnapshot {
    let (storage, watcher_rows) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.watcher_manager.list().await)
    };

    let task_rows = storage.get_tasks().await.unwrap_or_default();
    let app_rows = state.app_registry.list().await;

    SuggestionRunSnapshot {
        task_ids: task_rows
            .into_iter()
            .map(|task| (task.id.clone(), task))
            .collect(),
        watcher_ids: watcher_rows
            .into_iter()
            .map(|watcher| (watcher.id.to_string(), watcher))
            .collect(),
        app_ids: app_rows
            .into_iter()
            .filter_map(|app| {
                let rec = app.as_object()?;
                let id = rec.get("id")?.as_str()?.trim().to_string();
                if id.is_empty() {
                    None
                } else {
                    Some((id, serde_json::Value::Object(rec.clone())))
                }
            })
            .collect(),
    }
}

pub(super) async fn collect_run_outcomes(
    state: &AppState,
    before: &SuggestionRunSnapshot,
    suggestion_kind: &str,
) -> Vec<ChatSuggestionOutcome> {
    let after = capture_run_snapshot(state).await;
    let mut outcomes = Vec::new();

    for (id, task) in after.task_ids {
        if before.task_ids.contains_key(&id) {
            continue;
        }
        let title = if task.description.trim().is_empty() {
            "Created task".to_string()
        } else {
            task.description.trim().to_string()
        };
        let detail = format!("Action: {}", task.action);
        let status = normalize_task_status(&task.status);
        let created_at = task.created_at;
        outcomes.push(ChatSuggestionOutcome {
            kind: "task".to_string(),
            id,
            title,
            detail: Some(detail),
            status: Some(status),
            url: None,
            view: Some("tasks".to_string()),
            created_at: Some(created_at),
            primary: false,
        });
    }

    for (id, watcher) in after.watcher_ids {
        if before.watcher_ids.contains_key(&id) {
            continue;
        }
        let title = if watcher.description.trim().is_empty() {
            "Created watcher".to_string()
        } else {
            watcher.description.trim().to_string()
        };
        let detail = format!(
            "Polls {} every {}s",
            watcher.poll_action, watcher.interval_secs
        );
        let status = watcher_status_label(&watcher.status);
        let created_at = watcher.created_at.to_rfc3339();
        outcomes.push(ChatSuggestionOutcome {
            kind: "watcher".to_string(),
            id,
            title,
            detail: Some(detail),
            status: Some(status),
            url: None,
            view: Some("watchers".to_string()),
            created_at: Some(created_at),
            primary: false,
        });
    }

    for (id, app) in after.app_ids {
        if before.app_ids.contains_key(&id) {
            continue;
        }
        let title = app
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Deployed app")
            .to_string();
        let runtime_mode = app
            .get("runtime_mode")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        outcomes.push(ChatSuggestionOutcome {
            kind: "app".to_string(),
            id,
            title,
            detail: (!runtime_mode.is_empty()).then(|| format!("Runtime: {}", runtime_mode)),
            status: Some(
                app.get("running")
                    .and_then(|value| value.as_bool())
                    .map(|running| if running { "running" } else { "stopped" })
                    .unwrap_or("unknown")
                    .to_string(),
            ),
            url: app
                .get("access_url")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            view: Some("apps".to_string()),
            created_at: app
                .get("created_at")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            primary: false,
        });
    }

    outcomes.sort_by(|a, b| {
        outcome_kind_priority(suggestion_kind, &a.kind)
            .cmp(&outcome_kind_priority(suggestion_kind, &b.kind))
            .then_with(|| a.title.cmp(&b.title))
    });
    if let Some(first) = outcomes.first_mut() {
        first.primary = true;
    }
    outcomes
}

pub(super) async fn update_chat_suggestion_after_run(
    storage: &crate::storage::Storage,
    suggestion_id: &str,
    trace_id: &str,
    run_status: &str,
    completed_at: &str,
    last_run_error: Option<String>,
    accepted_outcomes: Vec<ChatSuggestionOutcome>,
) {
    let mut suggestions = load_chat_suggestions(storage).await;
    let Some(idx) = suggestions.iter().position(|item| item.id == suggestion_id) else {
        return;
    };

    suggestions[idx].run_status = Some(run_status.to_string());
    suggestions[idx].last_run_completed_at = Some(completed_at.to_string());
    suggestions[idx].last_run_error = last_run_error;
    suggestions[idx].updated_at = completed_at.to_string();
    suggestions[idx].accepted_goal_id = None;
    suggestions[idx].accepted_outcomes = accepted_outcomes;
    if !trace_id.trim().is_empty() {
        suggestions[idx].accepted_trace_id = Some(trace_id.to_string());
    }
    if run_status == "failed" {
        suggestions[idx].status = "open".to_string();
        suggestions[idx].accepted_at = None;
    } else {
        suggestions[idx].status = "accepted".to_string();
    }
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(storage, &suggestions).await;
}

pub(super) async fn get_autonomy_suggestion_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    let suggestions = load_chat_suggestions(&storage).await;
    match suggestions
        .into_iter()
        .find(|suggestion| suggestion.id == id)
    {
        Some(suggestion) => (
            StatusCode::OK,
            Json(serde_json::json!({ "suggestion": suggestion })),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Suggestion not found".to_string(),
            }),
        )
            .into_response(),
    }
}
