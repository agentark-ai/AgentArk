//! Browser session manager for LLM-driven browser automation.
//!
//! Sessions are long-running background tasks that control the Playwright bridge,
//! pause for explicit operator handoff when the browser needs a human, and keep
//! enough durable state to survive restarts.

use anyhow::{anyhow, Result};
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::integrations::browser::{BrowserIntegration, BrowserSidecarSessionState};

const MAX_ITERATIONS: u32 = 30;
const MAX_PERSISTED_ACTION_HISTORY: usize = 80;
const OPERATOR_HANDOFF_TIMEOUT_SECS: u64 = 30 * 60;
const INTERRUPTED_BROWSER_SESSION_REASON: &str =
    "Browser session was interrupted by an app restart before it could finish.";
const INTERRUPTED_BROWSER_HANDOFF_REASON: &str =
    "Browser handoff was interrupted by an app restart before it could finish.";

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
    Completed,
    Failed,
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

    pub fn integration(&self) -> &Arc<BrowserIntegration> {
        &self.integration
    }

    pub async fn start_session(
        &self,
        task: &str,
        channel: &str,
        llm_client: super::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) -> Result<String> {
        let sidecar_id = self.integration.create_session().await?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let created_at = now_rfc3339();

        self.sessions.insert(
            session_id.clone(),
            BrowserSession {
                id: session_id.clone(),
                sidecar_session_id: sidecar_id.clone(),
                channel: channel.to_string(),
                task_description: task.to_string(),
                status: SessionStatus::Active,
                action_history: Vec::new(),
                created_at: created_at.clone(),
                updated_at: created_at,
                operator_handoff_tx: None,
            },
        );
        self.persist_session(&session_id).await;

        let sessions = self.sessions.clone();
        let integration = self.integration.clone();
        let sid = session_id.clone();
        let task_desc = task.to_string();
        let storage = self.storage.clone();

        tokio::spawn(async move {
            let result = run_browser_loop(
                &sid,
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

            let _ = integration.close_session(&sidecar_id).await;
            let snapshot = if let Some(mut entry) = sessions.get_mut(&sid) {
                entry.status = match result {
                    Ok(summary) => SessionStatus::Completed { summary },
                    Err(error) => {
                        let error_text = error.to_string();
                        if !error_text.starts_with("Reached max iterations (") {
                            let message = format!("Browser automation failed: {}", error_text);
                            notify_fn(BrowserSessionNotification::failed(&sid, message, None));
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
        });

        Ok(session_id)
    }

    pub async fn describe_session(&self, session_id: &str) -> Option<BrowserSessionView> {
        let (id, sidecar_session_id, task_description, status, created_at, updated_at) =
            self.sessions.get(session_id).map(|entry| {
                (
                    entry.id.clone(),
                    entry.sidecar_session_id.clone(),
                    entry.task_description.clone(),
                    entry.status.clone(),
                    entry.created_at.clone(),
                    entry.updated_at.clone(),
                )
            })?;

        let sidecar_state =
            if session_status_has_live_session(&status) && !sidecar_session_id.trim().is_empty() {
                self.integration
                    .get_session_state(&sidecar_session_id)
                    .await
                    .ok()
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

    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    SessionStatus::Active
                        | SessionStatus::WaitingForOperator { .. }
                        | SessionStatus::OperatorClaimed { .. }
                        | SessionStatus::AwaitingResume { .. }
                )
            })
            .count()
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
            self.sessions.insert(session_id, session);
            if let Some(snapshot) = snapshot {
                persist_browser_session(Some(storage), &snapshot).await;
            }
        }
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn trimmed_action_history(history: &[String]) -> Vec<String> {
    let start = history.len().saturating_sub(MAX_PERSISTED_ACTION_HISTORY);
    history[start..].to_vec()
}

fn session_status_has_live_session(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Active
            | SessionStatus::WaitingForOperator { .. }
            | SessionStatus::OperatorClaimed { .. }
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
            chat_id: None,
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
            chat_id: _,
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
         - {{\"action\":\"done\",\"summary\":\"...\",\"message\":\"...\"}}\n",
        ctx.task
    );

    let mut history: Vec<String> = Vec::new();

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
                        if !outcome.note.trim().is_empty() {
                            history.push(format!("Operator note: {}", outcome.note.trim()));
                        }
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
        match session.status {
            SessionStatus::AwaitingResume { question } => {
                assert!(question.contains("Please finish the live login"));
            }
            other => panic!("unexpected restored status: {:?}", other),
        }
    }
}
