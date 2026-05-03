//! Browser session manager for LLM-driven browser automation.
//!
//! Sessions are long-running background tasks that control the Playwright bridge,
//! pause for explicit operator handoff when the browser needs a human, and keep
//! enough durable state to survive restarts.

use anyhow::{Result, anyhow};
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::integrations::browser::{BrowserIntegration, BrowserSidecarSessionState, PageContent};

const MAX_ITERATIONS: u32 = 30;
const MAX_PERSISTED_ACTION_HISTORY: usize = 80;
const OPERATOR_HANDOFF_TIMEOUT_SECS: u64 = 30 * 60;
const LIVE_SESSION_IDLE_TIMEOUT_SECS: i64 = 15 * 60;
const LIVE_SESSION_IDLE_WARNING_SECS: i64 = LIVE_SESSION_IDLE_TIMEOUT_SECS - 60;
const IDLE_WATCHDOG_POLL_SECS: u64 = 15;
const INTERRUPTED_BROWSER_SESSION_REASON: &str =
    "Browser session was interrupted by an app restart before it could finish.";
const INTERRUPTED_BROWSER_HANDOFF_REASON: &str =
    "Browser handoff was interrupted by an app restart before it could finish.";
const INTERRUPTED_READY_SESSION_REASON: &str =
    "Browser session ended during an app restart. Restart the browser task.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedBrowserSession {
    pub id: String,
    pub status: String,
    pub task_description: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    #[serde(default)]
    pub action_history: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub enum SessionStatus {
    Active,
    WaitingForOperator { question: String },
    OperatorClaimed { question: String },
    Ready { summary: String },
    AwaitingResume { question: String },
    Interrupted { reason: String },
    Completed { summary: String },
    Failed(String),
}

#[derive(Debug)]
struct OperatorHandoffOutcome {
    note: String,
}

pub struct BrowserSession {
    pub id: String,
    pub sidecar_session_id: String,
    pub channel: String,
    pub conversation_id: Option<String>,
    pub task_description: String,
    pub status: SessionStatus,
    pub action_history: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    operator_handoff_tx: Option<oneshot::Sender<OperatorHandoffOutcome>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSessionNotificationKind {
    Progress,
    NeedsInput,
    Notice,
    Completed,
    Failed,
    Closed,
}

#[derive(Debug, Clone)]
pub struct BrowserSessionNotification {
    pub session_id: String,
    pub kind: BrowserSessionNotificationKind,
    pub message: String,
    pub screenshot: Option<Vec<u8>>,
}

impl BrowserSessionNotification {
    fn progress(session_id: &str, message: impl Into<String>) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind: BrowserSessionNotificationKind::Progress,
            message: message.into(),
            screenshot: None,
        }
    }

    fn needs_input(
        session_id: &str,
        message: impl Into<String>,
        screenshot: Option<Vec<u8>>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind: BrowserSessionNotificationKind::NeedsInput,
            message: message.into(),
            screenshot,
        }
    }

    fn notice(session_id: &str, message: impl Into<String>) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind: BrowserSessionNotificationKind::Notice,
            message: message.into(),
            screenshot: None,
        }
    }

    fn completed(
        session_id: &str,
        message: impl Into<String>,
        screenshot: Option<Vec<u8>>,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind: BrowserSessionNotificationKind::Completed,
            message: message.into(),
            screenshot,
        }
    }

    fn failed(session_id: &str, message: impl Into<String>, screenshot: Option<Vec<u8>>) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind: BrowserSessionNotificationKind::Failed,
            message: message.into(),
            screenshot,
        }
    }

    fn closed(session_id: &str, message: impl Into<String>) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind: BrowserSessionNotificationKind::Closed,
            message: message.into(),
            screenshot: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserSessionView {
    pub id: String,
    pub task_description: String,
    pub status: String,
    pub question: Option<String>,
    pub summary: Option<String>,
    pub reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    pub live_view_enabled: bool,
    pub live_view_port: Option<u16>,
    pub live_view_path: Option<String>,
    pub can_claim: bool,
    pub can_release: bool,
    pub can_complete: bool,
}

#[derive(Clone)]
pub struct BrowserSessionManager {
    sessions: Arc<DashMap<String, BrowserSession>>,
    integration: Arc<BrowserIntegration>,
    storage: Option<crate::storage::Storage>,
}

pub struct StartedBrowserSession {
    pub session_id: String,
    pub reused_existing: bool,
}

impl BrowserSessionManager {
    pub async fn new(storage: Option<crate::storage::Storage>) -> Self {
        let manager = Self {
            sessions: Arc::new(DashMap::new()),
            integration: Arc::new(BrowserIntegration::new()),
            storage,
        };
        manager.restore_persisted_sessions().await;
        manager
    }

    pub async fn is_available(&self) -> bool {
        self.integration.is_available().await
    }

    pub async fn start_session(
        &self,
        task: &str,
        channel: &str,
        conversation_id: Option<&str>,
        llm_client: super::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) -> Result<StartedBrowserSession> {
        self.cleanup_stale_sessions().await;
        self.prune_unreachable_live_sessions().await;
        let conversation_id = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(existing_session_id) = self
            .take_reusable_ready_session(conversation_id.as_deref(), task, channel)
            .await
        {
            self.spawn_session_loop(existing_session_id.clone(), llm_client, notify_fn);
            return Ok(StartedBrowserSession {
                session_id: existing_session_id,
                reused_existing: true,
            });
        }
        if let Some(existing_session_id) = self
            .latest_managed_live_session_for_conversation(conversation_id.as_deref())
            .await
        {
            return Ok(StartedBrowserSession {
                session_id: existing_session_id,
                reused_existing: true,
            });
        }

        if self.active_count() >= 2 {
            anyhow::bail!("Maximum 2 concurrent browser sessions");
        }

        let sidecar_id = self.integration.create_session().await?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let created_at = now_rfc3339();

        self.sessions.insert(
            session_id.clone(),
            BrowserSession {
                id: session_id.clone(),
                sidecar_session_id: sidecar_id.clone(),
                channel: channel.to_string(),
                conversation_id,
                task_description: task.to_string(),
                status: SessionStatus::Active,
                action_history: Vec::new(),
                created_at: created_at.clone(),
                updated_at: created_at,
                operator_handoff_tx: None,
            },
        );
        self.persist_session(&session_id).await;
        self.spawn_idle_watchdog(session_id.clone(), notify_fn.clone());
        self.spawn_session_loop(session_id.clone(), llm_client, notify_fn);
        Ok(StartedBrowserSession {
            session_id,
            reused_existing: false,
        })
    }

