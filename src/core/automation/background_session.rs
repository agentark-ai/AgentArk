use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

const LEGACY_STORAGE_KEY: &str = "background_sessions:v1";
const MAX_EVENTS: usize = 80;
const MAX_TEXT_CHARS: usize = 8_000;
const MAX_WORKING_MEMORY_CHARS: usize = 24_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundSessionStatus {
    Draft,
    Active,
    Waiting,
    NeedsInput,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl BackgroundSessionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Waiting => "waiting",
            Self::NeedsInput => "needs_input",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundSessionEvent {
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BackgroundSessionPolicy {
    #[serde(default)]
    pub allowed_action_roles: Vec<String>,
    #[serde(default)]
    pub allowed_integration_classes: Vec<String>,
}

impl BackgroundSessionPolicy {
    pub fn normalized(mut self) -> Self {
        self.allowed_action_roles = normalize_policy_values(&self.allowed_action_roles);
        self.allowed_integration_classes =
            normalize_policy_values(&self.allowed_integration_classes);
        self
    }

    pub fn is_unset(&self) -> bool {
        self.allowed_action_roles.is_empty() && self.allowed_integration_classes.is_empty()
    }

    pub fn allows(&self, action_role: &str, integration_class: &str) -> bool {
        let role = normalize_policy_value(action_role);
        let class = normalize_policy_value(integration_class);
        (self.allowed_action_roles.is_empty() || self.allowed_action_roles.contains(&role))
            && (self.allowed_integration_classes.is_empty()
                || self.allowed_integration_classes.contains(&class))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundSession {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub status: BackgroundSessionStatus,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub waiting_on: Option<String>,
    #[serde(default)]
    pub next_expected_action: Option<String>,
    #[serde(default)]
    pub working_memory: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub preferred_delivery_channel: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub linked_task_ids: Vec<String>,
    #[serde(default)]
    pub linked_watcher_ids: Vec<String>,
    #[serde(default)]
    pub policy: BackgroundSessionPolicy,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    #[serde(default)]
    pub last_consolidated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub events: Vec<BackgroundSessionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackgroundSessionCreate {
    #[serde(default)]
    pub title: Option<String>,
    pub objective: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub waiting_on: Option<String>,
    #[serde(default)]
    pub next_expected_action: Option<String>,
    #[serde(default)]
    pub working_memory: Option<String>,
    #[serde(default)]
    pub preferred_delivery_channel: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub watcher_ids: Vec<String>,
    #[serde(default)]
    pub policy: BackgroundSessionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackgroundSessionUpdate {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub status: Option<BackgroundSessionStatus>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub waiting_on: Option<String>,
    #[serde(default)]
    pub next_expected_action: Option<String>,
    #[serde(default)]
    pub working_memory: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub preferred_delivery_channel: Option<String>,
    #[serde(default)]
    pub policy: Option<BackgroundSessionPolicy>,
}

#[derive(Clone)]
pub struct BackgroundSessionManager {
    sessions: Arc<RwLock<HashMap<String, BackgroundSession>>>,
    storage: Option<crate::storage::Storage>,
}

fn normalize_session_map(items: Vec<BackgroundSession>) -> HashMap<String, BackgroundSession> {
    items
        .into_iter()
        .map(|mut session| {
            session.policy = session.policy.normalized();
            (session.id.clone(), session)
        })
        .collect()
}

async fn load_legacy_background_sessions(
    storage_ref: &crate::storage::Storage,
) -> HashMap<String, BackgroundSession> {
    match storage_ref.get(LEGACY_STORAGE_KEY).await {
        Ok(Some(raw)) => match serde_json::from_slice::<Vec<BackgroundSession>>(&raw) {
            Ok(items) => normalize_session_map(items),
            Err(error) => {
                tracing::warn!(
                    "Failed to parse legacy background sessions; starting empty: {}",
                    error
                );
                HashMap::new()
            }
        },
        Ok(None) => HashMap::new(),
        Err(error) => {
            tracing::warn!(
                "Failed to load legacy background sessions; starting empty: {}",
                error
            );
            HashMap::new()
        }
    }
}

fn trim_to_option(value: Option<String>) -> Option<String> {
    value.and_then(|inner| {
        let trimmed = inner.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.trim().to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated.trim().to_string()
}

fn normalize_text_field(value: Option<String>, max_chars: usize) -> Option<String> {
    trim_to_option(value).map(|inner| truncate_text(&inner, max_chars))
}

fn normalize_id_list(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else if seen.insert(trimmed.to_string()) {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn normalize_policy_value(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn normalize_policy_values(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .filter_map(|value| {
            let normalized = normalize_policy_value(value);
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn derive_title(title: Option<String>, objective: &str) -> String {
    if let Some(explicit) = normalize_text_field(title, 180) {
        return explicit;
    }
    let objective = objective.trim();
    let first_sentence = objective
        .split_terminator(['.', '\n'])
        .next()
        .unwrap_or(objective)
        .trim();
    truncate_text(first_sentence, 96)
}

fn build_event(
    kind: &str,
    summary: &str,
    detail: Option<String>,
    actor: Option<&str>,
) -> BackgroundSessionEvent {
    BackgroundSessionEvent {
        id: Uuid::new_v4().to_string(),
        at: Utc::now(),
        kind: kind.to_string(),
        summary: truncate_text(summary, 240),
        detail: normalize_text_field(detail, MAX_TEXT_CHARS),
        actor: actor.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }),
    }
}

fn push_event(events: &mut Vec<BackgroundSessionEvent>, event: BackgroundSessionEvent) {
    events.push(event);
    if events.len() > MAX_EVENTS {
        let overflow = events.len() - MAX_EVENTS;
        events.drain(0..overflow);
    }
}

pub fn background_session_id_from_automation(arguments: &Value) -> Option<String> {
    arguments
        .get("_automation")
        .and_then(|value| value.get("background_session_id"))
        .and_then(|value| value.as_str())
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
}

pub fn set_background_session_id_in_automation(
    arguments: &Value,
    session_id: Option<&str>,
) -> Value {
    let mut next = if arguments.is_object() {
        arguments.clone()
    } else {
        serde_json::json!({})
    };
    let Some(root) = next.as_object_mut() else {
        return next;
    };

    let mut automation = root
        .remove("_automation")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    if let Some(id) = session_id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }) {
        automation.insert("background_session_id".to_string(), Value::String(id));
    } else {
        automation.remove("background_session_id");
    }

    if automation.is_empty() {
        root.remove("_automation");
    } else {
        root.insert("_automation".to_string(), Value::Object(automation));
    }

    next
}

impl BackgroundSessionManager {
    pub async fn new(storage: Option<crate::storage::Storage>) -> Self {
        let sessions = if let Some(storage_ref) = storage.as_ref() {
            match storage_ref.list_background_sessions().await {
                Ok(items) if !items.is_empty() => normalize_session_map(items),
                Ok(_) => {
                    let legacy = load_legacy_background_sessions(storage_ref).await;
                    if !legacy.is_empty() {
                        for session in legacy.values() {
                            if let Err(error) = storage_ref.upsert_background_session(session).await
                            {
                                tracing::warn!(
                                    "Failed to import legacy background session {} into row storage: {}",
                                    session.id,
                                    error
                                );
                            }
                        }
                        if let Err(error) = storage_ref.delete(LEGACY_STORAGE_KEY).await {
                            tracing::debug!(
                                "Failed to clear legacy background-session blob after import: {}",
                                error
                            );
                        }
                    }
                    legacy
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to load persisted background sessions from row storage; falling back to legacy blob: {}",
                        error
                    );
                    load_legacy_background_sessions(storage_ref).await
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            sessions: Arc::new(RwLock::new(sessions)),
            storage,
        }
    }

    async fn persist_session(&self, session: &BackgroundSession) {
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        if let Err(error) = storage.upsert_background_session(session).await {
            tracing::warn!(
                "Failed to persist background session {} to row storage: {}",
                session.id,
                error
            );
        }
    }

    async fn persist_sessions(&self, sessions: &[BackgroundSession]) {
        if sessions.is_empty() {
            return;
        }
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        for session in sessions {
            if let Err(error) = storage.upsert_background_session(session).await {
                tracing::warn!(
                    "Failed to persist background session {} to row storage: {}",
                    session.id,
                    error
                );
            }
        }
    }

    async fn delete_persisted_session(&self, id: &str) {
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        if let Err(error) = storage.delete_background_session(id).await {
            tracing::warn!(
                "Failed to delete persisted background session {} from row storage: {}",
                id,
                error
            );
        }
    }

    async fn persist_changed_ids(&self, changed_ids: &[String]) {
        if changed_ids.is_empty() {
            return;
        }
        let snapshots = {
            let sessions = self.sessions.read().await;
            changed_ids
                .iter()
                .filter_map(|id| sessions.get(id).cloned())
                .collect::<Vec<_>>()
        };
        self.persist_sessions(&snapshots).await;
    }

    pub async fn list(&self) -> Vec<BackgroundSession> {
        let mut sessions: Vec<_> = self.sessions.read().await.values().cloned().collect();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        sessions
    }

    pub async fn get(&self, id: &str) -> Option<BackgroundSession> {
        let normalized = id.trim();
        if normalized.is_empty() {
            return None;
        }
        self.sessions.read().await.get(normalized).cloned()
    }

    pub async fn list_for_conversation(
        &self,
        conversation_id: &str,
        include_closed: bool,
    ) -> Vec<BackgroundSession> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Vec::new();
        }
        let mut sessions = self
            .sessions
            .read()
            .await
            .values()
            .filter(|session| {
                (include_closed || !session.status.is_closed())
                    && session.conversation_id.as_deref().map(str::trim) == Some(conversation_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        sessions
    }

    pub async fn create(
        &self,
        request: BackgroundSessionCreate,
        actor: Option<&str>,
    ) -> BackgroundSession {
        let now = Utc::now();
        let objective = truncate_text(request.objective.trim(), 600);
        let mut session = BackgroundSession {
            id: Uuid::new_v4().to_string(),
            title: derive_title(request.title, &objective),
            objective,
            status: BackgroundSessionStatus::Active,
            summary: normalize_text_field(request.summary, MAX_TEXT_CHARS),
            current_focus: normalize_text_field(request.current_focus, MAX_TEXT_CHARS),
            waiting_on: normalize_text_field(request.waiting_on, MAX_TEXT_CHARS),
            next_expected_action: normalize_text_field(
                request.next_expected_action,
                MAX_TEXT_CHARS,
            ),
            working_memory: normalize_text_field(request.working_memory, MAX_WORKING_MEMORY_CHARS),
            last_error: None,
            preferred_delivery_channel: normalize_text_field(
                request.preferred_delivery_channel,
                120,
            ),
            channel: normalize_text_field(request.channel, 80),
            conversation_id: normalize_text_field(request.conversation_id, 120),
            project_id: None,
            linked_task_ids: normalize_id_list(&request.task_ids),
            linked_watcher_ids: normalize_id_list(&request.watcher_ids),
            policy: request.policy.normalized(),
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            last_consolidated_at: None,
            events: Vec::new(),
        };
        push_event(
            &mut session.events,
            build_event("created", "Background session created.", None, actor),
        );

        self.sessions
            .write()
            .await
            .insert(session.id.clone(), session.clone());
        self.persist_session(&session).await;
        session
    }

    pub async fn update(
        &self,
        id: &str,
        request: BackgroundSessionUpdate,
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            let mut changed_fields: Vec<&str> = Vec::new();

            if let Some(title) = normalize_text_field(request.title, 180) {
                if title != session.title {
                    session.title = title;
                    changed_fields.push("title");
                }
            }
            if let Some(objective) = normalize_text_field(request.objective, 600) {
                if objective != session.objective {
                    session.objective = objective;
                    changed_fields.push("objective");
                }
            }
            if let Some(status) = request.status {
                if status != session.status {
                    session.status = status;
                    changed_fields.push("status");
                }
            }

            let maybe_update_text = |slot: &mut Option<String>,
                                     next: Option<String>,
                                     changed_fields: &mut Vec<&str>,
                                     label: &'static str,
                                     max_chars: usize| {
                let Some(raw) = next else {
                    return;
                };
                let normalized = normalize_text_field(Some(raw), max_chars);
                if *slot != normalized {
                    *slot = normalized;
                    changed_fields.push(label);
                }
            };

            maybe_update_text(
                &mut session.summary,
                request.summary,
                &mut changed_fields,
                "summary",
                MAX_TEXT_CHARS,
            );
            maybe_update_text(
                &mut session.current_focus,
                request.current_focus,
                &mut changed_fields,
                "current_focus",
                MAX_TEXT_CHARS,
            );
            maybe_update_text(
                &mut session.waiting_on,
                request.waiting_on,
                &mut changed_fields,
                "waiting_on",
                MAX_TEXT_CHARS,
            );
            maybe_update_text(
                &mut session.next_expected_action,
                request.next_expected_action,
                &mut changed_fields,
                "next_expected_action",
                MAX_TEXT_CHARS,
            );
            maybe_update_text(
                &mut session.working_memory,
                request.working_memory,
                &mut changed_fields,
                "working_memory",
                MAX_WORKING_MEMORY_CHARS,
            );
            maybe_update_text(
                &mut session.last_error,
                request.last_error,
                &mut changed_fields,
                "last_error",
                MAX_TEXT_CHARS,
            );
            maybe_update_text(
                &mut session.preferred_delivery_channel,
                request.preferred_delivery_channel,
                &mut changed_fields,
                "preferred_delivery_channel",
                120,
            );
            if let Some(policy) = request.policy {
                let policy = policy.normalized();
                if policy != session.policy {
                    session.policy = policy;
                    changed_fields.push("policy");
                }
            }

            if changed_fields.is_empty() {
                return Some(session.clone());
            }

            let now = Utc::now();
            session.updated_at = now;
            session.last_activity_at = now;
            push_event(
                &mut session.events,
                build_event(
                    "updated",
                    "Session details updated.",
                    Some(format!("Changed: {}", changed_fields.join(", "))),
                    actor,
                ),
            );
            Some(session.clone())
        }?;

        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn set_status(
        &self,
        id: &str,
        status: BackgroundSessionStatus,
        summary: &str,
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            if session.status == status {
                return Some(session.clone());
            }
            session.status = status;
            let now = Utc::now();
            session.updated_at = now;
            session.last_activity_at = now;
            push_event(
                &mut session.events,
                build_event("status_changed", summary, None, actor),
            );
            Some(session.clone())
        }?;
        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn bind_runtime_context(
        &self,
        id: &str,
        channel: Option<&str>,
        conversation_id: Option<&str>,
        _project_id: Option<&str>,
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            let mut changed = false;

            let normalized_channel =
                normalize_text_field(channel.map(|value| value.to_string()), 80);
            if normalized_channel.is_some() && session.channel != normalized_channel {
                session.channel = normalized_channel;
                changed = true;
            }

            let normalized_conversation =
                normalize_text_field(conversation_id.map(|value| value.to_string()), 120);
            if normalized_conversation.is_some()
                && session.conversation_id != normalized_conversation
            {
                session.conversation_id = normalized_conversation;
                changed = true;
            }

            if !changed {
                return Some(session.clone());
            }

            let now = Utc::now();
            session.updated_at = now;
            session.last_activity_at = now;
            push_event(
                &mut session.events,
                build_event(
                    "bound",
                    "Background session linked to the current runtime context.",
                    None,
                    actor,
                ),
            );
            Some(session.clone())
        }?;

        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn apply_chat_turn(
        &self,
        id: &str,
        user_message: &str,
        assistant_response: &str,
        next_expected_action: Option<String>,
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            let mut changed = false;

            let focus = normalize_text_field(Some(user_message.to_string()), MAX_TEXT_CHARS);
            if focus.is_some() && session.current_focus != focus {
                session.current_focus = focus;
                changed = true;
            }

            if session.waiting_on.is_some() {
                session.waiting_on = None;
                changed = true;
            }

            let response_preview = normalize_text_field(Some(assistant_response.to_string()), 420);
            if response_preview.is_some() && session.summary.is_none() {
                session.summary = response_preview;
                changed = true;
            }

            let normalized_next = normalize_text_field(next_expected_action, 320);
            if normalized_next.is_some() && session.next_expected_action != normalized_next {
                session.next_expected_action = normalized_next;
                changed = true;
            }

            if !changed {
                return Some(session.clone());
            }

            let now = Utc::now();
            session.updated_at = now;
            session.last_activity_at = now;
            push_event(
                &mut session.events,
                build_event(
                    "chat_turn",
                    "Background session updated from the latest chat turn.",
                    None,
                    actor,
                ),
            );
            Some(session.clone())
        }?;

        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn record_consolidation(
        &self,
        id: &str,
        summary: Option<String>,
        working_memory: String,
        next_expected_action: Option<String>,
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            let normalized_summary = normalize_text_field(summary, MAX_TEXT_CHARS);
            let normalized_memory =
                normalize_text_field(Some(working_memory), MAX_WORKING_MEMORY_CHARS);
            let normalized_next = normalize_text_field(next_expected_action, 320);

            let mut changed = false;
            if normalized_summary.is_some() && session.summary != normalized_summary {
                session.summary = normalized_summary;
                changed = true;
            }
            if session.working_memory != normalized_memory {
                session.working_memory = normalized_memory;
                changed = true;
            }
            if normalized_next.is_some() && session.next_expected_action != normalized_next {
                session.next_expected_action = normalized_next;
                changed = true;
            }

            let now = Utc::now();
            if !changed
                && session
                    .last_consolidated_at
                    .map(|value| (now - value).num_minutes() < 1)
                    .unwrap_or(false)
            {
                return Some(session.clone());
            }

            session.updated_at = now;
            session.last_activity_at = now;
            session.last_consolidated_at = Some(now);
            push_event(
                &mut session.events,
                build_event(
                    "consolidated",
                    "Background session memory consolidated from recent evidence.",
                    None,
                    actor,
                ),
            );
            Some(session.clone())
        }?;

        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn attach_items(
        &self,
        id: &str,
        task_ids: &[String],
        watcher_ids: &[String],
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            let mut changed = false;

            for task_id in normalize_id_list(task_ids) {
                if !session
                    .linked_task_ids
                    .iter()
                    .any(|existing| existing == &task_id)
                {
                    session.linked_task_ids.push(task_id);
                    changed = true;
                }
            }
            for watcher_id in normalize_id_list(watcher_ids) {
                if !session
                    .linked_watcher_ids
                    .iter()
                    .any(|existing| existing == &watcher_id)
                {
                    session.linked_watcher_ids.push(watcher_id);
                    changed = true;
                }
            }

            if !changed {
                return Some(session.clone());
            }

            let now = Utc::now();
            session.updated_at = now;
            session.last_activity_at = now;
            push_event(
                &mut session.events,
                build_event("attached", "Linked work added to the session.", None, actor),
            );
            Some(session.clone())
        }?;
        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn detach_items(
        &self,
        id: &str,
        task_ids: &[String],
        watcher_ids: &[String],
        actor: Option<&str>,
    ) -> Option<BackgroundSession> {
        let updated = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(id.trim())?;
            let before_task_len = session.linked_task_ids.len();
            let before_watcher_len = session.linked_watcher_ids.len();
            let task_ids = normalize_id_list(task_ids);
            let watcher_ids = normalize_id_list(watcher_ids);
            session
                .linked_task_ids
                .retain(|value| !task_ids.iter().any(|id| id == value));
            session
                .linked_watcher_ids
                .retain(|value| !watcher_ids.iter().any(|id| id == value));
            if session.linked_task_ids.len() == before_task_len
                && session.linked_watcher_ids.len() == before_watcher_len
            {
                return Some(session.clone());
            }

            let now = Utc::now();
            session.updated_at = now;
            session.last_activity_at = now;
            push_event(
                &mut session.events,
                build_event(
                    "detached",
                    "Linked work removed from the session.",
                    None,
                    actor,
                ),
            );
            Some(session.clone())
        }?;
        self.persist_session(&updated).await;
        Some(updated)
    }

    pub async fn remove_child_references(
        &self,
        task_ids: &[String],
        watcher_ids: &[String],
        actor: Option<&str>,
    ) {
        let task_ids = normalize_id_list(task_ids);
        let watcher_ids = normalize_id_list(watcher_ids);
        if task_ids.is_empty() && watcher_ids.is_empty() {
            return;
        }

        let mut changed_ids = Vec::new();
        {
            let mut sessions = self.sessions.write().await;
            for session in sessions.values_mut() {
                let before_task_len = session.linked_task_ids.len();
                let before_watcher_len = session.linked_watcher_ids.len();
                session
                    .linked_task_ids
                    .retain(|value| !task_ids.iter().any(|id| id == value));
                session
                    .linked_watcher_ids
                    .retain(|value| !watcher_ids.iter().any(|id| id == value));
                if session.linked_task_ids.len() != before_task_len
                    || session.linked_watcher_ids.len() != before_watcher_len
                {
                    let now = Utc::now();
                    session.updated_at = now;
                    session.last_activity_at = now;
                    push_event(
                        &mut session.events,
                        build_event("rebound", "Linked work was moved or removed.", None, actor),
                    );
                    if session.linked_task_ids.is_empty()
                        && session.linked_watcher_ids.is_empty()
                        && !session.status.is_closed()
                    {
                        session.status = BackgroundSessionStatus::Completed;
                        push_event(
                            &mut session.events,
                            build_event(
                                "completed",
                                "Background session closed after all linked work moved elsewhere.",
                                None,
                                actor,
                            ),
                        );
                    }
                    changed_ids.push(session.id.clone());
                }
            }
        }

        if !changed_ids.is_empty() {
            self.persist_changed_ids(&changed_ids).await;
        }
    }

    pub async fn delete(&self, id: &str) -> Option<BackgroundSession> {
        let normalized = id.trim();
        let removed = self.sessions.write().await.remove(normalized);
        if removed.is_some() {
            self.delete_persisted_session(normalized).await;
        }
        removed
    }
}
