//! Browser session manager for LLM-driven browser automation.
//!
//! Sessions are long-running background tasks that control the Playwright bridge,
//! pause for explicit operator handoff when the browser needs a human, and keep
//! enough durable state to survive restarts.
#![allow(dead_code)]

use anyhow::{anyhow, Result};
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::integrations::browser::{
    BrowserDownloadArtifact, BrowserIntegration, BrowserSessionCreateOptions,
    BrowserSidecarSessionState, PageContent, PageElement,
};

const MAX_ITERATIONS: u32 = 30;
const MAX_LIVE_BROWSER_SESSIONS: usize = 3;
const CONTENT_SNAPSHOT_ATTEMPTS: usize = 5;
const MAX_PERSISTED_ACTION_HISTORY: usize = 80;
const MAX_NO_VISIBLE_PROGRESS_INTERACTIONS: u32 = 4;
const OPERATOR_HANDOFF_TIMEOUT_SECS: u64 = 30 * 60;
const LIVE_SESSION_IDLE_TIMEOUT_SECS: i64 = 5 * 60;
const LIVE_SESSION_IDLE_WARNING_SECS: i64 = LIVE_SESSION_IDLE_TIMEOUT_SECS - 60;
const IDLE_WATCHDOG_POLL_SECS: u64 = 15;
const TERMINAL_SESSION_EVIDENCE_RETENTION_SECS: u64 = 10 * 60;
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
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
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
    AwaitingChatInput { question: String },
    AwaitingResume { question: String },
    Interrupted { reason: String },
    Completed { summary: String },
    Failed(String),
}

#[derive(Debug)]
struct OperatorHandoffOutcome {
    note: String,
    resume_in_chat: bool,
}

pub struct BrowserSession {
    pub id: String,
    pub sidecar_session_id: String,
    pub channel: String,
    pub conversation_id: Option<String>,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub task_description: String,
    pub status: SessionStatus,
    pub action_history: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    loop_token: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub task_description: String,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub status: String,
    pub question: Option<String>,
    pub summary: Option<String>,
    pub reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<BrowserDownloadArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_page: Option<PageContent>,
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
    page_snapshots: Arc<DashMap<String, PageContent>>,
    integration: Arc<BrowserIntegration>,
    storage: Option<crate::storage::Storage>,
}

pub struct StartedBrowserSession {
    pub session_id: String,
    pub reused_existing: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserSessionProfileContext {
    pub profile_id: String,
    pub profile_name: Option<String>,
    pub browser: Option<String>,
    pub target_kind: Option<String>,
    pub target_endpoint: Option<String>,
    pub target_profile_path: Option<String>,
    pub target_workspace: Option<String>,
    pub managed: Option<bool>,
    pub manual_login: Option<bool>,
}

impl BrowserSessionProfileContext {
    pub fn from_browser_profile(profile: &crate::core::BrowserProfileRecord) -> Self {
        let options = BrowserSessionCreateOptions::from_browser_profile(profile);
        Self {
            profile_id: profile.id.clone(),
            profile_name: Some(profile.name.clone()),
            browser: options.browser,
            target_kind: options.target_kind,
            target_endpoint: options.target_endpoint,
            target_profile_path: options.target_profile_path,
            target_workspace: options.target_workspace,
            managed: options.managed,
            manual_login: options.manual_login,
        }
    }