    pub async fn describe_session(&self, session_id: &str) -> Option<BrowserSessionView> {
        let (id, sidecar_session_id, task_description, status, created_at, updated_at) =
            if let Some(entry) = self.sessions.get(session_id) {
                (
                    entry.id.clone(),
                    entry.sidecar_session_id.clone(),
                    entry.task_description.clone(),
                    entry.status.clone(),
                    entry.created_at.clone(),
                    entry.updated_at.clone(),
                )
            } else {
                let storage = self.storage.as_ref()?;
                let persisted = storage.load_browser_session(session_id).await.ok()??;
                let (session, _changed) = BrowserSession::restore_from_persisted(persisted);
                (
                    session.id,
                    session.sidecar_session_id,
                    session.task_description,
                    session.status,
                    session.created_at,
                    session.updated_at,
                )
            };

        let sidecar_state =
            if session_status_has_live_session(&status) && !sidecar_session_id.trim().is_empty() {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(1500),
                    self.integration.get_session_state(&sidecar_session_id),
                )
                .await
                {
                    Ok(Ok(state)) => Some(state),
                    Ok(Err(error)) => {
                        tracing::warn!(
                            "Browser handoff: failed to load sidecar session state for {}: {}",
                            &id,
                            error
                        );
                        None
                    }
                    Err(_) => {
                        tracing::warn!(
                            "Browser handoff: timed out loading sidecar session state for {}",
                            &id
                        );
                        None
                    }
                }
            } else {
                None
            };

