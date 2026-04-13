//! Browser session manager for LLM-driven browser automation.
//!
//! Sessions are long-running background tasks that control the Playwright bridge,
//! wait for `ask_user` responses, and keep enough durable state to survive restarts.

use anyhow::Result;
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::integrations::browser::BrowserIntegration;

const MAX_ITERATIONS: u32 = 30;
const MAX_PERSISTED_ACTION_HISTORY: usize = 80;
const INTERRUPTED_BROWSER_SESSION_REASON: &str =
    "Browser session was interrupted by an app restart before it could finish.";

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
    WaitingForUser {
        #[allow(dead_code)]
        screenshot: Vec<u8>,
        question: String,
    },
    AwaitingResume {
        question: String,
    },
    Interrupted {
        reason: String,
    },
    Completed {
        summary: String,
    },
    Failed(String),
}

pub struct BrowserSession {
    pub id: String,
    pub _sidecar_session_id: String,
    pub _channel: String,
    pub chat_id: String,
    pub task_description: String,
    pub status: SessionStatus,
    pub action_history: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub user_response_tx: Option<oneshot::Sender<String>>,
}

#[derive(Debug, Clone)]
pub struct WaitingSessionSummary {
    pub id: String,
    pub task_description: String,
    pub question: String,
    pub updated_at: String,
    pub can_accept_response: bool,
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
        chat_id: &str,
        llm_client: super::llm::LlmClient,
        notify_fn: Arc<dyn Fn(String, Option<Vec<u8>>) + Send + Sync>,
    ) -> Result<String> {
        let sidecar_id = self.integration.create_session().await?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let created_at = now_rfc3339();

        self.sessions.insert(
            session_id.clone(),
            BrowserSession {
                id: session_id.clone(),
                _sidecar_session_id: sidecar_id.clone(),
                _channel: channel.to_string(),
                chat_id: chat_id.to_string(),
                task_description: task.to_string(),
                status: SessionStatus::Active,
                action_history: Vec::new(),
                created_at: created_at.clone(),
                updated_at: created_at,
                user_response_tx: None,
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
                    Err(error) => SessionStatus::Failed(error.to_string()),
                };
                entry.user_response_tx = None;
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

    pub async fn provide_user_response(&self, session_id: &str, response: &str) -> bool {
        let snapshot = if let Some(mut entry) = self.sessions.get_mut(session_id) {
            if let Some(tx) = entry.user_response_tx.take() {
                let _ = tx.send(response.to_string());
                entry.status = SessionStatus::Active;
                entry.updated_at = now_rfc3339();
                Some(PersistedBrowserSession::from_session(&entry))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(snapshot) = snapshot {
            persist_browser_session(self.storage.as_ref(), &snapshot).await;
            return true;
        }
        false
    }

    pub fn waiting_sessions_for_chat(&self, chat_id: &str) -> Vec<WaitingSessionSummary> {
        if chat_id.trim().is_empty() {
            return Vec::new();
        }
        let mut waiting = self
            .sessions
            .iter()
            .filter_map(|entry| {
                if entry.chat_id != chat_id {
                    return None;
                }
                match &entry.status {
                    SessionStatus::WaitingForUser { question, .. } => Some(WaitingSessionSummary {
                        id: entry.id.clone(),
                        task_description: entry.task_description.clone(),
                        question: question.clone(),
                        updated_at: entry.updated_at.clone(),
                        can_accept_response: true,
                    }),
                    SessionStatus::AwaitingResume { question } => Some(WaitingSessionSummary {
                        id: entry.id.clone(),
                        task_description: entry.task_description.clone(),
                        question: question.clone(),
                        updated_at: entry.updated_at.clone(),
                        can_accept_response: false,
                    }),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        waiting.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        waiting
    }

    #[cfg(test)]
    pub fn get_waiting_session(&self, chat_id: &str) -> Option<(String, Vec<u8>, String)> {
        self.sessions.iter().find_map(|entry| {
            if entry.chat_id != chat_id {
                return None;
            }
            match &entry.status {
                SessionStatus::WaitingForUser {
                    screenshot,
                    question,
                } => Some((entry.id.clone(), screenshot.clone(), question.clone())),
                _ => None,
            }
        })
    }

    pub fn get_status(&self, session_id: &str) -> Option<SessionStatus> {
        self.sessions.get(session_id).map(|s| s.status.clone())
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
                        | SessionStatus::WaitingForUser { .. }
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

async fn set_waiting_for_user(
    sessions: &Arc<DashMap<String, BrowserSession>>,
    storage: Option<&crate::storage::Storage>,
    session_id: &str,
    screenshot: Vec<u8>,
    question: String,
    tx: oneshot::Sender<String>,
) {
    let snapshot = if let Some(mut entry) = sessions.get_mut(session_id) {
        entry.status = SessionStatus::WaitingForUser {
            screenshot,
            question,
        };
        entry.user_response_tx = Some(tx);
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
            channel: session._channel.clone(),
            chat_id: (!session.chat_id.trim().is_empty()).then(|| session.chat_id.clone()),
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
            "waiting_for_user" => (
                SessionStatus::AwaitingResume {
                    question: status_detail
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| {
                            "Browser session was waiting for your input before restart.".to_string()
                        }),
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
                _sidecar_session_id: String::new(),
                _channel: channel,
                chat_id: chat_id.unwrap_or_default(),
                task_description,
                status,
                action_history: trimmed_action_history(&action_history),
                created_at,
                updated_at: if changed { now_rfc3339() } else { updated_at },
                user_response_tx: None,
            },
            changed,
        )
    }
}

impl SessionStatus {
    fn kind(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::WaitingForUser { .. } => "waiting_for_user",
            Self::AwaitingResume { .. } => "awaiting_resume",
            Self::Interrupted { .. } => "interrupted",
            Self::Completed { .. } => "completed",
            Self::Failed(_) => "failed",
        }
    }

    fn detail(&self) -> Option<String> {
        match self {
            Self::Active => None,
            Self::WaitingForUser { question, .. } => Some(question.clone()),
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
    notify: &'a Arc<dyn Fn(String, Option<Vec<u8>>) + Send + Sync>,
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
         - {{\"action\":\"ask_user\",\"question\":\"...\"}}\n\
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
                        (ctx.notify)(message.to_string(), None);
                    }
                }
                history.push(format!("Step {}: Sent progress update", iteration + 1));
            }
            "ask_user" => {
                let question = action
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("I need your help to continue.");
                let screenshot = ctx
                    .integration
                    .screenshot(ctx.sidecar_id)
                    .await
                    .unwrap_or_default();
                let notify_screenshot = screenshot.clone();
                let (tx, rx) = oneshot::channel::<String>();
                set_waiting_for_user(
                    ctx.sessions,
                    ctx.storage.as_ref(),
                    session_id,
                    screenshot,
                    question.to_string(),
                    tx,
                )
                .await;
                (ctx.notify)(question.to_string(), Some(notify_screenshot));
                match tokio::time::timeout(tokio::time::Duration::from_secs(300), rx).await {
                    Ok(Ok(user_response)) => {
                        history.push(format!(
                            "Step {}: Asked user, got response ({} chars)",
                            iteration + 1,
                            user_response.len()
                        ));
                        history.push(format!("User replied: {}", user_response));
                    }
                    Ok(Err(_)) => return Err(anyhow::anyhow!("User response channel closed")),
                    Err(_) => return Err(anyhow::anyhow!("Timed out waiting for user response")),
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
                (ctx.notify)(message.to_string(), screenshot);
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
    (ctx.notify)(
        format!(
            "Browser session reached the maximum of {} steps.",
            MAX_ITERATIONS
        ),
        screenshot,
    );
    Ok(format!("Reached max iterations ({})", MAX_ITERATIONS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn waiting_session_lookup_requires_an_explicit_chat_id() {
        let now = now_rfc3339();
        let manager = BrowserSessionManager::new(None).await;
        manager.sessions.insert(
            "session-1".to_string(),
            BrowserSession {
                id: "session-1".to_string(),
                _sidecar_session_id: "sidecar-1".to_string(),
                _channel: "web".to_string(),
                chat_id: "conversation-1".to_string(),
                task_description: "demo".to_string(),
                status: SessionStatus::WaitingForUser {
                    screenshot: vec![1, 2, 3],
                    question: "Need help".to_string(),
                },
                action_history: Vec::new(),
                created_at: now.clone(),
                updated_at: now,
                user_response_tx: None,
            },
        );

        assert!(manager.get_waiting_session("").is_none());
        assert!(manager.get_waiting_session("other").is_none());
        let waiting = manager
            .get_waiting_session("conversation-1")
            .expect("matching chat id should resolve");
        assert_eq!(waiting.0, "session-1");
        assert_eq!(waiting.1, vec![1, 2, 3]);
        assert_eq!(waiting.2, "Need help");
    }

    #[test]
    fn restore_preserves_waiting_sessions_as_awaiting_resume() {
        let persisted = PersistedBrowserSession {
            id: "session-1".to_string(),
            status: "waiting_for_user".to_string(),
            task_description: "demo".to_string(),
            channel: "web".to_string(),
            chat_id: Some("conversation-1".to_string()),
            status_detail: Some("Please confirm".to_string()),
            action_history: vec!["Step 1: Navigated".to_string()],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:01:00Z".to_string(),
        };

        let (session, changed) = BrowserSession::restore_from_persisted(persisted);

        assert!(changed);
        match session.status {
            SessionStatus::AwaitingResume { question } => {
                assert!(question.contains("Please confirm"));
            }
            other => panic!("unexpected restored status: {:?}", other),
        }
    }
}