    fn create_options(&self) -> BrowserSessionCreateOptions {
        BrowserSessionCreateOptions {
            profile_id: Some(self.profile_id.clone()),
            profile_name: self.profile_name.clone(),
            browser: self.browser.clone(),
            target_kind: self.target_kind.clone(),
            target_endpoint: self.target_endpoint.clone(),
            target_profile_path: self.target_profile_path.clone(),
            target_workspace: self.target_workspace.clone(),
            managed: self.managed,
            manual_login: self.manual_login,
        }
    }
}

impl BrowserSessionManager {
    pub async fn new(storage: Option<crate::storage::Storage>) -> Self {
        let manager = Self {
            sessions: Arc::new(DashMap::new()),
            page_snapshots: Arc::new(DashMap::new()),
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
        llm_client: crate::core::model::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) -> Result<StartedBrowserSession> {
        self.start_session_with_profile(task, channel, conversation_id, None, llm_client, notify_fn)
            .await
    }

    pub async fn start_session_with_profile(
        &self,
        task: &str,
        channel: &str,
        conversation_id: Option<&str>,
        profile: Option<BrowserSessionProfileContext>,
        llm_client: crate::core::model::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) -> Result<StartedBrowserSession> {
        self.cleanup_stale_or_unreachable_sessions().await;
        self.close_ready_profile_login_sessions().await;
        let conversation_id = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let requested_profile_id = profile
            .as_ref()
            .map(|profile| profile.profile_id.trim())
            .filter(|value| !value.is_empty());
        if let Some(existing_session_id) = self
            .take_reusable_ready_session(
                conversation_id.as_deref(),
                task,
                channel,
                requested_profile_id,
            )
            .await
        {
            self.spawn_session_loop(existing_session_id.clone(), llm_client, notify_fn);
            return Ok(StartedBrowserSession {
                session_id: existing_session_id,
                reused_existing: true,
            });
        }
        if let Some(existing_session_id) = self
            .latest_managed_live_session_for_conversation(
                conversation_id.as_deref(),
                requested_profile_id,
            )
            .await
        {
            return Ok(StartedBrowserSession {
                session_id: existing_session_id,
                reused_existing: true,
            });
        }
        self.delete_superseded_failed_sessions(conversation_id.as_deref(), requested_profile_id)
            .await;

        if self.active_count() >= MAX_LIVE_BROWSER_SESSIONS {
            anyhow::bail!(
                "Maximum {} concurrent browser sessions",
                MAX_LIVE_BROWSER_SESSIONS
            );
        }

        let create_options = profile
            .as_ref()
            .map(BrowserSessionProfileContext::create_options)
            .unwrap_or_default();
        let profile_id = profile.as_ref().map(|profile| profile.profile_id.clone());
        let profile_name = profile
            .as_ref()
            .and_then(|profile| profile.profile_name.clone());
        let sidecar_id = self
            .integration
            .create_session_with_options(&create_options)
            .await?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let created_at = now_rfc3339();

        self.sessions.insert(
            session_id.clone(),
            BrowserSession {
                id: session_id.clone(),
                sidecar_session_id: sidecar_id.clone(),
                channel: channel.to_string(),
                conversation_id,
                profile_id,
                profile_name,
                task_description: public_browser_task_description(task),
                status: SessionStatus::Active,
                action_history: Vec::new(),
                created_at: created_at.clone(),
                updated_at: created_at,
                loop_token: None,
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

    pub async fn resume_chat_input_session(
        &self,
        session_id: &str,
        response: &str,
        channel: &str,
        llm_client: crate::core::model::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) -> Result<Option<StartedBrowserSession>> {
        let (sidecar_session_id, next_task) = {
            let Some(entry) = self.sessions.get(session_id) else {
                return Ok(None);
            };
            let SessionStatus::AwaitingChatInput { question } = &entry.status else {
                return Ok(None);
            };
            (
                entry.sidecar_session_id.clone(),
                browser_chat_follow_up_task_description(
                    &entry.task_description,
                    question,
                    response,
                ),
            )
        };
        if sidecar_session_id.trim().is_empty() {
            anyhow::bail!("Browser session has no live sidecar to resume");
        }
        self.integration
            .get_session_state(&sidecar_session_id)
            .await
            .map_err(|error| anyhow!("Browser session is no longer reachable: {error}"))?;

        let snapshot = if let Some(mut entry) = self.sessions.get_mut(session_id) {
            if !matches!(&entry.status, SessionStatus::AwaitingChatInput { .. }) {
                return Ok(None);
            }
            entry.status = SessionStatus::Active;
            entry.channel = channel.to_string();
            entry.task_description = next_task;
            entry.action_history.clear();
            entry.loop_token = None;
            entry.operator_handoff_tx = None;
            entry.updated_at = now_rfc3339();
            Some(PersistedBrowserSession::from_session(&entry))
        } else {
            None
        };
        if let Some(snapshot) = snapshot {
            persist_browser_session(self.storage.as_ref(), &snapshot).await;
        }
        self.spawn_session_loop(session_id.to_string(), llm_client, notify_fn);
        Ok(Some(StartedBrowserSession {
            session_id: session_id.to_string(),
            reused_existing: true,
        }))
    }

    pub async fn describe_session(&self, session_id: &str) -> Option<BrowserSessionView> {
        let (
            id,
            sidecar_session_id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
            status,
            created_at,
            updated_at,
        ) = if let Some(entry) = self.sessions.get(session_id) {
            (
                entry.id.clone(),
                entry.sidecar_session_id.clone(),
                entry.conversation_id.clone(),
                entry.task_description.clone(),
                entry.profile_id.clone(),
                entry.profile_name.clone(),
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
                session.conversation_id,
                session.task_description,
                session.profile_id,
                session.profile_name,
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

        let mut view = build_browser_session_view(
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
            status,
            created_at,
            updated_at,
            sidecar_state,
        );
        view.last_page = self
            .page_snapshots
            .get(session_id)
            .map(|snapshot| snapshot.clone());
        Some(view)
    }

    pub async fn list_session_views(&self) -> Vec<BrowserSessionView> {
        self.cleanup_stale_or_unreachable_sessions().await;

        let mut sessions = self
            .sessions
            .iter()
            .map(|entry| {
                build_browser_session_view(
                    entry.id.clone(),
                    entry.conversation_id.clone(),
                    entry.task_description.clone(),
                    entry.profile_id.clone(),
                    entry.profile_name.clone(),
                    entry.status.clone(),
                    entry.created_at.clone(),
                    entry.updated_at.clone(),
                    None,
                )
            })
            .collect::<Vec<_>>();
        let seen = sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<std::collections::HashSet<_>>();
        if let Some(storage) = self.storage.as_ref() {
            match storage.list_browser_sessions().await {
                Ok(persisted_sessions) => {
                    for persisted in persisted_sessions {
                        if seen.contains(&persisted.id) {
                            continue;
                        }
                        let (session, changed) = BrowserSession::restore_from_persisted(persisted);
                        if changed {
                            persist_browser_session(
                                Some(storage),
                                &PersistedBrowserSession::from_session(&session),
                            )
                            .await;
                        }
                        sessions.push(build_browser_session_view(
                            session.id,
                            session.conversation_id,
                            session.task_description,
                            session.profile_id,
                            session.profile_name,
                            session.status,
                            session.created_at,
                            session.updated_at,
                            None,
                        ));
                    }
                }
                Err(error) => {
                    tracing::warn!("Failed to list persisted browser sessions: {}", error)
                }
            }
        }
        sessions = browser_session_views_without_superseded_failures(sessions);
        sessions.retain(browser_session_view_is_live_listing);
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
                SessionStatus::AwaitingChatInput { question } => {
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
                entry.conversation_id.clone(),
                entry.task_description.clone(),
                entry.profile_id.clone(),
                entry.profile_name.clone(),
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

        let (
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
            status,
            created_at,
            updated_at,
        ) = view_parts.ok_or_else(|| anyhow!("Browser session not found"))?;
        Ok(build_browser_session_view(
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
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
                entry.conversation_id.clone(),
                entry.task_description.clone(),
                entry.profile_id.clone(),
                entry.profile_name.clone(),
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

        let (
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
            status,
            created_at,
            updated_at,
        ) = view_parts.ok_or_else(|| anyhow!("Browser session not found"))?;
        Ok(build_browser_session_view(
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
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
        resume_in_chat: bool,
    ) -> Result<BrowserSessionView> {
        let (sidecar_session_id, release_live_claim) = {
            let entry = self
                .sessions
                .get(session_id)
                .ok_or_else(|| anyhow!("Browser session not found"))?;
            let release_live_claim = match &entry.status {
                SessionStatus::OperatorClaimed { .. } => true,
                SessionStatus::WaitingForOperator { .. } => false,
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
            };
            (entry.sidecar_session_id.clone(), release_live_claim)
        };

        let sidecar_state = if sidecar_session_id.trim().is_empty() {
            None
        } else if release_live_claim {
            self.integration
                .release_session(&sidecar_session_id)
                .await
                .ok()
        } else {
            self.integration
                .get_session_state(&sidecar_session_id)
                .await
                .ok()
        };
        let (tx, view_parts, snapshot) = if let Some(mut entry) = self.sessions.get_mut(session_id)
        {
            match &entry.status {
                SessionStatus::OperatorClaimed { .. }
                | SessionStatus::WaitingForOperator { .. } => {}
                _ => {
                    return Err(anyhow!(
                        "Browser session is not waiting for a live handoff completion"
                    ));
                }
            }

            let tx = entry.operator_handoff_tx.take().ok_or_else(|| {
                anyhow!("Browser session is no longer waiting for a live operator handoff")
            })?;
            entry.status = if resume_in_chat {
                entry.loop_token = None;
                SessionStatus::Ready {
                    summary: "Browser handoff returned control. Continue from chat to inspect the current page.".to_string(),
                }
            } else {
                SessionStatus::Active
            };
            entry.updated_at = now_rfc3339();
            let parts = (
                entry.id.clone(),
                entry.conversation_id.clone(),
                entry.task_description.clone(),
                entry.profile_id.clone(),
                entry.profile_name.clone(),
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
                resume_in_chat,
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
        let (
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
            status,
            created_at,
            updated_at,
        ) = view_parts;
        Ok(build_browser_session_view(
            id,
            conversation_id,
            task_description,
            profile_id,
            profile_name,
            status,
            created_at,
            updated_at,
            sidecar_state,
        ))
    }

    pub async fn latest_live_session_view_for_conversation(
        &self,
        conversation_id: Option<&str>,
    ) -> Option<BrowserSessionView> {
        let session_id = self
            .latest_managed_live_session_for_conversation(conversation_id, None)
            .await?;
        self.describe_session(&session_id).await
    }

    pub async fn read_session_content(
        &self,
        session_id: &str,
    ) -> Result<(BrowserSessionView, PageContent)> {
        let sidecar_session_id = self
            .touch_live_session(session_id)
            .await
            .ok_or_else(|| anyhow!("Browser session is not live or could not be found"))?;
        let content =
            browser_content_snapshot(self.integration.as_ref(), &sidecar_session_id).await?;
        let view = self
            .describe_session(session_id)
            .await
            .ok_or_else(|| anyhow!("Browser session not found"))?;
        Ok((view, content))
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
                    entry.conversation_id.clone(),
                    entry.task_description.clone(),
                    entry.profile_id.clone(),
                    entry.profile_name.clone(),
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
                        session.conversation_id.clone(),
                        session.task_description.clone(),
                        session.profile_id.clone(),
                        session.profile_name.clone(),
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
        self.page_snapshots.remove(session_id);

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

    pub async fn start_profile_login_session(
        &self,
        profile: BrowserSessionProfileContext,
    ) -> Result<BrowserSessionView> {
        let profile_id = profile.profile_id.trim().to_string();
        if profile_id.is_empty() {
            anyhow::bail!("Browser profile id required");
        }

        self.cleanup_stale_or_unreachable_sessions().await;

        let mut existing = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (entry.profile_id.as_deref() == Some(profile_id.as_str())
                    && session_status_has_live_session(&entry.status))
                .then(|| (entry.id.clone(), entry.updated_at.clone()))
            })
            .collect::<Vec<_>>();
        existing.sort_by(|left, right| right.1.cmp(&left.1));
        for (session_id, _) in existing {
            if let Some(view) = self.describe_session(&session_id).await {
                return Ok(view);
            }
        }

        if self.active_count() >= MAX_LIVE_BROWSER_SESSIONS {
            anyhow::bail!(
                "Maximum {} concurrent browser sessions",
                MAX_LIVE_BROWSER_SESSIONS
            );
        }

        let profile_name = profile
            .profile_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut create_options = profile.create_options();
        create_options.manual_login = Some(true);
        let sidecar_id = self
            .integration
            .create_session_with_options(&create_options)
            .await?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let created_at = now_rfc3339();
        let display_name = profile_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(profile_id.as_str())
            .to_string();

        self.sessions.insert(
            session_id.clone(),
            BrowserSession {
                id: session_id.clone(),
                sidecar_session_id: sidecar_id.clone(),
                channel: "web".to_string(),
                conversation_id: None,
                profile_id: Some(profile_id.clone()),
                profile_name,
                task_description: format!("Manual login setup for saved browser profile \"{display_name}\""),
                status: SessionStatus::Ready {
                    summary: "Live browser launched for manual login setup. Sign in, then close the session to save this profile.".to_string(),
                },
                action_history: Vec::new(),
                created_at: created_at.clone(),
                updated_at: created_at,
                loop_token: None,
                operator_handoff_tx: None,
            },
        );
        self.persist_session(&session_id).await;
        self.spawn_idle_watchdog(session_id.clone(), Arc::new(|_| {}));
        let sidecar_state = self.integration.get_session_state(&sidecar_id).await.ok();

        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("Browser session not found"))?;
        Ok(build_browser_session_view(
            session.id.clone(),
            session.conversation_id.clone(),
            session.task_description.clone(),
            session.profile_id.clone(),
            session.profile_name.clone(),
            session.status.clone(),
            session.created_at.clone(),
            session.updated_at.clone(),
            sidecar_state,
        ))
    }

    pub async fn stop_profile_sessions(&self, profile_id: &str) -> Result<Vec<BrowserSessionView>> {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (entry.profile_id.as_deref() == Some(profile_id)
                    && session_status_has_live_session(&entry.status))
                .then(|| entry.id.clone())
            })
            .collect::<Vec<_>>();
        let mut stopped = Vec::new();
        for session_id in session_ids {
            match self.stop_session(&session_id).await {
                Ok(view) => stopped.push(view),
                Err(error) => {
                    tracing::warn!(
                        "Failed to stop browser session '{}' for profile '{}': {}",
                        session_id,
                        profile_id,
                        error
                    );
                }
            }
        }
        Ok(stopped)
    }

    pub async fn delete_profile_storage(&self, profile_id: &str) -> Result<()> {
        self.integration.delete_profile_storage(profile_id).await
    }

    pub fn active_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|entry| session_counts_against_live_limit(&entry.status))
            .count()
    }

    pub async fn cleanup_stale_or_unreachable_sessions(&self) {
        self.cleanup_stale_sessions().await;
        self.prune_unreachable_live_sessions().await;
        self.cleanup_persisted_session_records().await;
    }

    async fn cleanup_stale_sessions(&self) {
        let now = Utc::now();
        let stale_sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|entry| {
                ready_session_is_expired(entry.value())
                    || browser_session_record_should_be_cleaned_up(
                        &entry.status,
                        &entry.updated_at,
                        &now,
                    )
            })
            .map(|entry| {
                (
                    entry.id.clone(),
                    entry.status.kind().to_string(),
                    entry.sidecar_session_id.clone(),
                )
            })
            .collect();

        for (session_id, status, sidecar_session_id) in stale_sessions {
            self.sessions.remove(&session_id);
            tracing::info!(
                "Cleaning up stale browser session '{}' with status '{}'",
                session_id,
                status
            );
            self.close_sidecar_best_effort(&sidecar_session_id).await;
            delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
        }
    }

    async fn cleanup_persisted_session_records(&self) {
        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        let persisted_sessions = match storage.list_browser_sessions().await {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!(
                    "Failed to list persisted browser sessions for cleanup: {}",
                    error
                );
                return;
            }
        };
        let now = Utc::now();
        for persisted in persisted_sessions {
            if self.sessions.contains_key(&persisted.id) {
                continue;
            }
            let (session, changed) = BrowserSession::restore_from_persisted(persisted);
            if browser_session_record_should_be_cleaned_up(
                &session.status,
                &session.updated_at,
                &now,
            ) {
                self.page_snapshots.remove(&session.id);
                delete_persisted_browser_session(Some(storage), &session.id).await;
            } else if changed {
                persist_browser_session(
                    Some(storage),
                    &PersistedBrowserSession::from_session(&session),
                )
                .await;
            }
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
            self.page_snapshots.remove(&session_id);
            delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
        }
    }

    async fn close_ready_profile_login_sessions(&self) {
        let sessions = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (entry.conversation_id.is_none()
                    && entry.profile_id.is_some()
                    && matches!(entry.status, SessionStatus::Ready { .. }))
                .then(|| (entry.id.clone(), entry.sidecar_session_id.clone()))
            })
            .collect::<Vec<_>>();

        for (session_id, sidecar_session_id) in sessions {
            tracing::info!(
                "Closing stale manual browser profile login session '{}' before starting browser automation",
                session_id
            );
            self.sessions.remove(&session_id);
            self.page_snapshots.remove(&session_id);
            self.close_sidecar_best_effort(&sidecar_session_id).await;
            delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
        }
    }

    async fn delete_superseded_failed_sessions(
        &self,
        conversation_id: Option<&str>,
        requested_profile_id: Option<&str>,
    ) {
        let Some(conversation_id) = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };

        let live_failed_sessions = self
            .sessions
            .iter()
            .filter_map(|entry| {
                failed_browser_session_is_superseded_candidate(
                    entry.conversation_id.as_deref(),
                    entry.profile_id.as_deref(),
                    &entry.status,
                    Some(conversation_id),
                    requested_profile_id,
                )
                .then(|| (entry.id.clone(), entry.sidecar_session_id.clone()))
            })
            .collect::<Vec<_>>();
        for (session_id, sidecar_session_id) in live_failed_sessions {
            self.sessions.remove(&session_id);
            self.page_snapshots.remove(&session_id);
            self.close_sidecar_best_effort(&sidecar_session_id).await;
            delete_persisted_browser_session(self.storage.as_ref(), &session_id).await;
        }

        let Some(storage) = self.storage.as_ref() else {
            return;
        };
        let persisted_sessions = match storage.list_browser_sessions().await {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!(
                    "Failed to inspect persisted browser sessions before superseding failures: {}",
                    error
                );
                return;
            }
        };
        for persisted in persisted_sessions {
            if !persisted.status.trim().eq_ignore_ascii_case("failed") {
                continue;
            }
            let failed_status = SessionStatus::Failed(
                persisted
                    .status_detail
                    .clone()
                    .unwrap_or_else(|| "Browser session failed".to_string()),
            );
            if failed_browser_session_is_superseded_candidate(
                persisted.chat_id.as_deref(),
                persisted.profile_id.as_deref(),
                &failed_status,
                Some(conversation_id),
                requested_profile_id,
            ) {
                delete_persisted_browser_session(Some(storage), &persisted.id).await;
            }
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
        requested_profile_id: Option<&str>,
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
                    && browser_session_profile_matches(
                        entry.profile_id.as_deref(),
                        requested_profile_id,
                    )
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
        let now = Utc::now();
        for persisted in restored {
            let (session, changed) = BrowserSession::restore_from_persisted(persisted);
            let session_id = session.id.clone();
            let snapshot = changed.then(|| PersistedBrowserSession::from_session(&session));
            if browser_session_record_should_be_cleaned_up(
                &session.status,
                &session.updated_at,
                &now,
            ) {
                delete_persisted_browser_session(Some(storage), &session_id).await;
                continue;
            }
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
        requested_profile_id: Option<&str>,
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
                    && browser_session_profile_matches(
                        entry.profile_id.as_deref(),
                        requested_profile_id,
                    )
                    && matches!(
                        entry.status,
                        SessionStatus::Ready { .. } | SessionStatus::AwaitingChatInput { .. }
                    ))
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
                let next_task = match &entry.status {
                    SessionStatus::AwaitingChatInput { question } => {
                        browser_chat_follow_up_task_description(
                            &entry.task_description,
                            question,
                            task,
                        )
                    }
                    _ => public_browser_task_description(task),
                };
                entry.status = SessionStatus::Active;
                entry.channel = channel.to_string();
                entry.task_description = next_task;
                entry.action_history.clear();
                entry.loop_token = None;
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
        llm_client: crate::core::model::llm::LlmClient,
        notify_fn: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    ) {
        let sessions = self.sessions.clone();
        let page_snapshots = self.page_snapshots.clone();
        let integration = self.integration.clone();
        let storage = self.storage.clone();

        crate::spawn_logged!(
            "src/core/browser_session.rs:spawn_session_loop",
            async move {
                let loop_token = uuid::Uuid::new_v4().to_string();
                let Some((sidecar_id, task_desc)) =
                    sessions.get_mut(&session_id).map(|mut entry| {
                        entry.loop_token = Some(loop_token.clone());
                        (
                            entry.sidecar_session_id.clone(),
                            entry.task_description.clone(),
                        )
                    })
                else {
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

                if !sidecar_id.trim().is_empty() {
                    if let Ok(content) =
                        browser_content_snapshot(integration.as_ref(), &sidecar_id).await
                    {
                        page_snapshots
                            .insert(session_id.clone(), compact_browser_page_content(&content));
                    }
                }

                let mut close_sidecar = false;
                let snapshot = if let Some(mut entry) = sessions.get_mut(&session_id) {
                    if entry.loop_token.as_deref() != Some(loop_token.as_str()) {
                        None
                    } else {
                        entry.status = match result {
                            Ok(outcome) => {
                                let (status, should_close_sidecar) =
                                    browser_loop_success_status(outcome);
                                close_sidecar = should_close_sidecar;
                                status
                            }
                            Err(error) => {
                                close_sidecar = true;
                                let error_text = error.to_string();
                                if !error_text.starts_with("Reached max iterations (") {
                                    let message =
                                        format!("Browser automation failed: {}", error_text);
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
                        entry.loop_token = None;
                        entry.updated_at = now_rfc3339();
                        Some(PersistedBrowserSession::from_session(&entry))
                    }
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
                    let page_snapshots = page_snapshots.clone();
                    let session_id_for_cleanup = session_id.clone();
                    crate::spawn_logged!(
                        "src/core/browser_session.rs:terminal_evidence_cleanup",
                        async move {
                            tokio::time::sleep(tokio::time::Duration::from_secs(
                                TERMINAL_SESSION_EVIDENCE_RETENTION_SECS,
                            ))
                            .await;
                            page_snapshots.remove(&session_id_for_cleanup);
                        }
                    );
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
        let page_snapshots = self.page_snapshots.clone();
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
                            "This browser session has been idle for almost 5 minutes. It will close automatically in about 1 minute unless you continue the task or reopen the live handoff.",
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
                    page_snapshots.remove(&session_id);
                    delete_persisted_browser_session(storage.as_ref(), &session_id).await;
                    notify_fn(BrowserSessionNotification::closed(
                        &session_id,
                        "Browser session closed after 5 minutes of inactivity to keep AgentArk responsive. Ask me to reopen it if you want to continue.",
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

fn public_browser_task_description(raw: &str) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let lower = trimmed.to_ascii_lowercase();
    let label = "planner browser task";
    if let Some(label_start) = lower.find(label) {
        let after_label = &trimmed[label_start + label.len()..];
        let after_colon = after_label
            .strip_prefix(':')
            .or_else(|| after_label.trim_start().strip_prefix(':'))
            .unwrap_or(after_label)
            .trim();
        if !after_colon.is_empty() {
            return after_colon.to_string();
        }
    }

    trimmed.to_string()
}

fn browser_chat_follow_up_task_description(
    previous_task: &str,
    question: &str,
    response: &str,
) -> String {
    let previous_task = public_browser_task_description(previous_task);
    let question = question.split_whitespace().collect::<Vec<_>>().join(" ");
    let response = public_browser_task_description(response);
    let response = if response.trim().is_empty() {
        "Continue from the pending browser question.".to_string()
    } else {
        response
    };

    let mut parts = Vec::new();
    if !previous_task.trim().is_empty() {
        parts.push(format!("Previous browser task: {}", previous_task.trim()));
    }
    if !question.trim().is_empty() {
        parts.push(format!("Pending browser question: {}", question.trim()));
    }
    parts.push(format!("User response: {}", response.trim()));
    parts.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserNavigationDecision {
    Allow,
    Reject { reason: &'static str },
}

#[derive(Debug, Default)]
struct BrowserNavigationProgress {
    reached_urls: Vec<String>,
}

impl BrowserNavigationProgress {
    fn observe_page_url(&mut self, url: &str) {
        let Some(normalized) = normalize_browser_navigation_url(url) else {
            return;
        };
        if self.reached_urls.last() == Some(&normalized) {
            return;
        }
        self.reached_urls.push(normalized);
    }

    fn decide_navigation(&self, target_url: &str, current_url: &str) -> BrowserNavigationDecision {
        let Some(target) = normalize_browser_navigation_url(target_url) else {
            return BrowserNavigationDecision::Reject {
                reason: "navigation target is empty",
            };
        };
        if normalize_browser_navigation_url(current_url).as_deref() == Some(target.as_str()) {
            return BrowserNavigationDecision::Reject {
                reason: "navigation target is already the current page",
            };
        }
        if self
            .reached_urls
            .iter()
            .rev()
            .skip(1)
            .any(|reached| reached == &target)
        {
            return BrowserNavigationDecision::Reject {
                reason: "navigation target is an earlier page already left behind",
            };
        }
        if self
            .reached_urls
            .iter()
            .filter(|reached| *reached == &target)
            .count()
            >= 2
        {
            return BrowserNavigationDecision::Reject {
                reason: "navigation target was already reached repeatedly",
            };
        }
        BrowserNavigationDecision::Allow
    }
}

fn normalize_browser_navigation_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(mut parsed) = reqwest::Url::parse(trimmed) {
        parsed.set_fragment(None);
        let mut normalized = parsed.to_string();
        if parsed.query().is_none() && parsed.path() == "/" {
            normalized = normalized.trim_end_matches('/').to_string();
        }
        return Some(normalized);
    }
    Some(trimmed.trim_end_matches('/').to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserTaskSiteConstraint {
    source_host: String,
    allowed_host_suffix: String,
}

impl BrowserTaskSiteConstraint {
    fn allows_navigation(&self, target_url: &str, current_url: &str) -> bool {
        browser_navigation_target_host(target_url, current_url)
            .map(|host| self.allows_host(&host))
            .unwrap_or(false)
    }

    fn allows_host(&self, host: &str) -> bool {
        let normalized = normalize_browser_host(host);
        normalized == self.allowed_host_suffix
            || normalized.ends_with(&format!(".{}", self.allowed_host_suffix))
    }

    fn navigation_rejection_reason(&self, target_url: &str, current_url: &str) -> String {
        match browser_navigation_target_host(target_url, current_url) {
            Some(host) => format!(
                "the task explicitly targets {}, but the requested navigation goes to {}",
                self.allowed_host_suffix,
                normalize_browser_host(&host)
            ),
            None => format!(
                "the task explicitly targets {}, but the requested navigation target is not a usable same-site URL",
                self.allowed_host_suffix
            ),
        }
    }
}

fn browser_task_site_constraint(task: &str) -> Option<BrowserTaskSiteConstraint> {
    let mut source_hosts = Vec::new();
    let mut allowed_suffixes = Vec::new();

    for url in extract_browser_task_urls(task) {
        let Some(host) = url.host_str().map(normalize_browser_host) else {
            continue;
        };
        let suffix = browser_site_suffix_for_host(&host);
        if !source_hosts.contains(&host) {
            source_hosts.push(host);
        }
        if !allowed_suffixes.contains(&suffix) {
            allowed_suffixes.push(suffix);
        }
    }

    if allowed_suffixes.len() == 1 {
        Some(BrowserTaskSiteConstraint {
            source_host: source_hosts
                .first()
                .cloned()
                .unwrap_or_else(|| allowed_suffixes[0].clone()),
            allowed_host_suffix: allowed_suffixes.remove(0),
        })
    } else {
        None
    }
}

fn truncate_browser_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let preview = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn compact_browser_page_content(content: &PageContent) -> PageContent {
    PageContent {
        title: content.title.clone(),
        url: content.url.clone(),
        body_text: truncate_browser_text(&content.body_text, 12_000),
        elements: content.elements.iter().take(80).cloned().collect(),
        diagnostics: content.diagnostics.iter().take(30).cloned().collect(),
        downloads: content.downloads.iter().take(20).cloned().collect(),
        download_dir: content.download_dir.clone(),
    }
}

fn format_browser_downloads(content: &PageContent) -> String {
    content
        .downloads
        .iter()
        .take(20)
        .map(|download| {
            let status = download.status.trim();
            let status = if status.is_empty() {
                "recorded"
            } else {
                status
            };
            let filename = download.filename.trim();
            let path = download.path.trim();
            let label = if filename.is_empty() {
                download.id.trim()
            } else {
                filename
            };
            if path.is_empty() {
                format!("- {} ({}, {} bytes)", label, status, download.bytes)
            } else {
                format!(
                    "- {} ({}, {} bytes) path={}",
                    label, status, download.bytes, path
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn validate_browser_ask_user_resolution(
    ctx: &BrowserLoopContext<'_>,
    question: &str,
    content: &PageContent,
    history: &[String],
) -> BrowserAskUserResolution {
    let element_preview = content
        .elements
        .iter()
        .take(20)
        .map(format_browser_loop_element)
        .collect::<Vec<_>>()
        .join("\n");
    let history_preview = history
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let body_preview = truncate_browser_text(&content.body_text, 4_000);
    let downloads = format_browser_downloads(content);
    let system = "\
You are validating the next outcome for a browser automation run.
Return exactly one JSON object with:
- action: \"complete\", \"ask_user\", or \"continue\"
- requires_human: boolean; required and true only when action is \"ask_user\"
- summary: concise user-facing answer when action is complete; this must contain the actual visible facts requested, not a status line
- message: fuller user-facing answer when action is complete
- guidance: next browser-loop guidance when action is continue

Judge the underlying requested outcome and the visible page state, not surface wording.
Use action \"complete\" when the requested outcome is information-delivery and the current page snapshot already contains enough visible facts to answer. In that case, include the page-derived answer directly.
Use action \"ask_user\" only when a human decision or site operation is genuinely required; in that case set requires_human=true.
Use action \"continue\" when more browser work is needed before either answering or asking the user.
If the page is incomplete, still loading, stale, or otherwise not enough to answer but no human action is required, use action \"continue\".
Do not ask whether the visible snapshot is sufficient when the original task was to report visible facts.";
    let user = format!(
        "Original task:\n{}\n\nCurrent page:\nURL: {}\nTitle: {}\n\nPage text:\n{}\n\nInteractive elements:\n{}\n\nDownloaded files:\n{}\n\nRecent browser history:\n{}\n\nProposed ask_user question:\n{}",
        ctx.task,
        content.url,
        content.title,
        body_preview,
        element_preview,
        downloads,
        history_preview,
        question
    );
    match ctx.llm.chat_classifier_bounded(system, &user, 900).await {
        Ok(response) => {
            let response_text = response.content.trim();
            let json_text = extract_first_json_object(response_text)
                .map(|(json, _)| json)
                .unwrap_or(response_text);
            match serde_json::from_str::<serde_json::Value>(json_text) {
                Ok(value) => browser_ask_user_resolution_from_value(&value, question),
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        response = %truncate_browser_text(response_text, 500),
                        "Browser ask_user validation returned invalid JSON"
                    );
                    browser_ask_user_resolution_when_verdict_unavailable(question)
                }
            }
        }
        Err(error) => {
            tracing::debug!(error = %error, "Browser ask_user validation failed");
            browser_ask_user_resolution_when_verdict_unavailable(question)
        }
    }
}

async fn validate_browser_done_answer(
    _ctx: &BrowserLoopContext<'_>,
    action: &serde_json::Value,
    content: &PageContent,
    page_changed_since_last_interaction: bool,
    _history: &[String],
    summary: &str,
    message: &str,
) -> BrowserDoneAnswerValidation {
    browser_done_answer_local_validation(
        action,
        content,
        page_changed_since_last_interaction,
        summary,
        message,
    )
}

async fn validate_browser_snapshot_completion(
    ctx: &BrowserLoopContext<'_>,
    content: &PageContent,
    history: &[String],
    no_progress_reason: &str,
) -> BrowserDoneAnswerValidation {
    let element_preview = content
        .elements
        .iter()
        .take(20)
        .map(format_browser_loop_element)
        .collect::<Vec<_>>()
        .join("\n");
    let history_preview = history
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let body_preview = truncate_browser_text(&content.body_text, 4_000);
    let downloads = format_browser_downloads(content);
    let fallback_guidance =
        "The current visible snapshot does not yet contain a complete evidenced answer."
            .to_string();
    let system = "\
You are validating whether a stalled browser automation run can answer from the current visible page snapshot before asking the user for help.
Return exactly one JSON object with:
- action: \"complete\" or \"continue\"
- summary: concise user-facing answer when action is complete
- message: fuller user-facing answer when action is complete; include the actual visible facts requested
- guidance: next browser-loop guidance when action is continue

Judge the user's underlying browser objective and the visible page evidence, not surface wording.
Use action \"complete\" only when the current visible page snapshot or downloaded file records contain enough evidence to satisfy the requested outcome.
For information-delivery outcomes, the completed answer must include the actual requested visible facts, not merely say the page loaded or the information is visible.
Use action \"continue\" when the snapshot is not enough to answer, when a human site operation is needed, or when completing would require hidden state not visible in the snapshot.";
    let user = format!(
        "Requested browser outcome:\n{}\n\nNo-progress condition:\n{}\n\nCurrent page:\nURL: {}\nTitle: {}\n\nVisible page text:\n{}\n\nVisible interactive elements:\n{}\n\nDownloaded files:\n{}\n\nRecent browser history:\n{}",
        ctx.task,
        no_progress_reason,
        content.url,
        content.title,
        body_preview,
        element_preview,
        downloads,
        history_preview
    );
    match ctx.llm.chat_classifier_bounded(system, &user, 900).await {
        Ok(response) => {
            let response_text = response.content.trim();
            let json_text = extract_first_json_object(response_text)
                .map(|(json, _)| json)
                .unwrap_or(response_text);
            match serde_json::from_str::<serde_json::Value>(json_text) {
                Ok(value) => browser_snapshot_completion_from_value(&value, &fallback_guidance),
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        response = %truncate_browser_text(response_text, 500),
                        "Browser snapshot-completion validator returned invalid JSON"
                    );
                    BrowserDoneAnswerValidation::Continue {
                        guidance: fallback_guidance,
                    }
                }
            }
        }
        Err(error) => {
            tracing::debug!(error = %error, "Browser snapshot-completion validation failed");
            BrowserDoneAnswerValidation::Continue {
                guidance: fallback_guidance,
            }
        }
    }
}

async fn classify_browser_done_finalization(
    ctx: &BrowserLoopContext<'_>,
    content: &PageContent,
    history: &[String],
    summary: &str,
    message: &str,
) -> BrowserDoneFinalization {
    let element_preview = content
        .elements
        .iter()
        .take(20)
        .map(format_browser_loop_element)
        .collect::<Vec<_>>()
        .join("\n");
    let history_preview = history
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let body_preview = truncate_browser_text(&content.body_text, 4_000);
    let downloads = format_browser_downloads(content);
    let system = "\
You are deciding whether a completed browser task should close its live browser session.
Return exactly one JSON object with:
- keep_open: boolean
- reason: concise explanation

Set keep_open to false for ordinary completed browser work where the requested outcome has already been delivered in chat.
Set keep_open to true only when the user's underlying requested outcome is itself an ongoing browser state that should remain available for the user to inspect or control after the assistant stops.
Infer intent from meaning and current page state. Do not rely on exact wording, casing, order, punctuation, or anticipated phrases.
If unsure, choose keep_open=false so browser processes and profile locks are cleaned up.";
    let user = format!(
        "Requested browser outcome:\n{}\n\nCurrent page:\nURL: {}\nTitle: {}\n\nVisible page text:\n{}\n\nVisible interactive elements:\n{}\n\nDownloaded files:\n{}\n\nRecent browser history:\n{}\n\nProposed completion summary:\n{}\n\nProposed completion message:\n{}",
        ctx.task,
        content.url,
        content.title,
        body_preview,
        element_preview,
        downloads,
        history_preview,
        summary,
        message
    );
    match ctx.llm.chat_classifier_bounded(system, &user, 500).await {
        Ok(response) => {
            let response_text = response.content.trim();
            let json_text = extract_first_json_object(response_text)
                .map(|(json, _)| json)
                .unwrap_or(response_text);
            match serde_json::from_str::<serde_json::Value>(json_text) {
                Ok(value) => browser_done_finalization_from_value(&value),
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        response = %truncate_browser_text(response_text, 500),
                        "Browser done finalization classifier returned invalid JSON"
                    );
                    BrowserDoneFinalization::Close
                }
            }
        }
        Err(error) => {
            tracing::debug!(error = %error, "Browser done finalization classifier failed");
            BrowserDoneFinalization::Close
        }
    }
}

fn extract_browser_task_urls(task: &str) -> Vec<reqwest::Url> {
    task.split_whitespace()
        .filter_map(|token| {
            let candidate = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\''
                        | '`'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | ','
                        | '.'
                        | ';'
                        | '!'
                        | '?'
                )
            });
            if candidate.starts_with("http://") || candidate.starts_with("https://") {
                reqwest::Url::parse(candidate).ok()
            } else {
                None
            }
        })
        .collect()
}

fn browser_navigation_target_host(target_url: &str, current_url: &str) -> Option<String> {
    let trimmed = target_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(parsed) = reqwest::Url::parse(trimmed) {
        return parsed.host_str().map(normalize_browser_host);
    }

    reqwest::Url::parse(current_url)
        .ok()
        .and_then(|base| base.join(trimmed).ok())
        .and_then(|parsed| parsed.host_str().map(normalize_browser_host))
}

fn normalize_browser_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn browser_site_suffix_for_host(host: &str) -> String {
    let normalized = normalize_browser_host(host);
    normalized
        .strip_prefix("www.")
        .filter(|stripped| !stripped.is_empty())
        .unwrap_or(&normalized)
        .to_string()
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
            | SessionStatus::AwaitingChatInput { .. }
    )
}

fn browser_session_profile_matches(
    session_profile_id: Option<&str>,
    requested_profile_id: Option<&str>,
) -> bool {
    match requested_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(requested) => session_profile_id
            .map(str::trim)
            .is_some_and(|profile_id| profile_id == requested),
        None => true,
    }
}

fn browser_session_profile_exactly_matches(
    session_profile_id: Option<&str>,
    requested_profile_id: Option<&str>,
) -> bool {
    let normalized_session = session_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let normalized_requested = requested_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    normalized_session == normalized_requested
}

fn failed_browser_session_is_superseded_candidate(
    session_conversation_id: Option<&str>,
    session_profile_id: Option<&str>,
    status: &SessionStatus,
    conversation_id: Option<&str>,
    requested_profile_id: Option<&str>,
) -> bool {
    let Some(conversation_id) = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    session_conversation_id
        .map(str::trim)
        .is_some_and(|session_conversation_id| session_conversation_id == conversation_id)
        && browser_session_profile_exactly_matches(session_profile_id, requested_profile_id)
        && matches!(status, SessionStatus::Failed(_))
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

fn browser_session_record_should_be_cleaned_up(
    status: &SessionStatus,
    updated_at: &str,
    now: &chrono::DateTime<Utc>,
) -> bool {
    if session_should_be_cleaned_up(status) {
        return true;
    }
    if !session_status_is_terminal(status) {
        return false;
    }
    parse_rfc3339_utc(updated_at)
        .map(|updated_at| {
            (*now - updated_at).num_seconds() >= TERMINAL_SESSION_EVIDENCE_RETENTION_SECS as i64
        })
        .unwrap_or(true)
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
    conversation_id: Option<String>,
    task_description: String,
    profile_id: Option<String>,
    profile_name: Option<String>,
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
        SessionStatus::AwaitingChatInput { question } => (
            String::from("awaiting_chat_input"),
            Some(question),
            None,
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
        conversation_id,
        task_description: public_browser_task_description(&task_description),
        profile_id: profile_id.or_else(|| {
            sidecar_state
                .as_ref()
                .and_then(|state| state.profile_id.clone())
        }),
        profile_name: profile_name.or_else(|| {
            sidecar_state
                .as_ref()
                .and_then(|state| state.profile_name.clone())
        }),
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
        download_dir: sidecar_state
            .as_ref()
            .and_then(|state| state.download_dir.clone())
            .filter(|value| !value.trim().is_empty()),
        downloads: sidecar_state
            .as_ref()
            .map(|state| state.downloads.clone())
            .unwrap_or_default(),
        last_page: None,
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

fn browser_session_views_without_superseded_failures(
    sessions: Vec<BrowserSessionView>,
) -> Vec<BrowserSessionView> {
    sessions
        .iter()
        .filter(|session| !browser_session_view_is_superseded_failure(session, sessions.as_slice()))
        .cloned()
        .collect()
}

fn browser_session_view_is_live_listing(session: &BrowserSessionView) -> bool {
    matches!(
        session.status.as_str(),
        "active" | "waiting_for_operator" | "operator_claimed" | "ready" | "awaiting_chat_input"
    )
}

fn browser_session_view_is_superseded_failure(
    candidate: &BrowserSessionView,
    sessions: &[BrowserSessionView],
) -> bool {
    if candidate.status != "failed" {
        return false;
    }
    let Some(conversation_id) = candidate
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    sessions.iter().any(|other| {
        other.id != candidate.id
            && other.status != "failed"
            && other
                .conversation_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|other_conversation_id| other_conversation_id == conversation_id)
            && browser_session_profile_exactly_matches(
                candidate.profile_id.as_deref(),
                other.profile_id.as_deref(),
            )
            && other.updated_at > candidate.updated_at
    })
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

async fn set_awaiting_chat_input(
    sessions: &Arc<DashMap<String, BrowserSession>>,
    storage: Option<&crate::storage::Storage>,
    session_id: &str,
    question: &str,
) {
    let snapshot = if let Some(mut entry) = sessions.get_mut(session_id) {
        entry.status = SessionStatus::AwaitingChatInput {
            question: question.trim().to_string(),
        };
        entry.operator_handoff_tx = None;
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
            task_description: public_browser_task_description(&session.task_description),
            channel: session.channel.clone(),
            chat_id: session.conversation_id.clone(),
            profile_id: session.profile_id.clone(),
            profile_name: session.profile_name.clone(),
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
            profile_id,
            profile_name,
            status_detail,
            action_history,
            created_at,
            updated_at,
        } = persisted;
        let public_task_description = public_browser_task_description(&task_description);
        let task_description_changed = public_task_description != task_description;
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
            "waiting_for_user"
            | "waiting_for_operator"
            | "operator_claimed"
            | "awaiting_chat_input" => (
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
                profile_id,
                profile_name,
                task_description: public_task_description,
                status,
                action_history: trimmed_action_history(&action_history),
                created_at,
                updated_at: if changed { now_rfc3339() } else { updated_at },
                loop_token: None,
                operator_handoff_tx: None,
            },
            changed || task_description_changed,
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
            Self::AwaitingChatInput { .. } => "awaiting_chat_input",
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
            Self::AwaitingChatInput { question } => Some(question.clone()),
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
    llm: &'a crate::core::model::llm::LlmClient,
    notify: &'a Arc<dyn Fn(BrowserSessionNotification) + Send + Sync>,
    storage: Option<crate::storage::Storage>,
}

enum BrowserOperatorWaitResult {
    Continue { pending_resume_context: String },
    ReturnToChat { outcome: BrowserLoopOutcome },
}

enum BrowserLoopOutcome {
    Ready { summary: String },
    Completed { summary: String },
    NeedsChatInput { question: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserAskUserResolution {
    AskUser,
    Complete { summary: String, message: String },
    Continue { guidance: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserDoneFinalization {
    Close,
    KeepOpen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserDoneAnswerValidation {
    Complete {
        summary: String,
        message: String,
    },
    Continue {
        guidance: String,
    },
    /// The proposed answer claims an action effect that the visible browser
    /// evidence does not confirm. Rather than report fabricated success, hand
    /// the live browser back to the user to verify or finish it.
    /// `question` is the user-facing prompt.
    HandBack {
        question: String,
    },
}

fn browser_loop_success_status(outcome: BrowserLoopOutcome) -> (SessionStatus, bool) {
    match outcome {
        BrowserLoopOutcome::Ready { summary } => (SessionStatus::Ready { summary }, false),
        BrowserLoopOutcome::Completed { summary } => (SessionStatus::Completed { summary }, true),
        BrowserLoopOutcome::NeedsChatInput { question } => {
            (SessionStatus::AwaitingChatInput { question }, false)
        }
    }
}

fn browser_json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|inner| inner.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn browser_json_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|inner| inner.as_bool())
}

fn default_browser_continue_guidance() -> String {
    "Continue browser work from the current page until the requested outcome is either answered from visible page facts or genuinely needs user input."
        .to_string()
}

fn browser_done_unconfirmed_action_question() -> String {
    "I could not confirm from the visible browser page that the requested action/result actually completed. Please verify or finish it in the live browser, then tell me how to proceed."
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserDoneOutcomeKind {
    InformationDelivery,
    ActionEffect,
    LiveBrowserState,
    Download,
}

fn browser_done_outcome_kind_from_action(
    action: &serde_json::Value,
) -> Option<BrowserDoneOutcomeKind> {
    match action.get("outcome_kind")?.as_str()?.trim() {
        "information_delivery" => Some(BrowserDoneOutcomeKind::InformationDelivery),
        "action_effect" => Some(BrowserDoneOutcomeKind::ActionEffect),
        "live_browser_state" => Some(BrowserDoneOutcomeKind::LiveBrowserState),
        "download" => Some(BrowserDoneOutcomeKind::Download),
        _ => None,
    }
}

fn browser_done_evidence_from_action(action: &serde_json::Value) -> Option<String> {
    action
        .get("evidence")
        .and_then(|inner| inner.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn browser_done_evidence_is_structural(
    kind: BrowserDoneOutcomeKind,
    content: &PageContent,
    _page_changed_since_last_interaction: bool,
) -> bool {
    match kind {
        BrowserDoneOutcomeKind::ActionEffect => false,
        BrowserDoneOutcomeKind::Download => !content.downloads.is_empty(),
        BrowserDoneOutcomeKind::InformationDelivery | BrowserDoneOutcomeKind::LiveBrowserState => {
            browser_snapshot_has_visible_state(content)
        }
    }
}

fn browser_done_answer_local_validation(
    action: &serde_json::Value,
    content: &PageContent,
    page_changed_since_last_interaction: bool,
    summary: &str,
    message: &str,
) -> BrowserDoneAnswerValidation {
    let Some(kind) = browser_done_outcome_kind_from_action(action) else {
        return BrowserDoneAnswerValidation::Continue {
            guidance: "Return done only with an outcome_kind and evidence that match the completed browser outcome."
                .to_string(),
        };
    };

    if kind == BrowserDoneOutcomeKind::ActionEffect
        && !browser_done_evidence_is_structural(kind, content, page_changed_since_last_interaction)
    {
        return BrowserDoneAnswerValidation::HandBack {
            question: browser_done_unconfirmed_action_question(),
        };
    }

    let has_declared_evidence = browser_done_evidence_from_action(action).is_some();
    let has_structural_evidence =
        browser_done_evidence_is_structural(kind, content, page_changed_since_last_interaction);
    if has_declared_evidence && has_structural_evidence {
        BrowserDoneAnswerValidation::Complete {
            summary: summary.to_string(),
            message: message.to_string(),
        }
    } else {
        BrowserDoneAnswerValidation::Continue {
            guidance: "Return done only after the current visible browser state provides evidence for the selected outcome_kind, and include that evidence in the done action."
                .to_string(),
        }
    }
}

fn browser_ask_user_resolution_from_value(
    value: &serde_json::Value,
    fallback_question: &str,
) -> BrowserAskUserResolution {
    let action = browser_json_string(value, &["action", "decision", "outcome"])
        .unwrap_or_else(|| "continue".to_string())
        .to_ascii_lowercase();
    match action.as_str() {
        "complete" | "done" => {
            let summary = browser_json_string(value, &["summary", "answer", "message"])
                .unwrap_or_else(|| fallback_question.trim().to_string());
            let message = browser_json_string(value, &["message", "answer"])
                .unwrap_or_else(|| summary.clone());
            BrowserAskUserResolution::Complete { summary, message }
        }
        "continue" | "continue_browser" | "keep_working" => {
            let guidance = browser_json_string(value, &["guidance", "reason", "next_step"])
                .unwrap_or_else(default_browser_continue_guidance);
            BrowserAskUserResolution::Continue { guidance }
        }
        "ask_user" => {
            let requires_human = browser_json_bool(value, "requires_human").unwrap_or(false);
            if requires_human {
                BrowserAskUserResolution::AskUser
            } else {
                let guidance = browser_json_string(value, &["guidance", "next_step"])
                    .unwrap_or_else(default_browser_continue_guidance);
                BrowserAskUserResolution::Continue { guidance }
            }
        }
        _ => BrowserAskUserResolution::Continue {
            guidance: default_browser_continue_guidance(),
        },
    }
}

fn browser_ask_user_resolution_when_verdict_unavailable(
    _question: &str,
) -> BrowserAskUserResolution {
    // A proposed user checkpoint may be overridden only by a valid structured
    // verifier verdict. If the verifier is unavailable or emits no JSON, keep
    // the browser loop's handoff rather than continuing autonomous actions.
    BrowserAskUserResolution::AskUser
}

fn browser_snapshot_completion_from_value(
    value: &serde_json::Value,
    fallback_guidance: &str,
) -> BrowserDoneAnswerValidation {
    let fallback_guidance = fallback_guidance.trim();
    let fallback_guidance = if fallback_guidance.is_empty() {
        "Continue browser work until the requested outcome is backed by visible page evidence."
    } else {
        fallback_guidance
    };
    let action = browser_json_string(value, &["action", "decision", "outcome"])
        .unwrap_or_else(|| "continue".to_string())
        .to_ascii_lowercase();
    match action.as_str() {
        "complete" | "done" => {
            let message = browser_json_string(value, &["message", "answer"]);
            let summary = browser_json_string(value, &["summary"]).or_else(|| {
                message
                    .as_ref()
                    .map(|message| truncate_browser_text(message, 160))
            });
            match (summary, message) {
                (Some(summary), Some(message)) => {
                    BrowserDoneAnswerValidation::Complete { summary, message }
                }
                _ => BrowserDoneAnswerValidation::Continue {
                    guidance: fallback_guidance.to_string(),
                },
            }
        }
        _ => {
            let guidance = browser_json_string(value, &["guidance", "reason", "next_step"])
                .unwrap_or_else(|| fallback_guidance.to_string());
            BrowserDoneAnswerValidation::Continue { guidance }
        }
    }
}

fn browser_done_finalization_from_value(value: &serde_json::Value) -> BrowserDoneFinalization {
    let _ = value;
    BrowserDoneFinalization::Close
}

fn browser_snapshot_has_visible_state(content: &PageContent) -> bool {
    let url = content.url.trim();
    let title = content.title.trim();
    let body = content.body_text.trim();
    if url.eq_ignore_ascii_case("about:blank")
        && title.is_empty()
        && body.is_empty()
        && content.elements.is_empty()
    {
        return false;
    }
    !url.is_empty() || !title.is_empty() || !body.is_empty() || !content.elements.is_empty()
}

/// True when a navigated real page's snapshot is still an un-hydrated shell: a
/// non-blank URL but empty body text AND no interactive elements. JS apps (SPAs
/// like Gmail, Notion, Linear, etc.) render exactly this shell at
/// `domcontentloaded`, before their content mounts via XHR. A snapshot taken
/// then makes downstream deciders (access-blocker / ask_user) wrongly read an
/// empty page as "not signed in" / blocked. Generic by construction: keys only
/// on emptiness + a real URL — never on site, selector, or wording.
fn browser_snapshot_is_unhydrated(content: &PageContent) -> bool {
    let url = content.url.trim();
    if url.is_empty() || url.eq_ignore_ascii_case("about:blank") {
        return false;
    }
    content.body_text.trim().is_empty() && content.elements.is_empty()
}

fn browser_snapshot_has_structural_access_gate(content: &PageContent) -> bool {
    if !browser_snapshot_has_visible_state(content) || browser_snapshot_is_unhydrated(content) {
        return false;
    }
    content
        .elements
        .iter()
        .any(|element| element.r#type.eq_ignore_ascii_case("password"))
}

fn browser_access_blocker_reason_from_value(value: &serde_json::Value) -> Option<String> {
    if value
        .get("blocked")
        .and_then(|inner| inner.as_bool())
        .unwrap_or(false)
    {
        Some(
            browser_json_string(value, &["reason", "summary", "message"])
                .unwrap_or_else(|| {
                    "The current page is blocking the requested browser task and requires a human account or security action before AgentArk can continue."
                        .to_string()
                }),
        )
    } else {
        None
    }
}

#[derive(Default)]
struct BrowserLoopStallTracker {
    last_key: Option<String>,
    repeat_count: u32,
}

impl BrowserLoopStallTracker {
    fn record(&mut self, key: impl Into<String>) -> u32 {
        let key = key.into();
        if self.last_key.as_deref() == Some(key.as_str()) {
            self.repeat_count = self.repeat_count.saturating_add(1);
        } else {
            self.last_key = Some(key);
            self.repeat_count = 1;
        }
        self.repeat_count
    }

    fn reset(&mut self) {
        self.last_key = None;
        self.repeat_count = 0;
    }
}

#[derive(Default)]
struct BrowserLoopProgressTracker {
    last_key: Option<String>,
    repeat_count: u32,
}

impl BrowserLoopProgressTracker {
    fn record_action(&mut self, content: &PageContent, action: &serde_json::Value) -> u32 {
        let key = format!(
            "state={}|action={}",
            browser_page_progress_key(content),
            browser_action_progress_key(action)
        );
        if self.last_key.as_deref() == Some(key.as_str()) {
            self.repeat_count = self.repeat_count.saturating_add(1);
        } else {
            self.last_key = Some(key);
            self.repeat_count = 1;
        }
        self.repeat_count
    }
}

#[derive(Debug, Clone)]
struct BrowserInteractionProgressProbe {
    before_page_key: String,
    expects_visible_progress: bool,
}

impl BrowserInteractionProgressProbe {
    fn new(before_page_key: String, expects_visible_progress: bool) -> Self {
        Self {
            before_page_key,
            expects_visible_progress,
        }
    }
}

fn browser_observe_interaction_progress(
    progress_probe: BrowserInteractionProgressProbe,
    current_page_key: &str,
    no_visible_progress_interaction_count: &mut u32,
) -> bool {
    let page_changed_since_last_interaction = progress_probe.before_page_key != current_page_key;
    if progress_probe.expects_visible_progress {
        if page_changed_since_last_interaction {
            *no_visible_progress_interaction_count = 0;
        } else {
            *no_visible_progress_interaction_count =
                no_visible_progress_interaction_count.saturating_add(1);
        }
    }
    page_changed_since_last_interaction
}

fn browser_page_progress_key(content: &PageContent) -> String {
    let elements = content
        .elements
        .iter()
        .take(50)
        .map(|element| {
            serde_json::json!({
                "index": element.index,
                "tag": element.tag.trim(),
                "type": element.r#type.trim(),
                "text": element.text.trim(),
                "name": element.name.trim(),
                "id": element.id.trim(),
                "href": element.href.trim(),
                "x": element.x,
                "y": element.y,
            })
        })
        .collect::<Vec<_>>();
    let downloads = content
        .downloads
        .iter()
        .take(20)
        .map(|download| {
            serde_json::json!({
                "id": download.id.trim(),
                "filename": download.filename.trim(),
                "path": download.path.trim(),
                "bytes": download.bytes,
                "url": download.url.trim(),
                "status": download.status.trim(),
            })
        })
        .collect::<Vec<_>>();
    let progress_state = serde_json::json!({
        "url": content.url.trim(),
        "title": content.title.trim(),
        "body_text": truncate_browser_text(content.body_text.trim(), 8_000),
        "elements": elements,
        "downloads": downloads,
        "download_dir": content.download_dir.as_deref().unwrap_or("").trim(),
    });
    stable_browser_value_key(&progress_state)
}

fn browser_action_progress_key(action: &serde_json::Value) -> String {
    stable_browser_value_key(action)
}

fn stable_browser_value_key(value: &serde_json::Value) -> String {
    format!("{:016x}", stable_browser_value_hash(value))
}

fn stable_browser_value_hash(value: &serde_json::Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;

    let mut hasher = DefaultHasher::new();
    browser_hash_value(value, &mut hasher);
    hasher.finish()
}

fn browser_hash_value<H: std::hash::Hasher>(value: &serde_json::Value, state: &mut H) {
    use std::hash::Hash;

    match value {
        serde_json::Value::Null => {
            0u8.hash(state);
        }
        serde_json::Value::Bool(value) => {
            1u8.hash(state);
            value.hash(state);
        }
        serde_json::Value::Number(value) => {
            2u8.hash(state);
            value.to_string().hash(state);
        }
        serde_json::Value::String(value) => {
            3u8.hash(state);
            value.trim().hash(state);
        }
        serde_json::Value::Array(values) => {
            4u8.hash(state);
            values.len().hash(state);
            for value in values {
                browser_hash_value(value, state);
            }
        }
        serde_json::Value::Object(values) => {
            5u8.hash(state);
            values.len().hash(state);
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(left, _)| *left);
            for (key, value) in entries {
                key.hash(state);
                browser_hash_value(value, state);
            }
        }
    }
}

fn browser_action_kind(action: &serde_json::Value) -> String {
    action
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("browser action")
        .to_ascii_lowercase()
}

fn repeated_browser_action_guidance(action_kind: &str) -> String {
    format!(
        "The previous {action_kind} action was requested again from an unchanged page snapshot. Do not repeat an action that has not changed the visible browser state. Inspect the current snapshot, choose a different next milestone, return done if the requested outcome is already visibly satisfied, or ask_user only when human input is genuinely required."
    )
}

async fn browser_content_snapshot(
    integration: &BrowserIntegration,
    sidecar_id: &str,
) -> Result<PageContent> {
    let mut last_error = None;
    for attempt in 0..CONTENT_SNAPSHOT_ATTEMPTS {
        match integration.get_content(sidecar_id).await {
            Ok(content) => {
                // Re-poll an un-hydrated SPA shell (real URL, empty body + no
                // elements) before returning, so no downstream decider — for
                // ANY browser flow ever classifies a transient empty page as
                // blocked / requiring access.
                // Reuses the same bounded budget + backoff as transport errors,
                // stops the instant content hydrates, and returns the snapshot
                // on the final attempt regardless so a genuinely-sparse page
                // still resolves.
                if attempt + 1 < CONTENT_SNAPSHOT_ATTEMPTS
                    && browser_snapshot_is_unhydrated(&content)
                {
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        250 + (attempt as u64 * 250),
                    ))
                    .await;
                    continue;
                }
                return Ok(content);
            }
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < CONTENT_SNAPSHOT_ATTEMPTS {
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        250 + (attempt as u64 * 250),
                    ))
                    .await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Unable to read browser page snapshot")))
}

async fn browser_loop_content_snapshot(ctx: &BrowserLoopContext<'_>) -> Result<PageContent> {
    browser_content_snapshot(ctx.integration.as_ref(), ctx.sidecar_id).await
}

async fn wait_for_browser_operator_input(
    session_id: &str,
    ctx: &BrowserLoopContext<'_>,
    question: &str,
    _content: &PageContent,
    history: &mut Vec<String>,
) -> Result<BrowserOperatorWaitResult> {
    let question = question.trim();
    let question = if question.is_empty() {
        "I need your input before I can continue this browser task."
    } else {
        question
    };
    set_awaiting_chat_input(ctx.sessions, ctx.storage.as_ref(), session_id, question).await;
    (ctx.notify)(BrowserSessionNotification::needs_input(
        session_id,
        question.to_string(),
        None,
    ));
    history.push("Paused browser task for chat input".to_string());
    sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, history).await;
    Ok(BrowserOperatorWaitResult::ReturnToChat {
        outcome: BrowserLoopOutcome::NeedsChatInput {
            question: question.to_string(),
        },
    })
}

fn stalled_browser_question(content: &PageContent, reason: &str) -> String {
    let title = content.title.trim();
    let url = content.url.trim();
    let location = if title.is_empty() { url } else { title };
    let page_status = browser_visible_page_status(content);
    format!(
        "The browser automation paused because {reason}. Current page: {location}. {page_status} Please verify or finish the next step in the live browser, then tell me how to proceed."
    )
}

fn browser_visible_page_status(content: &PageContent) -> String {
    let mut parts = Vec::new();
    let url = content.url.trim();
    if !url.is_empty() {
        parts.push(format!("URL: {url}."));
    }
    let body = content
        .body_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !body.is_empty() {
        parts.push(format!(
            "Visible text starts: {}.",
            truncate_browser_text(&body, 240)
        ));
    }
    if content.elements.is_empty() {
        parts.push("No visible interactive elements were reported.".to_string());
    } else {
        parts.push(format!(
            "{} visible interactive elements were reported.",
            content.elements.len()
        ));
    }
    if !content.downloads.is_empty() {
        parts.push(format!(
            "{} download records were reported.",
            content.downloads.len()
        ));
    }
    parts.join(" ")
}

async fn run_browser_loop(
    session_id: &str,
    ctx: BrowserLoopContext<'_>,
) -> Result<BrowserLoopOutcome> {
    let site_constraint = browser_task_site_constraint(ctx.task);
    let site_constraint_instruction = site_constraint
        .as_ref()
        .map(|constraint| {
            format!(
                "- The task explicitly targets {}. Keep navigation on {} and same-site subdomains unless the user explicitly names another site. Do not switch to unrelated external sites as a fallback.\n         ",
                constraint.source_host,
                constraint.allowed_host_suffix
            )
        })
        .unwrap_or_default();
    let browser_system_prompt = format!(
        "You are a browser automation agent. Your task: {}\n\n\
         Respond with exactly one JSON action per step:\n\
         - {{\"action\":\"navigate\",\"url\":\"...\"}}\n\
         - {{\"action\":\"click\",\"element_index\":N}}\n\
         - {{\"action\":\"click\",\"text\":\"...\"}}\n\
         - {{\"action\":\"click\",\"selector\":\"...\"}}\n\
         - {{\"action\":\"click\",\"x\":N,\"y\":N}}\n\
         - {{\"action\":\"type_text\",\"text\":\"...\",\"element_index\":N,\"clear\":false}}\n\
         - {{\"action\":\"type_text\",\"text\":\"...\",\"selector\":\"...\",\"clear\":false}}\n\
         - {{\"action\":\"scroll\",\"direction\":\"down\"}}\n\
         - {{\"action\":\"press_key\",\"key\":\"Enter\"}}\n\
         - {{\"action\":\"ask_user\",\"question\":\"...\"}} when the task asks you to pause for the user, when the next step depends on a user decision, or when the site needs a real human to take over the live browser for sensitive account/security operations. For decision checkpoints and user choices, the user may answer in chat. For sensitive site operations, the user may use live browser handoff. Do not ask the user to paste secret authentication material into chat when live browser handoff will work.\n\
         - {{\"action\":\"notify\",\"message\":\"...\"}}\n\
         - {{\"action\":\"done\",\"outcome_kind\":\"information_delivery|action_effect|live_browser_state|download\",\"summary\":\"...\",\"message\":\"...\",\"evidence\":\"...\"}}\n\n\
         Guardrails:\n\
         {}\
         - Treat the task as a multi-step objective. If it includes a gated access step plus later explicit website work, use live handoff only for the gated step, then continue the remaining task yourself after the handoff resumes.\n\
         - Decompose the objective into ordered milestones internally, but output only the next single browser action. Never output multiple JSON objects, arrays of actions, numbered steps, or prose around the action.\n\
         - Base every decision on the current page snapshot shown in the latest step.\n\
         - Prefer element_index for visible interactive elements listed as `element N` when clicking or typing; do not turn `element N` into a CSS selector.\n\
         - Use the current page snapshot and previous action history to decide the next incomplete milestone. Do not restart earlier milestones that the current page already proves complete.\n\
         - When the visible page already contains the active workflow surface, continue operating that surface instead of opening a duplicate one.\n\
         - Treat any operator note from a live handoff as an unverified hint, never as proof.\n\
         - After a live handoff resumes, inspect the fresh page state before deciding what changed.\n\
         - Internally distinguish the requested outcome before choosing ask_user or done: human-input checkpoints require a user decision or site operation; live-browser state checkpoints require leaving the browser open for the user to inspect/control; information-delivery outcomes require reporting facts that are visible in the current page snapshot; action-effect outcomes require direct visible confirmation or destination evidence in the current page/download records. A successful browser control result or generic page change is not enough to prove an external action effect.\n\
         - For information-delivery outcomes, keep using the current page snapshot until the requested visible facts are available, then return done with those facts in summary/message. Do not use ask_user as a substitute for the final answer, and do not merely say the information is visible.\n\
         - For download outcomes, use the browser until the download record appears in the current page snapshot. If the task also asks to use the file elsewhere in AgentArk, finish with the downloaded file record so the outer agent can call the appropriate file or document tool.\n\
         - For live-browser state checkpoints, return ask_user only when the user needs to inspect/control the browser next or the task explicitly asked you to pause at that state.\n\
         - If the task already states what should happen after a gated checkpoint, continue directly instead of asking for confirmation.\n\
         - Use ask_user only when a real human must operate the site, when the user explicitly requested a checkpoint, or when the task is genuinely underspecified even after considering the task text and current page.\n\
         - Only claim a gated checkpoint succeeded when the current page directly supports that claim.\n\
         - If the requested checkpoint cannot be directly verified from the current page, summarize the visible state conservatively instead of inventing hidden state.\n\
         - Use done when the requested outcome is completed. Always set outcome_kind to exactly one of information_delivery, action_effect, live_browser_state, or download. Always include evidence: for information_delivery/download, cite the visible page facts or download record used; for live_browser_state, describe the visible state now left for the user; for action_effect, cite the direct visible confirmation or destination evidence. For information-delivery outcomes, done must include the observed page-derived answer.\n\
         - Only stop for user input when the explicit task is complete with no further implied action, a real human must operate the site, or browser controls have stopped making visible progress.",
        ctx.task, site_constraint_instruction
    );

    let mut history: Vec<String> = Vec::new();
    let mut pending_resume_context: Option<String> = None;
    let mut last_content: Option<PageContent> = None;
    let mut navigation_progress = BrowserNavigationProgress::default();
    let mut stall_tracker = BrowserLoopStallTracker::default();
    let mut progress_tracker = BrowserLoopProgressTracker::default();
    let mut last_interaction_progress_probe: Option<BrowserInteractionProgressProbe> = None;
    let mut no_visible_progress_interaction_count = 0u32;

    for iteration in 0..MAX_ITERATIONS {
        let content = browser_loop_content_snapshot(&ctx).await?;
        let current_page_key = browser_page_progress_key(&content);
        let mut page_changed_since_last_interaction = false;
        if let Some(progress_probe) = last_interaction_progress_probe.take() {
            page_changed_since_last_interaction = browser_observe_interaction_progress(
                progress_probe,
                &current_page_key,
                &mut no_visible_progress_interaction_count,
            );
        }
        navigation_progress.observe_page_url(&content.url);
        last_content = Some(content.clone());
        if no_visible_progress_interaction_count >= MAX_NO_VISIBLE_PROGRESS_INTERACTIONS {
            let reason = "recent browser control actions did not produce visible page progress";
            history.push(format!(
                "Step {}: Browser controls are no longer producing visible progress",
                iteration + 1
            ));
            let question = stalled_browser_question(&content, reason);
            history.push(format!(
                "Step {}: Asking user after repeated no-progress browser controls",
                iteration + 1
            ));
            match wait_for_browser_operator_input(
                session_id,
                &ctx,
                &question,
                &content,
                &mut history,
            )
            .await?
            {
                BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                    return Ok(outcome);
                }
                BrowserOperatorWaitResult::Continue {
                    pending_resume_context: resume_context,
                } => {
                    pending_resume_context = Some(resume_context);
                    no_visible_progress_interaction_count = 0;
                    stall_tracker.reset();
                }
            }
            continue;
        }
        let elements_str = content
            .elements
            .iter()
            .take(30)
            .map(format_browser_loop_element)
            .collect::<Vec<_>>()
            .join("\n");

        let body_preview = if content.body_text.chars().count() > 2000 {
            format!(
                "{}...",
                content.body_text.chars().take(2000).collect::<String>()
            )
        } else {
            content.body_text.clone()
        };
        let downloads_str = format_browser_downloads(&content);

        let mut messages = vec![format!(
            "Step {}/{}\nURL: {}\nTitle: {}\n\nPage text:\n{}\n\nInteractive elements:\n{}\n\nDownloaded files:\n{}",
            iteration + 1,
            MAX_ITERATIONS,
            content.url,
            content.title,
            body_preview,
            elements_str,
            downloads_str
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
        let (json_str, ignored_trailing_output) =
            extract_first_json_object(&response_text).unwrap_or((&response_text, false));
        let action: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(value) => value,
            Err(_) => {
                history.push(format!("Step {}: Error parsing action", iteration + 1));
                sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history)
                    .await;
                continue;
            }
        };
        if ignored_trailing_output {
            history.push(format!(
                "Step {}: Model returned more than one action; executing the first valid action only",
                iteration + 1
            ));
        }
        let progress_repeat_count = progress_tracker.record_action(&content, &action);
        if progress_repeat_count >= 2 {
            let action_kind = browser_action_kind(&action);
            let reason =
                "the same browser action is being requested again from an unchanged page state";
            history.push(format!(
                "Step {}: Skipped repeated {} action because {}",
                iteration + 1,
                action_kind,
                reason
            ));
            if progress_repeat_count == 2 {
                pending_resume_context = Some(repeated_browser_action_guidance(&action_kind));
                sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history)
                    .await;
                continue;
            }
            match validate_browser_snapshot_completion(&ctx, &content, &history, reason).await {
                BrowserDoneAnswerValidation::Complete { summary, message } => {
                    let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
                    history.push(format!(
                        "Step {}: DONE after no-progress snapshot validation - {}",
                        iteration + 1,
                        summary
                    ));
                    sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history)
                        .await;
                    (ctx.notify)(BrowserSessionNotification::completed(
                        session_id, message, screenshot,
                    ));
                    return Ok(BrowserLoopOutcome::Completed { summary });
                }
                BrowserDoneAnswerValidation::Continue { guidance } => {
                    history.push(format!(
                        "Step {}: No-progress snapshot validation could not complete - {}",
                        iteration + 1,
                        guidance
                    ));
                }
                BrowserDoneAnswerValidation::HandBack { question } => {
                    // Snapshot-completion never emits HandBack today; treat it
                    // like Continue and fall through to the user handoff below.
                    history.push(format!(
                        "Step {}: No-progress snapshot validation could not confirm completion - {}",
                        iteration + 1,
                        question
                    ));
                }
            }
            let question = stalled_browser_question(&content, reason);
            history.push(format!(
                "Step {}: Asking user after repeated no-progress browser actions",
                iteration + 1
            ));
            match wait_for_browser_operator_input(
                session_id,
                &ctx,
                &question,
                &content,
                &mut history,
            )
            .await?
            {
                BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                    return Ok(outcome);
                }
                BrowserOperatorWaitResult::Continue {
                    pending_resume_context: resume_context,
                } => {
                    pending_resume_context = Some(resume_context);
                    stall_tracker.reset();
                }
            }
            continue;
        }

        match action
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
        {
            "navigate" => {
                if let Some(url) = action.get("url").and_then(|value| value.as_str()) {
                    if let Some(constraint) = site_constraint
                        .as_ref()
                        .filter(|constraint| !constraint.allows_navigation(url, &content.url))
                    {
                        let reason = constraint.navigation_rejection_reason(url, &content.url);
                        let allowed_host_suffix = constraint.allowed_host_suffix.clone();
                        let source_host = constraint.source_host.clone();
                        tracing::debug!(
                            session_id,
                            reason,
                            "Browser loop blocked off-site navigation"
                        );
                        let repeat_count = stall_tracker
                            .record(format!("navigate:site_constraint:{allowed_host_suffix}"));
                        history.push(format!(
                            "Step {}: Skipped navigation because {}; current page remains {} ({})",
                            iteration + 1,
                            reason,
                            content.url,
                            content.title
                        ));
                        if repeat_count == 1 {
                            pending_resume_context = Some(format!(
                                "The previous navigation request was skipped because {reason}. The task explicitly targets {source_host}; keep browser actions on {allowed_host_suffix}. Use the current page, same-site links, or same-site search. If that cannot complete the requested task, return ask_user."
                            ));
                        } else {
                            let question = stalled_browser_question(&content, &reason);
                            history.push(format!(
                                "Step {}: Asking user after repeated off-site navigation attempts",
                                iteration + 1
                            ));
                            match wait_for_browser_operator_input(
                                session_id,
                                &ctx,
                                &question,
                                &content,
                                &mut history,
                            )
                            .await?
                            {
                                BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                                    return Ok(outcome);
                                }
                                BrowserOperatorWaitResult::Continue {
                                    pending_resume_context: resume_context,
                                } => {
                                    pending_resume_context = Some(resume_context);
                                    stall_tracker.reset();
                                }
                            }
                        }
                    } else {
                        match navigation_progress.decide_navigation(url, &content.url) {
                            BrowserNavigationDecision::Allow => {
                                let (final_url, title) =
                                    ctx.integration.navigate(ctx.sidecar_id, url).await?;
                                last_interaction_progress_probe =
                                    Some(BrowserInteractionProgressProbe::new(
                                        current_page_key.clone(),
                                        true,
                                    ));
                                navigation_progress.observe_page_url(&final_url);
                                stall_tracker.reset();
                                history.push(format!(
                                    "Step {}: Navigated to {} ({})",
                                    iteration + 1,
                                    final_url,
                                    title
                                ));
                            }
                            BrowserNavigationDecision::Reject { reason } => {
                                tracing::debug!(
                                    session_id,
                                    reason,
                                    "Browser loop rejected no-progress navigation"
                                );
                                let normalized_target = normalize_browser_navigation_url(url)
                                    .unwrap_or_else(|| url.to_string());
                                let repeat_count = stall_tracker
                                    .record(format!("navigate:{reason}:{normalized_target}"));
                                history.push(format!(
                                "Step {}: Skipped navigation because {}; current page remains {} ({})",
                                iteration + 1,
                                reason,
                                content.url,
                                content.title
                            ));
                                if repeat_count == 1 {
                                    pending_resume_context = Some(format!(
                                        "The previous navigation request was skipped because {reason}. Do not request that same navigation again. Use the current page snapshot to choose the next incomplete milestone. If the current page satisfies an information-delivery outcome, return done with the visible facts. If it satisfies an explicit user checkpoint, return ask_user. If the task is complete, return done."
                                    ));
                                } else {
                                    let question = stalled_browser_question(&content, reason);
                                    history.push(format!(
                                    "Step {}: Asking user after repeated no-progress navigation",
                                    iteration + 1
                                ));
                                    match wait_for_browser_operator_input(
                                        session_id,
                                        &ctx,
                                        &question,
                                        &content,
                                        &mut history,
                                    )
                                    .await?
                                    {
                                        BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                                            return Ok(outcome);
                                        }
                                        BrowserOperatorWaitResult::Continue {
                                            pending_resume_context: resume_context,
                                        } => {
                                            pending_resume_context = Some(resume_context);
                                            stall_tracker.reset();
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    history.push(format!("Step {}: Missing navigate URL", iteration + 1));
                }
            }
            "click" => {
                let element_index = action
                    .get("element_index")
                    .or_else(|| action.get("index"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                let selector = action.get("selector").and_then(|v| v.as_str());
                let text = action.get("text").and_then(|v| v.as_str());
                let x = action.get("x").and_then(|v| v.as_i64()).map(|v| v as i32);
                let y = action.get("y").and_then(|v| v.as_i64()).map(|v| v as i32);
                let label = element_index
                    .map(|index| format!("element {}", index))
                    .or_else(|| text.or(selector).map(str::to_string))
                    .unwrap_or_else(|| "element".to_string());
                match ctx
                    .integration
                    .click(ctx.sidecar_id, selector, text, x, y, element_index)
                    .await
                {
                    Ok(()) => {
                        last_interaction_progress_probe = Some(
                            BrowserInteractionProgressProbe::new(current_page_key.clone(), true),
                        );
                        stall_tracker.reset();
                        history.push(format!("Step {}: Clicked '{}'", iteration + 1, label));
                    }
                    Err(error) => {
                        let repeat_count =
                            stall_tracker.record(format!("click_failed:{label}:{error}"));
                        history.push(format!(
                            "Step {}: Click failed for '{}': {}",
                            iteration + 1,
                            label,
                            error
                        ));
                        if repeat_count >= 2 {
                            let question =
                                stalled_browser_question(&content, "the same click keeps failing");
                            history.push(format!(
                                "Step {}: Asking user after repeated click failure",
                                iteration + 1
                            ));
                            match wait_for_browser_operator_input(
                                session_id,
                                &ctx,
                                &question,
                                &content,
                                &mut history,
                            )
                            .await?
                            {
                                BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                                    return Ok(outcome);
                                }
                                BrowserOperatorWaitResult::Continue {
                                    pending_resume_context: resume_context,
                                } => {
                                    pending_resume_context = Some(resume_context);
                                    stall_tracker.reset();
                                }
                            }
                        }
                    }
                }
            }
            "type_text" => {
                let text = action.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let selector = action.get("selector").and_then(|v| v.as_str());
                let element_index = action
                    .get("element_index")
                    .or_else(|| action.get("index"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                let clear = action
                    .get("clear")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match ctx
                    .integration
                    .type_text(ctx.sidecar_id, text, selector, element_index, clear)
                    .await
                {
                    Ok(()) => {
                        last_interaction_progress_probe = Some(
                            BrowserInteractionProgressProbe::new(current_page_key.clone(), false),
                        );
                        stall_tracker.reset();
                        history.push(format!(
                            "Step {}: Typed {} chars",
                            iteration + 1,
                            text.len()
                        ));
                    }
                    Err(error) => {
                        let target_label = element_index
                            .map(|index| format!(" for element {}", index))
                            .or_else(|| {
                                selector.map(|selector| format!(" for selector '{}'", selector))
                            })
                            .unwrap_or_default();
                        let repeat_count =
                            stall_tracker.record(format!("type_failed:{target_label}:{error}"));
                        history.push(format!(
                            "Step {}: Type failed{}: {}",
                            iteration + 1,
                            target_label,
                            error
                        ));
                        if repeat_count >= 2 {
                            let question = stalled_browser_question(
                                &content,
                                "the same typing action keeps failing",
                            );
                            history.push(format!(
                                "Step {}: Asking user after repeated typing failure",
                                iteration + 1
                            ));
                            match wait_for_browser_operator_input(
                                session_id,
                                &ctx,
                                &question,
                                &content,
                                &mut history,
                            )
                            .await?
                            {
                                BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                                    return Ok(outcome);
                                }
                                BrowserOperatorWaitResult::Continue {
                                    pending_resume_context: resume_context,
                                } => {
                                    pending_resume_context = Some(resume_context);
                                    stall_tracker.reset();
                                }
                            }
                        }
                    }
                }
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
                last_interaction_progress_probe = Some(BrowserInteractionProgressProbe::new(
                    current_page_key.clone(),
                    true,
                ));
                stall_tracker.reset();
                history.push(format!("Step {}: Scrolled {}", iteration + 1, dir));
            }
            "press_key" => {
                let key = action
                    .get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Enter");
                match ctx.integration.press_key(ctx.sidecar_id, key).await {
                    Ok(()) => {
                        last_interaction_progress_probe = Some(
                            BrowserInteractionProgressProbe::new(current_page_key.clone(), true),
                        );
                        stall_tracker.reset();
                        history.push(format!("Step {}: Pressed {}", iteration + 1, key));
                    }
                    Err(error) => history.push(format!(
                        "Step {}: Key press '{}' failed: {}",
                        iteration + 1,
                        key,
                        error
                    )),
                }
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
                match validate_browser_ask_user_resolution(&ctx, question, &content, &history).await
                {
                    BrowserAskUserResolution::Complete { summary, message } => {
                        let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
                        history.push(format!(
                            "Step {}: DONE after validating proposed user checkpoint - {}",
                            iteration + 1,
                            summary
                        ));
                        sync_session_history(
                            ctx.sessions,
                            ctx.storage.as_ref(),
                            session_id,
                            &history,
                        )
                        .await;
                        (ctx.notify)(BrowserSessionNotification::completed(
                            session_id, message, screenshot,
                        ));
                        return Ok(BrowserLoopOutcome::Completed { summary });
                    }
                    BrowserAskUserResolution::Continue { guidance } => {
                        history.push(format!(
                            "Step {}: Rejected proposed user checkpoint; continuing browser work",
                            iteration + 1
                        ));
                        pending_resume_context = Some(guidance);
                        stall_tracker.reset();
                    }
                    BrowserAskUserResolution::AskUser => {
                        history.push(format!("Step {}: Asking user: {}", iteration + 1, question));
                        match wait_for_browser_operator_input(
                            session_id,
                            &ctx,
                            question,
                            &content,
                            &mut history,
                        )
                        .await?
                        {
                            BrowserOperatorWaitResult::ReturnToChat { outcome } => {
                                return Ok(outcome);
                            }
                            BrowserOperatorWaitResult::Continue {
                                pending_resume_context: resume_context,
                            } => {
                                pending_resume_context = Some(resume_context);
                                stall_tracker.reset();
                            }
                        }
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
                let (summary, message) = match validate_browser_done_answer(
                    &ctx,
                    &action,
                    &content,
                    page_changed_since_last_interaction,
                    &history,
                    summary,
                    message,
                )
                .await
                {
                    BrowserDoneAnswerValidation::Complete { summary, message } => {
                        (summary, message)
                    }
                    BrowserDoneAnswerValidation::Continue { guidance } => {
                        history.push(format!(
                            "Step {}: Completion evidence check requested more browser work - {}",
                            iteration + 1,
                            guidance
                        ));
                        sync_session_history(
                            ctx.sessions,
                            ctx.storage.as_ref(),
                            session_id,
                            &history,
                        )
                        .await;
                        pending_resume_context = Some(guidance);
                        stall_tracker.reset();
                        continue;
                    }
                    BrowserDoneAnswerValidation::HandBack { question } => {
                        // The answer claimed an action effect the page did not
                        // confirm. Hand the live browser back to the user to
                        // verify/finish, instead of reporting fabricated success.
                        history.push(format!(
                                "Step {}: Completion evidence check could not confirm the claimed action effect; handing the live browser back to the user - {}",
                                iteration + 1,
                                question
                            ));
                        sync_session_history(
                            ctx.sessions,
                            ctx.storage.as_ref(),
                            session_id,
                            &history,
                        )
                        .await;
                        let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
                        set_awaiting_chat_input(
                            ctx.sessions,
                            ctx.storage.as_ref(),
                            session_id,
                            &question,
                        )
                        .await;
                        (ctx.notify)(BrowserSessionNotification::needs_input(
                            session_id,
                            question.clone(),
                            screenshot,
                        ));
                        return Ok(BrowserLoopOutcome::NeedsChatInput { question });
                    }
                };
                let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
                history.push(format!(
                    "Step {}: DONE (closing browser session) - {}",
                    iteration + 1,
                    summary
                ));
                sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history)
                    .await;
                (ctx.notify)(BrowserSessionNotification::completed(
                    session_id,
                    message.clone(),
                    screenshot,
                ));
                return Ok(BrowserLoopOutcome::Completed { summary });
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

    let question = "I reached the browser step limit while working on this page. The live browser is ready for you to inspect or hand back so I can continue from chat.".to_string();
    let (tx, rx) = oneshot::channel::<OperatorHandoffOutcome>();
    set_waiting_for_operator(
        ctx.sessions,
        ctx.storage.as_ref(),
        session_id,
        question.clone(),
        tx,
    )
    .await;
    let screenshot = ctx.integration.screenshot(ctx.sidecar_id).await.ok();
    (ctx.notify)(BrowserSessionNotification::needs_input(
        session_id, question, screenshot,
    ));
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(OPERATOR_HANDOFF_TIMEOUT_SECS),
        rx,
    )
    .await
    {
        Ok(Ok(outcome)) => {
            history.push("Step limit handoff completed by operator".to_string());
            sync_session_history(ctx.sessions, ctx.storage.as_ref(), session_id, &history).await;
            if outcome.resume_in_chat {
                Ok(BrowserLoopOutcome::Ready {
                    summary:
                        "Browser step limit handoff returned control. Continue from chat to inspect the current page."
                            .to_string(),
                })
            } else {
                let note = outcome.note.trim();
                let visible_state = last_content
                    .as_ref()
                    .map(describe_page_snapshot)
                    .unwrap_or_else(|| "No page snapshot was available.".to_string());
                Ok(BrowserLoopOutcome::Ready {
                    summary: if note.is_empty() {
                        format!(
                            "Browser is waiting after reaching the step limit.\n{visible_state}"
                        )
                    } else {
                        format!(
                            "Browser is waiting after reaching the step limit.\n{visible_state}\nOperator note: {note}"
                        )
                    },
                })
            }
        }
        Ok(Err(_)) => Err(anyhow!("Live browser handoff channel closed unexpectedly")),
        Err(_) => Err(anyhow!(
            "Timed out waiting for the live browser handoff to finish"
        )),
    }
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
    if let Some(download_dir) = content
        .download_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Download directory: {}", download_dir));
    }
    let downloads = format_browser_downloads(content);
    if !downloads.trim().is_empty() {
        lines.push(format!("Downloads:\n{}", downloads));
    }
    lines.join("\n")
}

fn extract_first_json_object(text: &str) -> Option<(&str, bool)> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = index + ch.len_utf8();
                    let trailing = text[end..].trim();
                    return start.map(|start| (&text[start..end], !trailing.is_empty()));
                }
            }
            _ => {}
        }
    }

    None
}

fn browser_loop_element_label(element: &PageElement) -> &str {
    if !element.text.trim().is_empty() {
        element.text.trim()
    } else if !element.name.trim().is_empty() {
        element.name.trim()
    } else if !element.id.trim().is_empty() {
        element.id.trim()
    } else if !element.href.trim().is_empty() {
        element.href.trim()
    } else {
        "unlabeled"
    }
}

fn format_browser_loop_element(element: &PageElement) -> String {
    let mut details = Vec::new();
    if !element.r#type.trim().is_empty() {
        details.push(format!("type={}", element.r#type.trim()));
    }
    if !element.name.trim().is_empty() {
        details.push(format!("name={}", element.name.trim()));
    }
    if !element.id.trim().is_empty() {
        details.push(format!("id={}", element.id.trim()));
    }
    if !element.href.trim().is_empty() {
        details.push(format!("href={}", element.href.trim()));
    }
    let detail_suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" [{}]", details.join(", "))
    };
    format!(
        "element {}: <{}> \"{}\" at ({},{}){}",
        element.index,
        element.tag.trim(),
        browser_loop_element_label(element),
        element.x,
        element.y,
        detail_suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn build_browser_session_view_flags_operator_handoff_states() {
        let waiting = build_browser_session_view(
            "session-1".to_string(),
            Some("conversation-1".to_string()),
            "demo".to_string(),
            None,
            None,
            SessionStatus::WaitingForOperator {
                question: "Log in manually".to_string(),
            },
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        assert_eq!(waiting.conversation_id.as_deref(), Some("conversation-1"));
        assert_eq!(waiting.status, "waiting_for_operator");
        assert!(waiting.can_claim);
        assert!(!waiting.can_complete);

        let claimed = build_browser_session_view(
            "session-1".to_string(),
            None,
            "demo".to_string(),
            None,
            None,
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
            None,
            "demo".to_string(),
            None,
            None,
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
    fn browser_ask_user_resolution_can_convert_validated_checkpoint_to_completion() {
        let value = serde_json::json!({
            "action": "complete",
            "summary": "Inbox rows extracted.",
            "message": "The visible inbox rows are Google, Manus, and OpenAI."
        });

        let resolution = browser_ask_user_resolution_from_value(&value, "Should I keep going?");

        assert_eq!(
            resolution,
            BrowserAskUserResolution::Complete {
                summary: "Inbox rows extracted.".to_string(),
                message: "The visible inbox rows are Google, Manus, and OpenAI.".to_string(),
            }
        );
    }

    #[test]
    fn browser_ask_user_resolution_preserves_true_user_checkpoints() {
        let value = serde_json::json!({
            "action": "ask_user",
            "requires_human": true,
            "reason": "The page requires a user choice."
        });

        let resolution =
            browser_ask_user_resolution_from_value(&value, "Which account should I use?");

        assert_eq!(resolution, BrowserAskUserResolution::AskUser);
    }

    #[test]
    fn browser_ask_user_resolution_requires_typed_human_need() {
        let value = serde_json::json!({
            "action": "ask_user",
            "requires_human": false,
            "reason": "The page has not produced the requested evidence yet."
        });

        let resolution = browser_ask_user_resolution_from_value(&value, "Should I keep going?");

        match resolution {
            BrowserAskUserResolution::Continue { guidance } => {
                assert!(guidance.contains("Continue browser work"));
            }
            other => panic!("ambiguous ask_user should continue, got {other:?}"),
        }
    }

    #[test]
    fn browser_ask_user_resolution_can_request_more_browser_work() {
        let value = serde_json::json!({
            "action": "continue",
            "guidance": "Read the visible rows and answer from them."
        });

        let resolution = browser_ask_user_resolution_from_value(&value, "Is this enough?");

        assert_eq!(
            resolution,
            BrowserAskUserResolution::Continue {
                guidance: "Read the visible rows and answer from them.".to_string(),
            }
        );
    }

    #[test]
    fn browser_ask_user_resolution_preserves_checkpoint_when_verdict_unavailable() {
        let resolution =
            browser_ask_user_resolution_when_verdict_unavailable("The live page needs help.");

        assert_eq!(resolution, BrowserAskUserResolution::AskUser);
    }

    #[test]
    fn browser_access_blocker_resolution_requires_classifier_block_flag() {
        let blocked = serde_json::json!({
            "blocked": true,
            "reason": "The page requires account access before the requested inbox content is visible."
        });
        let clear = serde_json::json!({
            "blocked": false,
            "reason": "The requested content is visible."
        });

        assert_eq!(
            browser_access_blocker_reason_from_value(&blocked).as_deref(),
            Some("The page requires account access before the requested inbox content is visible.")
        );
        assert!(browser_access_blocker_reason_from_value(&clear).is_none());
    }

    #[test]
    fn browser_access_blocker_classifier_is_gated_by_structural_sensitive_controls() {
        let inbox = PageContent {
            title: "Inbox".to_string(),
            url: "https://mail.example.test/inbox".to_string(),
            body_text: "Suno Your receipt from Suno\nKrea Welcome to Krea".to_string(),
            elements: vec![PageElement {
                index: 1,
                tag: "button".to_string(),
                r#type: "button".to_string(),
                text: "Refresh".to_string(),
                name: String::new(),
                id: String::new(),
                href: String::new(),
                x: 10,
                y: 10,
            }],
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };
        let sensitive_form = PageContent {
            elements: vec![PageElement {
                index: 2,
                tag: "input".to_string(),
                r#type: "password".to_string(),
                text: String::new(),
                name: String::new(),
                id: String::new(),
                href: String::new(),
                x: 10,
                y: 40,
            }],
            ..inbox.clone()
        };

        assert!(!browser_snapshot_has_structural_access_gate(&inbox));
        assert!(browser_snapshot_has_structural_access_gate(&sensitive_form));
    }

    #[test]
    fn browser_done_local_validation_accepts_page_grounded_information() {
        let content = PageContent {
            title: "Inbox".to_string(),
            url: "https://mail.example.test/inbox".to_string(),
            body_text: "Suno Your receipt from Suno\nKrea Welcome to Krea".to_string(),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };
        let action = serde_json::json!({
            "action": "done",
            "outcome_kind": "information_delivery",
            "evidence": "Suno Your receipt from Suno"
        });

        let validation = browser_done_answer_local_validation(
            &action,
            &content,
            false,
            "Latest emails found",
            "Suno sent Your receipt from Suno. Krea sent Welcome to Krea.",
        );

        assert_eq!(
            validation,
            BrowserDoneAnswerValidation::Complete {
                summary: "Latest emails found".to_string(),
                message: "Suno sent Your receipt from Suno. Krea sent Welcome to Krea.".to_string(),
            }
        );
    }

    #[test]
    fn browser_done_local_validation_hands_back_ungrounded_action_effect() {
        let content = PageContent {
            title: "Compose".to_string(),
            url: "https://mail.example.test/compose".to_string(),
            body_text: "Compose\nTo\nSubject\nBody\nSend".to_string(),
            elements: vec![PageElement {
                index: 7,
                tag: "button".to_string(),
                r#type: "button".to_string(),
                text: "Send".to_string(),
                name: String::new(),
                id: String::new(),
                href: String::new(),
                x: 100,
                y: 100,
            }],
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };
        let action = serde_json::json!({
            "action": "done",
            "outcome_kind": "action_effect",
            "evidence": "The requested action completed."
        });

        let validation = browser_done_answer_local_validation(
            &action,
            &content,
            false,
            "Done",
            "The email has been sent.",
        );

        match validation {
            BrowserDoneAnswerValidation::HandBack { question } => {
                assert!(question.contains("could not confirm"));
            }
            other => panic!("ungrounded action-effect completion should hand back, got {other:?}"),
        }
    }

    #[test]
    fn browser_done_local_validation_does_not_treat_page_change_as_action_effect_proof() {
        let content = PageContent {
            title: "Result page".to_string(),
            url: "https://app.example.test/items".to_string(),
            body_text: "The visible page changed after the last control action.".to_string(),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };
        let action = serde_json::json!({
            "action": "done",
            "outcome_kind": "action_effect",
            "evidence": "The page changed after the final browser interaction."
        });

        let validation = browser_done_answer_local_validation(
            &action,
            &content,
            true,
            "Done",
            "The requested action completed.",
        );

        match validation {
            BrowserDoneAnswerValidation::HandBack { question } => {
                assert!(question.contains("could not confirm"));
            }
            other => panic!("page change alone should not confirm action effect, got {other:?}"),
        }
    }

    #[test]
    fn blank_browser_snapshot_is_not_classified_as_access_blocker() {
        let content = PageContent {
            title: String::new(),
            url: "about:blank".to_string(),
            body_text: String::new(),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };

        assert!(!browser_snapshot_has_visible_state(&content));
    }

    #[test]
    fn superseded_failed_browser_session_candidate_requires_same_conversation_and_profile() {
        let failed = SessionStatus::Failed("Loading gate blocked the browser task".to_string());
        let completed = SessionStatus::Completed {
            summary: "Inbox loaded".to_string(),
        };

        assert!(failed_browser_session_is_superseded_candidate(
            Some("conversation-1"),
            Some("profile-1"),
            &failed,
            Some("conversation-1"),
            Some("profile-1"),
        ));
        assert!(!failed_browser_session_is_superseded_candidate(
            Some("conversation-1"),
            Some("profile-1"),
            &failed,
            Some("conversation-2"),
            Some("profile-1"),
        ));
        assert!(!failed_browser_session_is_superseded_candidate(
            Some("conversation-1"),
            Some("profile-1"),
            &failed,
            Some("conversation-1"),
            Some("profile-2"),
        ));
        assert!(!failed_browser_session_is_superseded_candidate(
            Some("conversation-1"),
            Some("profile-1"),
            &completed,
            Some("conversation-1"),
            Some("profile-1"),
        ));
    }

    #[test]
    fn browser_done_finalization_always_closes_completed_browser_tasks() {
        let close = serde_json::json!({
            "keep_open": false,
            "reason": "The answer was delivered in chat."
        });
        let keep_open = serde_json::json!({
            "keep_open": true,
            "reason": "The requested outcome is the live page state."
        });
        let missing = serde_json::json!({});

        assert_eq!(
            browser_done_finalization_from_value(&close),
            BrowserDoneFinalization::Close
        );
        assert_eq!(
            browser_done_finalization_from_value(&keep_open),
            BrowserDoneFinalization::Close
        );
        assert_eq!(
            browser_done_finalization_from_value(&missing),
            BrowserDoneFinalization::Close
        );
    }

    #[test]
    fn browser_session_list_hides_failed_attempt_superseded_by_newer_same_profile_result() {
        let failed = build_browser_session_view(
            "failed".to_string(),
            Some("conversation-1".to_string()),
            "Try browser task".to_string(),
            Some("profile-1".to_string()),
            Some("alex".to_string()),
            SessionStatus::Failed("Blocked".to_string()),
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        let completed = build_browser_session_view(
            "completed".to_string(),
            Some("conversation-1".to_string()),
            "Retry browser task".to_string(),
            Some("profile-1".to_string()),
            Some("alex".to_string()),
            SessionStatus::Completed {
                summary: "Done".to_string(),
            },
            "2026-01-01T00:02:00Z".to_string(),
            "2026-01-01T00:03:00Z".to_string(),
            None,
        );
        let different_profile_failed = build_browser_session_view(
            "other-profile-failed".to_string(),
            Some("conversation-1".to_string()),
            "Different profile task".to_string(),
            Some("profile-2".to_string()),
            Some("other".to_string()),
            SessionStatus::Failed("Blocked".to_string()),
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );

        let visible = browser_session_views_without_superseded_failures(vec![
            failed,
            completed,
            different_profile_failed,
        ]);
        let ids = visible
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["completed", "other-profile-failed"]);
    }

    #[test]
    fn browser_session_listing_keeps_only_live_or_claimable_sessions() {
        let active = build_browser_session_view(
            "active".to_string(),
            Some("conversation-1".to_string()),
            "Active task".to_string(),
            None,
            None,
            SessionStatus::Active,
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        let awaiting_chat = build_browser_session_view(
            "awaiting-chat".to_string(),
            Some("conversation-1".to_string()),
            "Needs chat input".to_string(),
            None,
            None,
            SessionStatus::AwaitingChatInput {
                question: "Choose next step".to_string(),
            },
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        let completed = build_browser_session_view(
            "completed".to_string(),
            Some("conversation-1".to_string()),
            "Done task".to_string(),
            None,
            None,
            SessionStatus::Completed {
                summary: "Done".to_string(),
            },
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );
        let failed = build_browser_session_view(
            "failed".to_string(),
            Some("conversation-1".to_string()),
            "Failed task".to_string(),
            None,
            None,
            SessionStatus::Failed("Blocked".to_string()),
            "2026-01-01T00:00:00Z".to_string(),
            "2026-01-01T00:01:00Z".to_string(),
            None,
        );

        assert!(browser_session_view_is_live_listing(&active));
        assert!(browser_session_view_is_live_listing(&awaiting_chat));
        assert!(!browser_session_view_is_live_listing(&completed));
        assert!(!browser_session_view_is_live_listing(&failed));
    }

    #[test]
    fn restore_preserves_live_handoff_sessions_as_awaiting_resume() {
        let persisted = PersistedBrowserSession {
            id: "session-1".to_string(),
            status: "operator_claimed".to_string(),
            task_description: "demo".to_string(),
            channel: "web".to_string(),
            chat_id: Some("conversation-1".to_string()),
            profile_id: None,
            profile_name: None,
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
        assert!(session_counts_against_live_limit(
            &SessionStatus::AwaitingChatInput {
                question: "Choose the next section".to_string(),
            }
        ));
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
        assert!(!session_should_be_cleaned_up(
            &SessionStatus::AwaitingChatInput {
                question: "Choose the next section".to_string(),
            }
        ));
    }

    #[test]
    fn terminal_browser_session_records_expire_after_evidence_retention() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 20, 0).single().unwrap();
        let recent_terminal = (now
            - chrono::Duration::seconds(TERMINAL_SESSION_EVIDENCE_RETENTION_SECS as i64 - 1))
        .to_rfc3339();
        let expired_terminal = (now
            - chrono::Duration::seconds(TERMINAL_SESSION_EVIDENCE_RETENTION_SECS as i64))
        .to_rfc3339();

        assert!(!browser_session_record_should_be_cleaned_up(
            &SessionStatus::Completed {
                summary: "Done".to_string(),
            },
            &recent_terminal,
            &now,
        ));
        assert!(browser_session_record_should_be_cleaned_up(
            &SessionStatus::Completed {
                summary: "Done".to_string(),
            },
            &expired_terminal,
            &now,
        ));
        assert!(browser_session_record_should_be_cleaned_up(
            &SessionStatus::Failed("Blocked".to_string()),
            &expired_terminal,
            &now,
        ));
        assert!(browser_session_record_should_be_cleaned_up(
            &SessionStatus::Interrupted {
                reason: "App restarted".to_string(),
            },
            &recent_terminal,
            &now,
        ));
        assert!(!browser_session_record_should_be_cleaned_up(
            &SessionStatus::Active,
            &expired_terminal,
            &now,
        ));
    }

    #[test]
    fn browser_done_outcome_marks_session_completed_and_closes_sidecar() {
        let (status, close_sidecar) = browser_loop_success_status(BrowserLoopOutcome::Completed {
            summary: "Inbox scan completed".to_string(),
        });

        assert!(matches!(status, SessionStatus::Completed { .. }));
        assert!(close_sidecar);

        let (status, close_sidecar) = browser_loop_success_status(BrowserLoopOutcome::Ready {
            summary: "Browser is ready for follow-up".to_string(),
        });

        assert!(matches!(status, SessionStatus::Ready { .. }));
        assert!(!close_sidecar);
    }

    #[test]
    fn ready_sessions_expire_after_five_minutes_idle() {
        let fresh_updated_at = (Utc::now() - chrono::Duration::minutes(4)).to_rfc3339();
        let stale_updated_at = (Utc::now() - chrono::Duration::minutes(6)).to_rfc3339();

        assert!(!ready_session_is_expired(&test_session(
            "fresh-ready",
            "",
            Some("conversation-1"),
            SessionStatus::Ready {
                summary: "Ready".to_string(),
            },
            &fresh_updated_at,
        )));
        assert!(ready_session_is_expired(&test_session(
            "stale-ready",
            "",
            Some("conversation-1"),
            SessionStatus::Ready {
                summary: "Ready".to_string(),
            },
            &stale_updated_at,
        )));
    }

    #[tokio::test]
    async fn maintenance_cleanup_removes_expired_ready_sessions_without_request_path() {
        let manager = test_manager();
        let fresh_updated_at = (Utc::now() - chrono::Duration::minutes(4)).to_rfc3339();
        let stale_updated_at = (Utc::now() - chrono::Duration::minutes(6)).to_rfc3339();
        manager.sessions.insert(
            "fresh-ready".to_string(),
            test_session(
                "fresh-ready",
                "",
                Some("conversation-1"),
                SessionStatus::Ready {
                    summary: "Ready".to_string(),
                },
                &fresh_updated_at,
            ),
        );
        manager.sessions.insert(
            "stale-ready".to_string(),
            test_session(
                "stale-ready",
                "",
                Some("conversation-1"),
                SessionStatus::Ready {
                    summary: "Ready".to_string(),
                },
                &stale_updated_at,
            ),
        );

        manager.cleanup_stale_or_unreachable_sessions().await;

        assert!(manager.sessions.contains_key("fresh-ready"));
        assert!(!manager.sessions.contains_key("stale-ready"));
    }

    #[tokio::test]
    async fn maintenance_cleanup_removes_interrupted_and_awaiting_resume_sessions() {
        let manager = test_manager();
        let updated_at = Utc::now().to_rfc3339();
        manager.sessions.insert(
            "active".to_string(),
            test_session(
                "active",
                "",
                Some("conversation-1"),
                SessionStatus::Active,
                &updated_at,
            ),
        );
        manager.sessions.insert(
            "interrupted".to_string(),
            test_session(
                "interrupted",
                "",
                Some("conversation-1"),
                SessionStatus::Interrupted {
                    reason: "App restarted".to_string(),
                },
                &updated_at,
            ),
        );
        manager.sessions.insert(
            "awaiting-resume".to_string(),
            test_session(
                "awaiting-resume",
                "",
                Some("conversation-1"),
                SessionStatus::AwaitingResume {
                    question: "Restart the browser task".to_string(),
                },
                &updated_at,
            ),
        );

        manager.cleanup_stale_or_unreachable_sessions().await;

        assert!(manager.sessions.contains_key("active"));
        assert!(!manager.sessions.contains_key("interrupted"));
        assert!(!manager.sessions.contains_key("awaiting-resume"));
    }

    #[test]
    fn public_browser_task_description_removes_internal_spine_planner_text() {
        let raw = "Original user request: spine primitive `browse` Planner browser task: Open https://en.wikipedia.org/wiki/OpenAI. Go to https://en.wikipedia.org/wiki/OpenAI, wait for the page to load.";

        let cleaned = public_browser_task_description(raw);

        assert_eq!(
            cleaned,
            "Open https://en.wikipedia.org/wiki/OpenAI. Go to https://en.wikipedia.org/wiki/OpenAI, wait for the page to load."
        );
        assert!(!cleaned.contains("spine primitive"));
        assert!(!cleaned.contains("Planner browser task"));
        assert!(!cleaned.contains("Original user request"));
    }

    #[test]
    fn browser_chat_follow_up_task_description_keeps_checkpoint_context() {
        let task = browser_chat_follow_up_task_description(
            "Open the article.",
            "Should I inspect History or Products?",
            "History",
        );

        assert!(task.contains("Previous browser task: Open the article."));
        assert!(task.contains("Pending browser question: Should I inspect History or Products?"));
        assert!(task.contains("User response: History"));
    }

    #[test]
    fn browser_loop_element_line_does_not_look_like_css_attribute_selector() {
        let element = PageElement {
            index: 12,
            tag: "a".to_string(),
            r#type: String::new(),
            text: "OpenAI".to_string(),
            name: String::new(),
            id: String::new(),
            href: "/wiki/OpenAI".to_string(),
            x: 240,
            y: 140,
        };

        let line = format_browser_loop_element(&element);

        assert!(line.contains("element 12"));
        assert!(line.contains("OpenAI"));
        assert!(!line.contains("[12]"));
    }

    #[test]
    fn browser_navigation_guard_rejects_reload_of_current_page() {
        let mut progress = BrowserNavigationProgress::default();
        progress.observe_page_url("https://www.wikipedia.org/");

        let decision =
            progress.decide_navigation("https://www.wikipedia.org/", "https://www.wikipedia.org/");

        assert!(matches!(decision, BrowserNavigationDecision::Reject { .. }));
    }

    #[test]
    fn browser_navigation_guard_rejects_return_to_earlier_page_after_progress() {
        let mut progress = BrowserNavigationProgress::default();
        progress.observe_page_url("https://www.wikipedia.org/");
        progress.observe_page_url("https://en.wikipedia.org/wiki/OpenAI");

        let decision = progress.decide_navigation(
            "https://www.wikipedia.org/",
            "https://en.wikipedia.org/wiki/OpenAI",
        );

        assert!(matches!(decision, BrowserNavigationDecision::Reject { .. }));
    }

    #[test]
    fn browser_navigation_guard_allows_new_navigation_target() {
        let mut progress = BrowserNavigationProgress::default();
        progress.observe_page_url("https://www.wikipedia.org/");

        let decision = progress.decide_navigation(
            "https://en.wikipedia.org/wiki/OpenAI",
            "https://www.wikipedia.org/",
        );

        assert_eq!(decision, BrowserNavigationDecision::Allow);
    }

    #[test]
    fn browser_task_site_constraint_blocks_search_engine_fallback() {
        let constraint = browser_task_site_constraint(
            "Open https://www.wikipedia.org, search for \"OpenAI\", go to the article, then stop and ask me what section to inspect.",
        )
        .expect("explicit single-site task should create a site constraint");

        assert!(constraint.allows_navigation(
            "https://en.wikipedia.org/wiki/OpenAI",
            "https://www.wikipedia.org/"
        ));
        assert!(!constraint.allows_navigation(
            "https://www.google.com/search?q=OpenAI",
            "https://www.wikipedia.org/"
        ));
        assert!(!constraint.allows_navigation(
            "https://duckduckgo.com/?q=OpenAI+wikipedia",
            "https://www.wikipedia.org/"
        ));
    }

    #[test]
    fn browser_task_site_constraint_allows_relative_navigation_on_requested_site() {
        let constraint =
            browser_task_site_constraint("Open https://www.wikipedia.org and inspect OpenAI")
                .expect("explicit single-site task should create a site constraint");

        assert!(constraint.allows_navigation("/wiki/OpenAI", "https://www.wikipedia.org/"));
    }

    #[test]
    fn browser_task_site_constraint_is_not_created_for_multi_site_tasks() {
        let constraint = browser_task_site_constraint(
            "Open https://www.wikipedia.org and compare it with https://openai.com.",
        );

        assert!(constraint.is_none());
    }

    #[test]
    fn browser_stall_tracker_counts_only_repeated_same_stall() {
        let mut tracker = BrowserLoopStallTracker::default();

        assert_eq!(tracker.record("navigate:a"), 1);
        assert_eq!(tracker.record("navigate:a"), 2);
        assert_eq!(tracker.record("navigate:b"), 1);
        tracker.reset();
        assert_eq!(tracker.record("navigate:b"), 1);
    }

    #[test]
    fn browser_progress_tracker_counts_repeated_actions_only_on_unchanged_page_state() {
        let action = serde_json::json!({
            "action": "click",
            "element_index": 12
        });
        let changed_action = serde_json::json!({
            "action": "click",
            "element_index": 13
        });
        let content = PageContent {
            title: "Inbox".to_string(),
            url: "https://example.test/inbox".to_string(),
            body_text: "One visible task row".to_string(),
            elements: vec![PageElement {
                index: 12,
                tag: "button".to_string(),
                r#type: "button".to_string(),
                text: "Open".to_string(),
                name: String::new(),
                id: "open-row".to_string(),
                href: String::new(),
                x: 40,
                y: 60,
            }],
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };
        let changed_content = PageContent {
            body_text: "One visible task row\nDetails are now visible".to_string(),
            ..content.clone()
        };
        let mut tracker = BrowserLoopProgressTracker::default();

        assert_eq!(tracker.record_action(&content, &action), 1);
        assert_eq!(tracker.record_action(&content, &action), 2);
        assert_eq!(tracker.record_action(&changed_content, &action), 1);
        assert_eq!(tracker.record_action(&changed_content, &changed_action), 1);
    }

    #[test]
    fn browser_interaction_progress_counts_only_expected_unchanged_states() {
        let mut no_progress_count = 0;

        let changed = browser_observe_interaction_progress(
            BrowserInteractionProgressProbe::new("before".to_string(), true),
            "before",
            &mut no_progress_count,
        );
        assert!(!changed);
        assert_eq!(no_progress_count, 1);

        let changed = browser_observe_interaction_progress(
            BrowserInteractionProgressProbe::new("before".to_string(), true),
            "after",
            &mut no_progress_count,
        );
        assert!(changed);
        assert_eq!(no_progress_count, 0);

        let changed = browser_observe_interaction_progress(
            BrowserInteractionProgressProbe::new("after".to_string(), false),
            "after",
            &mut no_progress_count,
        );
        assert!(!changed);
        assert_eq!(no_progress_count, 0);
    }

    #[test]
    fn stalled_browser_question_reports_current_page_status() {
        let content = PageContent {
            title: "Current workflow".to_string(),
            url: "https://app.example.test/workflow".to_string(),
            body_text: "A visible panel is still open and waiting for the next site operation."
                .to_string(),
            elements: vec![PageElement {
                index: 4,
                tag: "button".to_string(),
                r#type: "button".to_string(),
                text: "Continue".to_string(),
                name: String::new(),
                id: String::new(),
                href: String::new(),
                x: 10,
                y: 20,
            }],
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };

        let question =
            stalled_browser_question(&content, "recent browser controls did not change the page");

        assert!(question.contains("automation paused"));
        assert!(question.contains("Current workflow"));
        assert!(question.contains("https://app.example.test/workflow"));
        assert!(question.contains("Visible text starts"));
        assert!(question.contains("1 visible interactive elements"));
        assert!(!question.contains("What should I do next"));
    }

    #[test]
    fn browser_snapshot_completion_requires_explicit_evidenced_answer() {
        let missing_action = serde_json::json!({
            "summary": "The page looks loaded."
        });
        let vague_complete = serde_json::json!({
            "action": "complete"
        });
        let answered = serde_json::json!({
            "action": "complete",
            "summary": "Latest visible rows found.",
            "message": "The latest visible rows are Alice: Budget review, Bob: Launch plan."
        });

        assert_eq!(
            browser_snapshot_completion_from_value(&missing_action, "Need more visible evidence."),
            BrowserDoneAnswerValidation::Continue {
                guidance: "Need more visible evidence.".to_string(),
            }
        );
        assert_eq!(
            browser_snapshot_completion_from_value(&vague_complete, "Need more visible evidence."),
            BrowserDoneAnswerValidation::Continue {
                guidance: "Need more visible evidence.".to_string(),
            }
        );
        assert_eq!(
            browser_snapshot_completion_from_value(&answered, "Need more visible evidence."),
            BrowserDoneAnswerValidation::Complete {
                summary: "Latest visible rows found.".to_string(),
                message: "The latest visible rows are Alice: Budget review, Bob: Launch plan."
                    .to_string(),
            }
        );
    }

    #[tokio::test]
    async fn starting_browser_work_removes_ready_profile_sessions_without_conversation() {
        let manager = test_manager();
        manager.sessions.insert(
            "profile-ready".to_string(),
            BrowserSession {
                profile_id: Some("profile-1".to_string()),
                status: SessionStatus::Ready {
                    summary: "Ready".to_string(),
                },
                ..test_session(
                    "profile-ready",
                    "",
                    None,
                    SessionStatus::Active,
                    "2026-01-01T00:00:00Z",
                )
            },
        );
        manager.sessions.insert(
            "conversation-ready".to_string(),
            BrowserSession {
                profile_id: Some("profile-1".to_string()),
                status: SessionStatus::Ready {
                    summary: "Ready".to_string(),
                },
                ..test_session(
                    "conversation-ready",
                    "",
                    Some("conversation-1"),
                    SessionStatus::Active,
                    "2026-01-01T00:00:00Z",
                )
            },
        );

        manager.close_ready_profile_login_sessions().await;

        assert!(!manager.sessions.contains_key("profile-ready"));
        assert!(manager.sessions.contains_key("conversation-ready"));
    }

    #[tokio::test]
    async fn complete_operator_handoff_can_return_control_to_chat() {
        let manager = test_manager();
        let (tx, rx) = oneshot::channel();
        manager.sessions.insert(
            "session-1".to_string(),
            BrowserSession {
                id: "session-1".to_string(),
                sidecar_session_id: String::new(),
                channel: "web".to_string(),
                conversation_id: Some("conversation-1".to_string()),
                profile_id: None,
                profile_name: None,
                task_description: "Inspect current page".to_string(),
                status: SessionStatus::OperatorClaimed {
                    question: "Continue?".to_string(),
                },
                action_history: Vec::new(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                loop_token: Some("loop-1".to_string()),
                operator_handoff_tx: Some(tx),
            },
        );

        let view = manager
            .complete_operator_handoff("session-1", "What changed", true)
            .await
            .expect("handoff completion should succeed");

        assert_eq!(view.status, "ready");
        assert_eq!(view.conversation_id.as_deref(), Some("conversation-1"));
        let outcome = rx.await.expect("browser loop should receive completion");
        assert_eq!(outcome.note, "What changed");
        assert!(outcome.resume_in_chat);
        let entry = manager
            .sessions
            .get("session-1")
            .expect("session should remain ready for chat reuse");
        assert!(entry.loop_token.is_none());
    }

    #[tokio::test]
    async fn waiting_operator_handoff_accepts_chat_response_without_live_claim() {
        let manager = test_manager();
        let (tx, rx) = oneshot::channel();
        manager.sessions.insert(
            "session-1".to_string(),
            BrowserSession {
                id: "session-1".to_string(),
                sidecar_session_id: String::new(),
                channel: "web".to_string(),
                conversation_id: Some("conversation-1".to_string()),
                profile_id: None,
                profile_name: None,
                task_description: "Inspect current page".to_string(),
                status: SessionStatus::WaitingForOperator {
                    question: "Choose the next section".to_string(),
                },
                action_history: Vec::new(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                loop_token: Some("loop-1".to_string()),
                operator_handoff_tx: Some(tx),
            },
        );

        let view = manager
            .complete_operator_handoff("session-1", "History section", false)
            .await
            .expect("chat response should resume waiting browser loop");

        assert_eq!(view.status, "active");
        let outcome = rx.await.expect("browser loop should receive chat reply");
        assert_eq!(outcome.note, "History section");
        assert!(!outcome.resume_in_chat);
    }

    #[tokio::test]
    async fn ask_user_checkpoint_returns_to_chat_without_live_handoff_wait() {
        let manager = test_manager();
        manager.sessions.insert(
            "session-1".to_string(),
            BrowserSession {
                id: "session-1".to_string(),
                sidecar_session_id: "sidecar-1".to_string(),
                channel: "web".to_string(),
                conversation_id: Some("conversation-1".to_string()),
                profile_id: None,
                profile_name: None,
                task_description: "Stop and ask which section to inspect.".to_string(),
                status: SessionStatus::Active,
                action_history: Vec::new(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                loop_token: Some("loop-1".to_string()),
                operator_handoff_tx: None,
            },
        );
        let integration = Arc::new(BrowserIntegration::new());
        let llm = crate::core::model::llm::LlmClient::new(
            &crate::core::model::llm::LlmProvider::default(),
        )
        .expect("test llm client");
        let notifications = Arc::new(std::sync::Mutex::new(Vec::new()));
        let notify_sink = notifications.clone();
        let notify: Arc<dyn Fn(BrowserSessionNotification) + Send + Sync> =
            Arc::new(move |notification| {
                notify_sink
                    .lock()
                    .expect("notification lock")
                    .push(notification);
            });
        let ctx = BrowserLoopContext {
            sidecar_id: "sidecar-1",
            task: "Stop and ask which section to inspect.",
            sessions: &manager.sessions,
            integration: &integration,
            llm: &llm,
            notify: &notify,
            storage: None,
        };
        let content = PageContent {
            title: "OpenAI - Wikipedia".to_string(),
            url: "https://en.wikipedia.org/wiki/OpenAI".to_string(),
            body_text: "OpenAI article".to_string(),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            downloads: Vec::new(),
            download_dir: None,
        };
        let mut history = Vec::new();

        let result = tokio::time::timeout(
            tokio::time::Duration::from_millis(50),
            wait_for_browser_operator_input(
                "session-1",
                &ctx,
                "Should I inspect the History section or the Products section?",
                &content,
                &mut history,
            ),
        )
        .await
        .expect("chat checkpoint should not wait for live handoff")
        .expect("chat checkpoint should succeed");

        match result {
            BrowserOperatorWaitResult::ReturnToChat {
                outcome: BrowserLoopOutcome::NeedsChatInput { question },
            } => {
                assert!(question.contains("History section"));
                assert!(question.contains("Products section"));
            }
            BrowserOperatorWaitResult::ReturnToChat { .. } => {
                panic!("explicit chat checkpoint should need chat input")
            }
            BrowserOperatorWaitResult::Continue { .. } => {
                panic!("explicit chat checkpoint should return to chat")
            }
        }
        let entry = manager
            .sessions
            .get("session-1")
            .expect("session should remain for resume");
        assert!(matches!(
            entry.status,
            SessionStatus::AwaitingChatInput { .. }
        ));
        assert!(entry.operator_handoff_tx.is_none());
        drop(entry);
        let view = manager
            .describe_session("session-1")
            .await
            .expect("session view");
        assert_eq!(view.status, "awaiting_chat_input");
        assert_eq!(
            view.question.as_deref(),
            Some("Should I inspect the History section or the Products section?")
        );
        assert!(notifications
            .lock()
            .expect("notification lock")
            .iter()
            .any(|notification| notification.kind == BrowserSessionNotificationKind::NeedsInput));
    }

    fn test_manager() -> BrowserSessionManager {
        BrowserSessionManager {
            sessions: Arc::new(DashMap::new()),
            page_snapshots: Arc::new(DashMap::new()),
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
            profile_id: None,
            profile_name: None,
            task_description: "demo task".to_string(),
            status,
            action_history: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated_at.to_string(),
            loop_token: None,
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
            .latest_managed_live_session_for_conversation(Some("conversation-1"), None)
            .await
            .expect("expected reusable live browser session");

        assert_eq!(session_id, "managed-new");
    }
}