        Some(build_browser_session_view(
            id,
            task_description,
            status,
            created_at,
            updated_at,
            sidecar_state,
        ))
    }

    pub async fn list_session_views(&self) -> Vec<BrowserSessionView> {
        self.cleanup_stale_sessions().await;
        self.prune_unreachable_live_sessions().await;

        let mut sessions = self
            .sessions
            .iter()
            .map(|entry| {
                build_browser_session_view(
                    entry.id.clone(),
                    entry.task_description.clone(),
                    entry.status.clone(),
                    entry.created_at.clone(),
                    entry.updated_at.clone(),
                    None,
                )
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        sessions
    }

    pub async fn claim_operator_handoff(&self, session_id: &str) -> Result<BrowserSessionView> {
        let sidecar_session_id = self
            .sessions
            .get(session_id)
            .map(|entry| entry.sidecar_session_id.clone())
            .ok_or_else(|| anyhow!("Browser session not found"))?;

        let sidecar_state = self.integration.claim_session(&sidecar_session_id).await?;
        let mut view_parts = None;
        let snapshot = if let Some(mut entry) = self.sessions.get_mut(session_id) {
            match &entry.status {
                SessionStatus::WaitingForOperator { question } => {
                    entry.status = SessionStatus::OperatorClaimed {
                        question: question.clone(),
                    };
                }
                SessionStatus::Ready { summary } => {
                    entry.status = SessionStatus::OperatorClaimed {
                        question: format!(
                            "The browser is ready for manual control. {}",
                            summary.trim()
                        ),
                    };
                }
                SessionStatus::OperatorClaimed { .. } => {}
                SessionStatus::AwaitingResume { .. } => {
                    return Err(anyhow!(
                        "Browser handoff was interrupted by a restart. Restart the browser task."
                    ));
                }
                _ => {
                    return Err(anyhow!(
                        "Browser session is not waiting for live operator handoff"
                    ));
                }
            }
            entry.updated_at = now_rfc3339();
            view_parts = Some((
                entry.id.clone(),
                entry.task_description.clone(),
                entry.status.clone(),
                entry.created_at.clone(),
                entry.updated_at.clone(),
            ));
            Some(PersistedBrowserSession::from_session(&entry))
        } else {
            None
        };
        if let Some(snapshot) = snapshot {
            persist_browser_session(self.storage.as_ref(), &snapshot).await;
        }

        let (id, task_description, status, created_at, updated_at) =
            view_parts.ok_or_else(|| anyhow!("Browser session not found"))?;
        Ok(build_browser_session_view(
            id,
            task_description,
            status,
            created_at,
            updated_at,
            Some(sidecar_state),
        ))
    }

    pub async fn release_operator_handoff(&self, session_id: &str) -> Result<BrowserSessionView> {
        let sidecar_session_id = self
            .sessions
            .get(session_id)
            .map(|entry| entry.sidecar_session_id.clone())
            .ok_or_else(|| anyhow!("Browser session not found"))?;

        let sidecar_state = self
            .integration
            .release_session(&sidecar_session_id)
            .await?;
        let mut view_parts = None;
        let snapshot = if let Some(mut entry) = self.sessions.get_mut(session_id) {
            match &entry.status {
                SessionStatus::OperatorClaimed { question } => {
                    entry.status = SessionStatus::WaitingForOperator {
                        question: question.clone(),
                    };
                }
                SessionStatus::WaitingForOperator { .. } => {}
                SessionStatus::AwaitingResume { .. } => {
                    return Err(anyhow!(
                        "Browser handoff was interrupted by a restart. Restart the browser task."
                    ));
                }
                _ => {
                    return Err(anyhow!(
                        "Browser session is not currently claimed for live operator handoff"
                    ));
                }
            }
            entry.updated_at = now_rfc3339();
            view_parts = Some((
                entry.id.clone(),
                entry.task_description.clone(),
                entry.status.clone(),
                entry.created_at.clone(),
                entry.updated_at.clone(),
            ));
            Some(PersistedBrowserSession::from_session(&entry))
        } else {
            None
        };
        if let Some(snapshot) = snapshot {
            persist_browser_session(self.storage.as_ref(), &snapshot).await;
        }

        let (id, task_description, status, created_at, updated_at) =
            view_parts.ok_or_else(|| anyhow!("Browser session not found"))?;
        Ok(build_browser_session_view(
            id,
            task_description,
            status,
            created_at,
            updated_at,
            Some(sidecar_state),
        ))
    }

    pub async fn complete_operator_handoff(
        &self,
        session_id: &str,
        note: &str,
    ) -> Result<BrowserSessionView> {
        let sidecar_session_id = self
            .sessions
            .get(session_id)
            .map(|entry| entry.sidecar_session_id.clone())
            .ok_or_else(|| anyhow!("Browser session not found"))?;

        let sidecar_state = self
            .integration
            .release_session(&sidecar_session_id)
            .await
            .ok();
        let (tx, view_parts, snapshot) = if let Some(mut entry) = self.sessions.get_mut(session_id)
        {
            match &entry.status {
                SessionStatus::OperatorClaimed { .. } => {}
                SessionStatus::WaitingForOperator { .. } => {
                    return Err(anyhow!(
                        "Claim the live browser before handing it back to AgentArk"
                    ));
                }
                SessionStatus::AwaitingResume { .. } => {
                    return Err(anyhow!(
                        "Browser handoff was interrupted by a restart. Restart the browser task."
                    ));
                }
                _ => {
                    return Err(anyhow!(
                        "Browser session is not waiting for a live handoff completion"
                    ));
                }
            }

            let tx = entry.operator_handoff_tx.take().ok_or_else(|| {
                anyhow!("Browser session is no longer waiting for a live operator handoff")
            })?;
            entry.status = SessionStatus::Active;
            entry.updated_at = now_rfc3339();
            let parts = (
                entry.id.clone(),
                entry.task_description.clone(),
                entry.status.clone(),
                entry.created_at.clone(),
                entry.updated_at.clone(),
            );
            let snapshot = PersistedBrowserSession::from_session(&entry);
            (tx, parts, snapshot)
        } else {
            return Err(anyhow!("Browser session not found"));
        };

        if tx
            .send(OperatorHandoffOutcome {
                note: note.trim().to_string(),
            })
            .is_err()
        {
            let snapshot = if let Some(mut entry) = self.sessions.get_mut(session_id) {
                entry.status = SessionStatus::Failed(
                    "Browser handoff could not resume because the browser loop stopped."
                        .to_string(),
                );
                entry.updated_at = now_rfc3339();
                Some(PersistedBrowserSession::from_session(&entry))
            } else {
                None
            };
            if let Some(snapshot) = snapshot {
                persist_browser_session(self.storage.as_ref(), &snapshot).await;
            }
            return Err(anyhow!(
                "Browser handoff could not resume because the browser loop stopped"
            ));
        }

        persist_browser_session(self.storage.as_ref(), &snapshot).await;
        let (id, task_description, status, created_at, updated_at) = view_parts;
        Ok(build_browser_session_view(
            id,
            task_description,
            status,
            created_at,
            updated_at,
            sidecar_state,
        ))
    }

    pub fn list_sessions(&self) -> Vec<(String, String, String)> {
        self.sessions
            .iter()
            .map(|entry| {
                (
                    entry.id.clone(),
                    entry.task_description.clone(),
                    entry.status.kind().to_string(),
                )
            })
            .collect()
    }

    pub async fn stop_session(&self, session_id: &str) -> Result<BrowserSessionView> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(anyhow!("Browser session not found"));
        }

        let (view, sidecar_session_id, snapshot) =
            if let Some(mut entry) = self.sessions.get_mut(session_id) {
                entry.status = SessionStatus::Interrupted {
                    reason: "Browser session stopped by user.".to_string(),
                };
                entry.operator_handoff_tx = None;
                entry.updated_at = now_rfc3339();
                let view = build_browser_session_view(
                    entry.id.clone(),
                    entry.task_description.clone(),
                    entry.status.clone(),
                    entry.created_at.clone(),
                    entry.updated_at.clone(),
                    None,
                );
                (
                    view,
                    entry.sidecar_session_id.clone(),
                    Some(PersistedBrowserSession::from_session(&entry)),
                )
            } else if let Some(storage) = self.storage.as_ref() {
                let Some(persisted) = storage.load_browser_session(session_id).await? else {
                    return Err(anyhow!("Browser session not found"));
                };
                let (session, changed) = BrowserSession::restore_from_persisted(persisted);
                let snapshot = changed.then(|| PersistedBrowserSession::from_session(&session));
                (
                    build_browser_session_view(
                        session.id.clone(),
                        session.task_description.clone(),
                        session.status.clone(),
                        session.created_at.clone(),
                        session.updated_at.clone(),
                        None,
                    ),
                    session.sidecar_session_id,
                    snapshot,
                )
            } else {
                return Err(anyhow!("Browser session not found"));
            };

        if let Some(snapshot) = snapshot {
            persist_browser_session(self.storage.as_ref(), &snapshot).await;
        }
        self.sessions.remove(session_id);
        self.close_sidecar_best_effort(&sidecar_session_id).await;
        Ok(view)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<bool> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Ok(false);
        }

        let removed_sidecar_session_id = self
            .sessions
            .remove(session_id)
            .map(|(_, mut session)| {
                session.operator_handoff_tx = None;
                session.sidecar_session_id
            })
            .unwrap_or_default();

        let existed_in_storage = if let Some(storage) = self.storage.as_ref() {
            storage.load_browser_session(session_id).await?.is_some()
        } else {
            false
        };

        self.close_sidecar_best_effort(&removed_sidecar_session_id)
            .await;
        delete_persisted_browser_session(self.storage.as_ref(), session_id).await;
        Ok(!removed_sidecar_session_id.is_empty() || existed_in_storage)
    }

    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|entry| session_counts_against_live_limit(&entry.status))
            .count()
    }

    async fn cleanup_stale_sessions(&self) {
        let stale_sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|entry| {
                session_should_be_cleaned_up(&entry.status)
                    || ready_session_is_expired(entry.value())
            })
            .map(|entry| (entry.id.clone(), entry.status.kind().to_string()))
            .collect();

        for (session_id, status) in stale_sessions {
            self.sessions.remove(&session_id);
            tracing::info!(
                "Cleaning up stale browser session '{}' with status '{}'",
                session_id,
                status
            );
            delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
        }
    }

    async fn prune_unreachable_live_sessions(&self) {
        let live_sessions = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (session_status_has_live_session(&entry.status)
                    && !entry.sidecar_session_id.trim().is_empty())
                .then(|| (entry.id.clone(), entry.sidecar_session_id.clone()))
            })
            .collect::<Vec<_>>();

        for (session_id, sidecar_session_id) in live_sessions {
            let missing = match tokio::time::timeout(
                tokio::time::Duration::from_millis(1500),
                self.integration.get_session_state(&sidecar_session_id),
            )
            .await
            {
                Ok(Err(error)) => browser_sidecar_session_missing_error(&error),
                _ => false,
            };
            if !missing {
                continue;
            }

            tracing::info!(
                "Cleaning up unreachable browser session '{}' because sidecar session '{}' is missing",
                session_id,
                sidecar_session_id
            );
            self.sessions.remove(&session_id);
            delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
        }
    }

    async fn touch_live_session(&self, session_id: &str) -> Option<String> {
        let touched = if let Some(mut entry) = self.sessions.get_mut(session_id) {
            if !session_status_has_live_session(&entry.status)
                || entry.sidecar_session_id.trim().is_empty()
            {
                return None;
            }
            entry.updated_at = now_rfc3339();
            let sidecar_session_id = entry.sidecar_session_id.clone();
            let snapshot = PersistedBrowserSession::from_session(&entry);
            Some((sidecar_session_id, snapshot))
        } else {
            None
        };
        let (sidecar_session_id, snapshot) = touched?;
        persist_browser_session(self.storage.as_ref(), &snapshot).await;
        Some(sidecar_session_id)
    }

    async fn latest_managed_live_session_for_conversation(
        &self,
        conversation_id: Option<&str>,
    ) -> Option<String> {
        let conversation_id = conversation_id?.trim();
        if conversation_id.is_empty() {
            return None;
        }
        let mut candidates = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (entry.conversation_id.as_deref() == Some(conversation_id)
                    && session_status_has_live_session(&entry.status))
                .then(|| (entry.id.clone(), entry.updated_at.clone()))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.1.cmp(&left.1));
        for (managed_session_id, _) in candidates {
            if self
                .touch_live_session(managed_session_id.as_str())
                .await
                .is_some()
            {
                return Some(managed_session_id);
            }
        }
        None
    }

    async fn close_sidecar_best_effort(&self, sidecar_session_id: &str) {
        let sidecar_session_id = sidecar_session_id.trim();
        if sidecar_session_id.is_empty() {
            return;
        }
        if let Err(error) = self.integration.close_session(sidecar_session_id).await {
            if !browser_sidecar_session_missing_error(&error) {
                tracing::warn!(
                    "Failed to close browser sidecar session '{}': {}",
                    sidecar_session_id,
                    error
                );
            }
        }
    }

    async fn persist_session(&self, session_id: &str) {
        let snapshot = self
            .sessions
            .get(session_id)
            .map(|entry| PersistedBrowserSession::from_session(&entry));
        if let Some(snapshot) = snapshot {
            persist_browser_session(self.storage.as_ref(), &snapshot).await;
        }
    }

    async fn restore_persisted_sessions(&self) {
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        let restored = match storage.list_browser_sessions().await {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!("Failed to restore browser sessions: {}", error);
                return;
            }
        };
        for persisted in restored {
            let (session, changed) = BrowserSession::restore_from_persisted(persisted);
            let session_id = session.id.clone();
            let snapshot = changed.then(|| PersistedBrowserSession::from_session(&session));
            if !session_status_is_terminal(&session.status) {
                self.sessions.insert(session_id, session);
            }
            if let Some(snapshot) = snapshot {
                persist_browser_session(Some(storage), &snapshot).await;
            }
        }
    }

    async fn take_reusable_ready_session(
        &self,
        conversation_id: Option<&str>,
        task: &str,
        channel: &str,
    ) -> Option<String> {
        let conversation_id = conversation_id?.trim();
        if conversation_id.is_empty() {
            return None;
        }

        let mut candidates = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (entry.conversation_id.as_deref() == Some(conversation_id)
                    && matches!(entry.status, SessionStatus::Ready { .. }))
                .then(|| {
                    (
                        entry.id.clone(),
                        entry.sidecar_session_id.clone(),
                        entry.updated_at.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.2.cmp(&left.2));

        for (session_id, sidecar_session_id, _) in candidates {
            if sidecar_session_id.trim().is_empty() {
                self.sessions.remove(&session_id);
                delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
                continue;
            }

            let sidecar_alive = tokio::time::timeout(
                tokio::time::Duration::from_millis(1500),
                self.integration.get_session_state(&sidecar_session_id),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .is_some();
            if !sidecar_alive {
                self.sessions.remove(&session_id);
                delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
                continue;
            }

            let snapshot = if let Some(mut entry) = self.sessions.get_mut(&session_id) {
                entry.status = SessionStatus::Active;
                entry.channel = channel.to_string();
                entry.task_description = task.to_string();
                entry.action_history.clear();
                entry.operator_handoff_tx = None;
                entry.updated_at = now_rfc3339();
                Some(PersistedBrowserSession::from_session(&entry))
            } else {
                None
            };
            if let Some(snapshot) = snapshot {
                persist_browser_session(self.storage.as_ref(), &snapshot).await;
            }
            return Some(session_id);
        }

        None
    }

    fn spawn_session_loop(
        &self,
        session_id: String,
        llm_client: super::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) {
        let sessions = self.sessions.clone();
        let integration = self.integration.clone();
        let storage = self.storage.clone();

        crate::spawn_logged!(
            "src/core/browser_session.rs:spawn_session_loop",
            async move {
                let Some((sidecar_id, task_desc)) = sessions.get(&session_id).map(|entry| {
                    (
                        entry.sidecar_session_id.clone(),
                        entry.task_description.clone(),
                    )
                }) else {
                    return;
                };

                let result = run_browser_loop(
                    &session_id,
                    BrowserLoopContext {
                        sidecar_id: &sidecar_id,
                        task: &task_desc,
                        sessions: &sessions,
                        integration: &integration,
                        llm: &llm_client,
                        notify: &notify_fn,
                        storage: storage.clone(),
                    },
                )
                .await;

                let mut close_sidecar = false;
                let snapshot = if let Some(mut entry) = sessions.get_mut(&session_id) {
                    entry.status = match result {
                        Ok(summary) => SessionStatus::Ready { summary },
                        Err(error) => {
                            close_sidecar = true;
                            let error_text = error.to_string();
                            if !error_text.starts_with("Reached max iterations (") {
                                let message = format!("Browser automation failed: {}", error_text);
                                notify_fn(BrowserSessionNotification::failed(
                                    &session_id,
                                    message,
                                    None,
                                ));
                            }
                            SessionStatus::Failed(error_text)
                        }
                    };
                    entry.operator_handoff_tx = None;
                    entry.updated_at = now_rfc3339();
                    Some(PersistedBrowserSession::from_session(&entry))
                } else {
                    None
                };
                if let Some(snapshot) = snapshot {
                    persist_browser_session(storage.as_ref(), &snapshot).await;
                }
                if close_sidecar {
                    let _ = integration.close_session(&sidecar_id).await;
                }
                let should_remove = sessions
                    .get(&session_id)
                    .map(|entry| session_status_is_terminal(&entry.status))
                    .unwrap_or(false);
                if should_remove {
                    sessions.remove(&session_id);
                }
            }
        );
    }

    fn spawn_idle_watchdog(
        &self,
        session_id: String,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) {
        let sessions = self.sessions.clone();
        let integration = self.integration.clone();
        let storage = self.storage.clone();

        crate::spawn_logged!(
            "src/core/browser_session.rs:spawn_idle_watchdog",
            async move {
                let mut warned_for_updated_at: Option<String> = None;
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(IDLE_WATCHDOG_POLL_SECS))
                        .await;

                    let Some((status, updated_at, sidecar_session_id)) =
                        sessions.get(&session_id).map(|entry| {
                            (
                                entry.status.clone(),
                                entry.updated_at.clone(),
                                entry.sidecar_session_id.clone(),
                            )
                        })
                    else {
                        return;
                    };

                    if !session_status_has_live_session(&status)
                        || session_status_is_terminal(&status)
                    {
                        return;
                    }

                    let Some(last_updated_at) = parse_rfc3339_utc(&updated_at) else {
                        continue;
                    };
                    let idle_secs = (Utc::now() - last_updated_at).num_seconds();
                    if idle_secs < LIVE_SESSION_IDLE_WARNING_SECS {
                        warned_for_updated_at = None;
                        continue;
                    }

                    if idle_secs >= LIVE_SESSION_IDLE_WARNING_SECS
                        && warned_for_updated_at.as_deref() != Some(updated_at.as_str())
                    {
                        notify_fn(BrowserSessionNotification::notice(
                            &session_id,
                            "This browser session has been idle for almost 15 minutes. It will close automatically in about 1 minute unless you continue the task or reopen the live handoff.",
                        ));
                        warned_for_updated_at = Some(updated_at.clone());
                    }

                    if idle_secs < LIVE_SESSION_IDLE_TIMEOUT_SECS {
                        continue;
                    }

                    if !sidecar_session_id.trim().is_empty() {
                        let _ = integration.close_session(&sidecar_session_id).await;
                    }
                    sessions.remove(&session_id);
                    delete_persisted_browser_session(storage.as_ref(), &session_id).await;
                    notify_fn(BrowserSessionNotification::closed(
                        &session_id,
                        "Browser session closed after 15 minutes of inactivity to keep AgentArk responsive. Ask me to reopen it if you want to continue.",
                    ));
                    return;
                }
            }
        );
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn trimmed_action_history(history: &[String]) -> Vec<String> {
    let start = history.len().saturating_sub(MAX_PERSISTED_ACTION_HISTORY);
    history[start..].to_vec()
}

fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn browser_sidecar_session_missing_error(error: &anyhow::Error) -> bool {
    let lower = error.to_string().to_ascii_lowercase();
    lower.contains("404")
        || (lower.contains("not found") && lower.contains("/session/"))
        || lower.contains("unknown session")
        || lower.contains("session does not exist")
}

fn session_status_has_live_session(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Active
            | SessionStatus::WaitingForOperator { .. }
            | SessionStatus::OperatorClaimed { .. }
            | SessionStatus::Ready { .. }
    )
}

fn session_counts_against_live_limit(status: &SessionStatus) -> bool {
    session_status_has_live_session(status)
}

fn session_should_be_cleaned_up(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::AwaitingResume { .. } | SessionStatus::Interrupted { .. }
    )
}

fn ready_session_is_expired(session: &BrowserSession) -> bool {
    if !matches!(session.status, SessionStatus::Ready { .. }) {
        return false;
    }

    parse_rfc3339_utc(&session.updated_at)
        .map(|updated_at| (Utc::now() - updated_at).num_seconds() >= LIVE_SESSION_IDLE_TIMEOUT_SECS)
        .unwrap_or(false)
}

fn session_status_is_terminal(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Completed { .. } | SessionStatus::Failed(_)
    )
}

fn build_browser_session_view(
    id: String,
    task_description: String,
    status: SessionStatus,
    created_at: String,
    updated_at: String,
    sidecar_state: Option<BrowserSidecarSessionState>,
) -> BrowserSessionView {
    let (status_key, question, summary, reason, can_claim, can_release, can_complete) = match status
    {
        SessionStatus::Active => (
            String::from("active"),
            None,
            None,
            None,
            false,
            false,
            false,
        ),
        SessionStatus::WaitingForOperator { question } => (
            String::from("waiting_for_operator"),
            Some(question),
            None,
            None,
            true,
            false,
            false,
        ),
        SessionStatus::OperatorClaimed { question } => (
            String::from("operator_claimed"),
            Some(question),
            None,
            None,
            false,
            true,
            true,
        ),
        SessionStatus::Ready { summary } => (
            String::from("ready"),
            None,
            Some(summary),
            None,
            true,
            false,
            false,
        ),
        SessionStatus::AwaitingResume { question } => (
            String::from("awaiting_resume"),
            Some(question),
            None,
            None,
            false,
            false,
            false,
        ),
        SessionStatus::Interrupted { reason } => (
            String::from("interrupted"),
            None,
            None,
            Some(reason),
            false,
            false,
            false,
        ),
        SessionStatus::Completed { summary } => (
            String::from("completed"),
            None,
            Some(summary),
            None,
            false,
            false,
            false,
        ),
        SessionStatus::Failed(error) => (
            String::from("failed"),
            None,
            None,
            Some(error),
            false,
            false,
            false,
        ),
    };

    BrowserSessionView {
        id,
        task_description,
        status: status_key,
        question,
        summary,
        reason,
        created_at,
        updated_at,
        page_url: sidecar_state
            .as_ref()
            .and_then(|state| (!state.url.trim().is_empty()).then(|| state.url.clone())),
        page_title: sidecar_state
            .as_ref()
            .and_then(|state| (!state.title.trim().is_empty()).then(|| state.title.clone())),
        live_view_enabled: sidecar_state
            .as_ref()
            .map(|state| state.live_view_enabled)
            .unwrap_or(false),
        live_view_port: sidecar_state
            .as_ref()
            .and_then(|state| state.live_view_port),
        live_view_path: sidecar_state
            .as_ref()
            .and_then(|state| state.live_view_path.clone()),
        can_claim,
        can_release,
        can_complete,
    }
}

async fn persist_browser_session(
    storage: Option<&crate::storage::Storage>,
    snapshot: &PersistedBrowserSession,
) {
    let Some(storage) = storage else {
        return;
    };
    if let Err(error) = storage.upsert_browser_session(snapshot).await {
        tracing::warn!(
            "Failed to persist browser session '{}': {}",
            snapshot.id,
            error
        );
    }
}

async fn delete_persisted_browser_session(
    storage: Option<&crate::storage::Storage>,
    session_id: &str,
) {
    let Some(storage) = storage else {
        return;
    };
    if let Err(error) = storage.delete_browser_session(session_id).await {
        tracing::warn!(
            "Failed to delete stale browser session '{}': {}",
            session_id,
            error
        );
    }
}

async fn sync_session_history(
    sessions: &Arc<DashMap<String, BrowserSession>>,
    storage: Option<&crate::storage::Storage>,
    session_id: &str,
    history: &[String],
) {
    let snapshot = if let Some(mut entry) = sessions.get_mut(session_id) {
        entry.action_history = trimmed_action_history(history);
        entry.updated_at = now_rfc3339();
        Some(PersistedBrowserSession::from_session(&entry))
    } else {
        None
    };
    if let Some(snapshot) = snapshot {
        persist_browser_session(storage, &snapshot).await;
    }
}

async fn set_waiting_for_operator(
    sessions: &Arc<DashMap<String, BrowserSession>>,
    storage: Option<&crate::storage::Storage>,
    session_id: &str,
    question: String,
    tx: oneshot::Sender<OperatorHandoffOutcome>,
) {
    let snapshot = if let Some(mut entry) = sessions.get_mut(session_id) {
        entry.status = SessionStatus::WaitingForOperator { question };
        entry.operator_handoff_tx = Some(tx);
        entry.updated_at = now_rfc3339();
        Some(PersistedBrowserSession::from_session(&entry))
    } else {
        None
    };
    if let Some(snapshot) = snapshot {
        persist_browser_session(storage, &snapshot).await;
    }
}

impl PersistedBrowserSession {
    fn from_session(session: &BrowserSession) -> Self {
        Self {
            id: session.id.clone(),
            status: session.status.kind().to_string(),
            task_description: session.task_description.clone(),
            channel: session.channel.clone(),
            chat_id: session.conversation_id.clone(),
            status_detail: session.status.detail(),
            action_history: trimmed_action_history(&session.action_history),
            created_at: session.created_at.clone(),
            updated_at: session.updated_at.clone(),
        }
    }
}

impl BrowserSession {
    fn restore_from_persisted(persisted: PersistedBrowserSession) -> (Self, bool) {
        let PersistedBrowserSession {
            id,
            status,
            task_description,
            channel,
            chat_id,
            status_detail,
            action_history,
            created_at,
            updated_at,
        } = persisted;
        let (status, changed) = match status.as_str() {
            "active" => (
                SessionStatus::Interrupted {
                    reason: INTERRUPTED_BROWSER_SESSION_REASON.to_string(),
                },
                true,
            ),
            "ready" => (
                SessionStatus::Interrupted {
                    reason: status_detail
                        .unwrap_or_else(|| INTERRUPTED_READY_SESSION_REASON.to_string()),
                },
                true,
            ),
            "waiting_for_user" | "waiting_for_operator" | "operator_claimed" => (
                SessionStatus::AwaitingResume {
                    question: status_detail
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| INTERRUPTED_BROWSER_HANDOFF_REASON.to_string()),
                },
                true,
            ),
            "awaiting_resume" => (
                SessionStatus::AwaitingResume {
                    question: status_detail
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| {
                            "Browser session is waiting for you to restart it.".to_string()
                        }),
                },
                false,
            ),
            "interrupted" => (
                SessionStatus::Interrupted {
                    reason: status_detail
                        .unwrap_or_else(|| INTERRUPTED_BROWSER_SESSION_REASON.to_string()),
                },
                false,
            ),
            "completed" => (
                SessionStatus::Completed {
                    summary: status_detail.unwrap_or_else(|| "Task completed".to_string()),
                },
                false,
            ),
            "failed" => (
                SessionStatus::Failed(
                    status_detail.unwrap_or_else(|| "Browser session failed".to_string()),
                ),
                false,
            ),
            other => (
                SessionStatus::Failed(format!(
                    "Unsupported restored browser session status: {other}"
                )),
                true,
            ),
        };
        (
            BrowserSession {
                id,
                sidecar_session_id: String::new(),
                channel,
                conversation_id: chat_id,
                task_description,
                status,
                action_history: trimmed_action_history(&action_history),
                created_at,
                updated_at: if changed { now_rfc3339() } else { updated_at },
                operator_handoff_tx: None,
            },
            changed,
        )
    }
}

impl SessionStatus {
    fn kind(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::WaitingForOperator { .. } => "waiting_for_operator",
            Self::OperatorClaimed { .. } => "operator_claimed",
            Self::Ready { .. } => "ready",
            Self::AwaitingResume { .. } => "awaiting_resume",
            Self::Interrupted { .. } => "interrupted",
            Self::Completed { .. } => "completed",
            Self::Failed(_) => "failed",
        }
    }

    fn detail(&self) -> Option<String> {
        match self {
            Self::Active => None,
            Self::WaitingForOperator { question } => Some(question.clone()),
            Self::OperatorClaimed { question } => Some(question.clone()),
            Self::Ready { summary } => Some(summary.clone()),
            Self::AwaitingResume { question } => Some(question.clone()),
            Self::Interrupted { reason } => Some(reason.clone()),
            Self::Completed { summary } => Some(summary.clone()),
            Self::Failed(error) => Some(error.clone()),
        }
    }
}

struct BrowserLoopContext<'a> {
    sidecar_id: &'a str,
    task: &'a str,
    sessions: &'a Arc<DashMap<String, BrowserSession>>,
    integration: &'a Arc<BrowserIntegration>,
    llm: &'a super::llm::LlmClient,
    notify: &'a Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    storage: Option<crate::storage::Storage>,
}

async fn run_browser_loop(session_id: &str, ctx: BrowserLoopContext<'_>) -> Result<String> {
    let browser_system_prompt = format!(
        "You are a browser automation agent. Your task: {}\n\n\
         Respond with exactly one JSON action per step:\n\
         - {{\"action\":\"navigate\",\"url\":\"...\"}}\n\
         - {{\"action\":\"click\",\"text\":\"...\"}}\n\
         - {{\"action\":\"click\",\"selector\":\"...\"}}\n\
         - {{\"action\":\"click\",\"x\":N,\"y\":N}}\n\
         - {{\"action\":\"type_text\",\"text\":\"...\",\"selector\":\"...\",\"clear\":false}}\n\
         - {{\"action\":\"scroll\",\"direction\":\"down\"}}\n\
         - {{\"action\":\"press_key\",\"key\":\"Enter\"}}\n\
         - {{\"action\":\"ask_user\",\"question\":\"...\"}} when the site needs a real human to take over the live browser for login, MFA, CAPTCHA, or other sensitive steps. Do not ask the user to paste website passwords or one-time codes into chat when live browser handoff will work.\n\
         - {{\"action\":\"notify\",\"message\":\"...\"}}\n\
         - {{\"action\":\"done\",\"summary\":\"...\",\"message\":\"...\"}}\n\n\
         Guardrails:\n\
         - Treat the task as a multi-step objective. If it includes a gated step such as login plus later explicit website work, use live handoff only for the gated step, then continue the remaining task yourself after the handoff resumes.\n\
         - Base every decision on the current page snapshot shown in the latest step.\n\
         - Treat any operator note from a live handoff as an unverified hint, never as proof.\n\
         - After a live handoff resumes, inspect the fresh page state before deciding what changed.\n\
         - If the task already states what should happen after login, MFA, CAPTCHA, or another gated checkpoint, continue directly instead of asking for confirmation.\n\
         - Use ask_user only when a real human must operate the site or when the task is genuinely underspecified even after considering the task text and current page.\n\
         - Only claim a gated checkpoint succeeded when the current page directly supports that claim.\n\
         - If the requested checkpoint cannot be directly verified from the current page, summarize the visible state conservatively instead of inventing hidden state.\n\
         - Use done only when the requested outcome is completed, or when you are blocked and have already asked the user for the missing human step.\n\
         - Only stop and ask what to do next when the explicit task is complete and no further action is implied by the task itself.",
        ctx.task
    );

    let mut history: Vec<String> = Vec::new();
    let mut pending_resume_context: Option<String> = None;

    for iteration in 0..MAX_ITERATIONS {
        let content = ctx.integration.get_content(ctx.sidecar_id).await?;
        let elements_str = content
            .elements
            .iter()
            .take(30)
            .map(|e| {
                let label = if !e.text.is_empty() {
                    &e.text
                } else if !e.name.is_empty() {
                    &e.name
                } else if !e.id.is_empty() {
                    &e.id
                } else {
                    "?"
                };
                format!(
                    "[{}] <{}> \"{}\" at ({},{})",
                    e.index, e.tag, label, e.x, e.y
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let body_preview = if content.body_text.len() > 2000 {
            format!("{}...", &content.body_text[..2000])
        } else {
            content.body_text.clone()
        };

        let mut messages = vec![format!(
            "Step {}/{}\nURL: {}\nTitle: {}\n\nPage text:\n{}\n\nInteractive elements:\n{}",
            iteration + 1,
            MAX_ITERATIONS,
            content.url,
            content.title,
            body_preview,
            elements_str
        )];
        if let Some(resume_context) = pending_resume_context.take() {
            messages.insert(0, resume_context);
        }
        if !history.is_empty() {
            messages.insert(0, format!("Previous actions:\n{}", history.join("\n")));
        }
        let user_msg = messages.join("\n\n---\n\n");

        let llm_response = ctx
            .llm
            .chat_with_system(&browser_system_prompt, &user_msg)
            .await?;
        let response_text = llm_response.content.trim().to_string();
        let json_str = response_text
            .find('{')
            .and_then(|start| response_text.rfind('}').map(|end| (start, end)))
            .map(|(start, end)| &response_text[start..=end])
            .unwrap_or(&response_text);
        let action: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(value) => value,
            Err(_) => {
                history.push(format!("Step {}: Error parsing action", iteration + 1));
                sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history)
                    .await;
                continue;
            }
        };

        match action
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
        {
            "navigate" => {
                if let Some(url) = action.get("url").and_then(|value| value.as_str()) {
                    let (final_url, title) = ctx.integration.navigate(ctx.sidecar_id, url).await?;
                    history.push(format!(
                        "Step {}: Navigated to {} ({})",
                        iteration + 1,
                        final_url,
                        title
                    ));
                } else {
                    history.push(format!("Step {}: Missing navigate URL", iteration + 1));
                }
            }
            "click" => {
                let selector = action.get("selector").and_then(|v| v.as_str());
                let text = action.get("text").and_then(|v| v.as_str());
                let x = action.get("x").and_then(|v| v.as_i64()).map(|v| v as i32);
                let y = action.get("y").and_then(|v| v.as_i64()).map(|v| v as i32);
                let label = text.or(selector).unwrap_or("element");
                ctx.integration
                    .click(ctx.sidecar_id, selector, text, x, y)
                    .await?;
                history.push(format!("Step {}: Clicked '{}'", iteration + 1, label));
            }
            "type_text" => {
                let text = action.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let selector = action.get("selector").and_then(|v| v.as_str());
                let clear = action
                    .get("clear")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                ctx.integration
                    .type_text(ctx.sidecar_id, text, selector, clear)
                    .await?;
                history.push(format!(
                    "Step {}: Typed {} chars",
                    iteration + 1,
                    text.len()
                ));
            }
            "scroll" => {
                let dir = action
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");
                let amount = action
                    .get("amount")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                ctx.integration.scroll(ctx.sidecar_id, dir, amount).await?;
                history.push(format!("Step {}: Scrolled {}", iteration + 1, dir));
            }
            "press_key" => {
                let key = action
                    .get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Enter");
                ctx.integration.press_key(ctx.sidecar_id, key).await?;
                history.push(format!("Step {}: Pressed {}", iteration + 1, key));
            }
            "notify" => {
                if let Some(message) = action.get("message").and_then(|v| v.as_str()) {
                    if !message.is_empty() {
                        (ctx.notify)(BrowserSessionNotification::progress(
                            session_id,
                            message.to_string(),
                        ));
                    }
                }
                history.push(format!("Step {}: Sent progress update", iteration + 1));
            }
            "ask_user" => {
                let question = action
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("I need your help to continue.");
                let (tx, rx) = oneshot::channel::<OperatorHandoffOutcome>();
                set_waiting_for_operator(
                    ctx.sessions,
                    ctx.storage.as_ref(),
                    session_id,
                    question.to_string(),
                    tx,
                )
                .await;
                (ctx.notify)(BrowserSessionNotification::needs_input(
                    session_id,
                    question.to_string(),
                    None,
                ));
                match tokio::time::timeout(
                    tokio::time::Duration::from_secs(OPERATOR_HANDOFF_TIMEOUT_SECS),
                    rx,
                )
                .await
                {
                    Ok(Ok(outcome)) => {
                        history.push(format!(
                            "Step {}: Operator completed live browser handoff",
                            iteration + 1
                        ));
                        let operator_note = outcome.note.trim();
                        let operator_note = if operator_note.is_empty() {
                            "No operator note was provided.".to_string()
                        } else {
                            format!("Operator note (hint only): {}", operator_note)
                        };
                        pending_resume_context = Some(format!(
                            "This step just resumed after a live operator handoff.\nPage state immediately before the handoff:\n{}\n{}\nUse the current page snapshot below as the source of truth. The operator note is a hint, not proof.",
                            describe_page_snapshot(&content),
                            operator_note
                        ));
                    }
                    Ok(Err(_)) => {
                        return Err(anyhow!("Live browser handoff channel closed unexpectedly"));
                    }
                    Err(_) => {
                        return Err(anyhow!(
                            "Timed out waiting for the live browser handoff to finish"
                        ));
                    }
                }
            }
            "done" => {
                let summary = action
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Task completed");
                let message = action
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or(summary);
                let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
                history.push(format!("Step {}: DONE - {}", iteration + 1, summary));
                sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history)
                    .await;
                (ctx.notify)(BrowserSessionNotification::completed(
                    session_id,
                    message.to_string(),
                    screenshot,
                ));
                return Ok(summary.to_string());
            }
            other => {
                history.push(format!(
                    "Step {}: Unknown action '{}'",
                    iteration + 1,
                    other
                ));
            }
        }

        sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history).await;
    }

    let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
    (ctx.notify)(BrowserSessionNotification::failed(
        session_id,
        format!(
            "Browser session reached the maximum of {} steps.",
            MAX_ITERATIONS
        ),
        screenshot,
    ));
    Err(anyhow!("Reached max iterations ({})", MAX_ITERATIONS))
}

fn describe_page_snapshot(content: &PageContent) -> String {
    let mut lines = vec![
        format!("URL: {}", content.url.trim()),
        format!("Title: {}", content.title.trim()),
    ];
    let body = content.body_text.trim();
    if !body.is_empty() {
        let body_preview = if body.chars().count() > 500 {
            let preview = body.chars().take(500).collect::<String>();
            format!("{}...", preview)
        } else {
            body.to_string()
        };
        lines.push(format!("Body preview: {}", body_preview));
    }
    let element_preview = content
        .elements
        .iter()
        .take(8)
        .map(|element| {
            let label = if !element.text.trim().is_empty() {
                element.text.trim()
            } else if !element.name.trim().is_empty() {
                element.name.trim()
            } else if !element.id.trim().is_empty() {
                element.id.trim()
            } else if !element.href.trim().is_empty() {
                element.href.trim()
            } else {
                element.tag.trim()
            };
            format!("<{}> {}", element.tag.trim(), label)
        })
        .collect::<Vec<_>>();
    if !element_preview.is_empty() {
        lines.push(format!("Elements: {}", element_preview.join("; ")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_browser_session_view_flags_operator_handoff_states() {
        let waiting = build_browser_session_view(
            "session-1".to_string(),
            "demo".to_string(),
            SessionStatus::WaitingForOperator {
                question: "Log in manually".to_string(),
            },
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        assert_eq!(waiting.status, "waiting_for_operator");
        assert!(waiting.can_claim);
        assert!(!waiting.can_complete);

        let claimed = build_browser_session_view(
            "session-1".to_string(),
            "demo".to_string(),
            SessionStatus::OperatorClaimed {
                question: "Log in manually".to_string(),
            },
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        assert_eq!(claimed.status, "operator_claimed");
        assert!(claimed.can_release);
        assert!(claimed.can_complete);

        let ready = build_browser_session_view(
            "session-1".to_string(),
            "demo".to_string(),
            SessionStatus::Ready {
                summary: "Browser is ready for follow-up.".to_string(),
            },
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        assert_eq!(ready.status, "ready");
        assert!(ready.can_claim);
        assert_eq!(
            ready.summary.as_deref(),
            Some("Browser is ready for follow-up.")
        );
    }

    #[test]
    fn restore_preserves_live_handoff_sessions_as_awaiting_resume() {
        let persisted = PersistedBrowserSession {
            id: "session-1".to_string(),
            status: "operator_claimed".to_string(),
            task_description: "demo".to_string(),
            channel: "web".to_string(),
            chat_id: Some("conversation-1".to_string()),
            status_detail: Some("Please finish the live login".to_string()),
            action_history: vec!["Step 1: Navigated".to_string()],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:01:00Z".to_string(),
        };

        let (session, changed) = BrowserSession::restore_from_persisted(persisted);

        assert!(changed);
        assert_eq!(session.conversation_id.as_deref(), Some("conversation-1"));
        match session.status {
            SessionStatus::AwaitingResume { question } => {
                assert!(question.contains("Please finish the live login"));
            }
            other => panic!("unexpected restored status: {:?}", other),
        }
    }

    #[test]
    fn awaiting_resume_does_not_count_against_live_session_limit() {
        assert!(session_counts_against_live_limit(&SessionStatus::Active));
        assert!(session_counts_against_live_limit(
            &SessionStatus::WaitingForOperator {
                question: "Log in".to_string(),
            }
        ));
        assert!(session_counts_against_live_limit(
            &SessionStatus::OperatorClaimed {
                question: "Log in".to_string(),
            }
        ));
        assert!(session_counts_against_live_limit(&SessionStatus::Ready {
            summary: "Browser is ready".to_string(),
        }));
        assert!(!session_counts_against_live_limit(
            &SessionStatus::AwaitingResume {
                question: "Restart the browser task".to_string(),
            }
        ));
    }

    #[test]
    fn interrupted_restart_sessions_are_marked_for_cleanup() {
        assert!(session_should_be_cleaned_up(
            &SessionStatus::AwaitingResume {
                question: "Restart the browser task".to_string(),
            }
        ));
        assert!(session_should_be_cleaned_up(&SessionStatus::Interrupted {
            reason: "App restarted".to_string(),
        }));
        assert!(!session_should_be_cleaned_up(&SessionStatus::Active));
        assert!(!session_should_be_cleaned_up(
            &SessionStatus::WaitingForOperator {
                question: "Log in".to_string(),
            }
        ));
        assert!(!session_should_be_cleaned_up(&SessionStatus::Ready {
            summary: "Browser is ready".to_string(),
        }));
    }

    fn test_manager() -> BrowserSessionManager {
        BrowserSessionManager {
            sessions: Arc::new(DashMap::new()),
            integration: Arc::new(BrowserIntegration::new()),
            storage: None,
        }
    }

    fn test_session(
        id: &str,
        sidecar_session_id: &str,
        conversation_id: Option<&str>,
        status: SessionStatus,
        updated_at: &str,
    ) -> BrowserSession {
        BrowserSession {
            id: id.to_string(),
            sidecar_session_id: sidecar_session_id.to_string(),
            channel: "web".to_string(),
            conversation_id: conversation_id.map(str::to_string),
            task_description: "demo task".to_string(),
            status,
            action_history: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
            operator_handoff_tx: None,
        }
    }

    #[tokio::test]
    async fn latest_managed_live_session_for_conversation_prefers_most_recent_live_session() {
        let manager = test_manager();
        manager.sessions.insert(
            "managed-old".to_string(),
            test_session(
                "managed-old",
                "sidecar-old",
                Some("conversation-1"),
                SessionStatus::WaitingForOperator {
                    question: "Log in".to_string(),
                },
                "2026-01-01T00:05:00Z",
            ),
        );
        manager.sessions.insert(
            "managed-new".to_string(),
            test_session(
                "managed-new",
                "sidecar-new",
                Some("conversation-1"),
                SessionStatus::Active,
                "2026-01-01T00:10:00Z",
            ),
        );

        let session_id = manager
            .latest_managed_live_session_for_conversation(Some("conversation-1"))
            .await
            .expect("expected reusable live browser session");

        assert_eq!(session_id, "managed-new");
    }
}
