//! Extension hook system - pre/post processing hooks for agent events

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

const HOOK_TIMEOUT_SECS: u64 = 5;
const HOOK_MAX_RETRY_SAME_EVENT: usize = 1;
const HOOK_RUN_HISTORY_LIMIT: usize = 512;

/// Hook trigger point
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HookTrigger {
    /// Before processing a message
    PreMessage,
    /// After generating a response
    PostMessage,
    /// Before executing an action
    PreAction,
    /// After executing an action
    PostAction,
    /// On learning consolidation
    OnConsolidate,
    /// On error
    OnError,
}

/// A registered hook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    pub id: String,
    pub name: String,
    /// Optional action this hook is attached to (for action-scoped automations)
    #[serde(default)]
    pub action_name: Option<String>,
    pub trigger: HookTrigger,
    /// The hook type - currently supports "webhook" (HTTP POST)
    pub hook_type: String,
    /// URL for webhook hooks
    pub url: Option<String>,
    /// Whether hook is active
    pub enabled: bool,
}

/// Hook execution context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub trigger: String,
    pub channel: String,
    pub message: Option<String>,
    pub response: Option<String>,
    pub action: Option<String>,
    pub timestamp: String,
}

/// Result of an attempted hook dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRunReport {
    pub id: String,
    pub event_id: String,
    pub hook_id: String,
    pub hook_name: String,
    pub trigger: String,
    pub channel: String,
    pub action: Option<String>,
    pub hook_type: String,
    pub target: Option<String>,
    pub status: String,
    pub attempts: usize,
    pub error: Option<String>,
    pub timestamp: String,
}

/// Hook manager
pub struct HookManager {
    hooks: Vec<Hook>,
    client: reqwest::Client,
    runs: Arc<RwLock<VecDeque<HookRunReport>>>,
}

impl HookManager {
    pub fn from_hooks(hooks: Vec<Hook>) -> Self {
        Self {
            hooks,
            client: reqwest::Client::new(),
            runs: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    pub fn add_hook(&mut self, hook: Hook) {
        self.hooks.push(hook);
    }

    pub fn remove_hook(&mut self, id: &str) {
        self.hooks.retain(|h| h.id != id);
    }

    pub fn list_hooks(&self) -> &[Hook] {
        &self.hooks
    }

    pub fn snapshot(&self) -> Vec<Hook> {
        self.hooks.clone()
    }

    pub async fn list_runs(&self, limit: usize) -> Vec<HookRunReport> {
        let capped = limit.clamp(1, 500);
        let runs = self.runs.read().await;
        runs.iter().rev().take(capped).cloned().collect()
    }

    fn hook_matches_action(hook: &Hook, action: Option<&str>) -> bool {
        match (hook.action_name.as_deref(), action) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(expected), Some(actual)) => expected.trim().eq_ignore_ascii_case(actual.trim()),
        }
    }

    async fn push_run(runs: &Arc<RwLock<VecDeque<HookRunReport>>>, report: HookRunReport) {
        let mut guard = runs.write().await;
        guard.push_back(report);
        while guard.len() > HOOK_RUN_HISTORY_LIMIT {
            guard.pop_front();
        }
    }

    async fn post_webhook_with_retry(
        client: reqwest::Client,
        url: String,
        context: HookContext,
    ) -> (usize, Option<String>) {
        let max_attempts = 1 + HOOK_MAX_RETRY_SAME_EVENT;
        let mut attempts = 0usize;
        let mut last_error: Option<String> = None;

        while attempts < max_attempts {
            attempts += 1;
            match client
                .post(&url)
                .json(&context)
                .timeout(std::time::Duration::from_secs(HOOK_TIMEOUT_SECS))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return (attempts, None),
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    let clipped = if body.chars().count() > 300 {
                        format!("{}...", body.chars().take(300).collect::<String>())
                    } else {
                        body
                    };
                    last_error = Some(format!("HTTP {} {}", status.as_u16(), clipped));
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                }
            }

            if attempts < max_attempts {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }

        (attempts, last_error)
    }

    /// Fire all hooks matching the given trigger
    pub async fn fire(&self, trigger: HookTrigger, context: HookContext) {
        let shared_event_id = context
            .event_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        for hook in &self.hooks {
            if hook.trigger != trigger || !hook.enabled {
                continue;
            }

            if !Self::hook_matches_action(hook, context.action.as_deref()) {
                continue;
            }

            let mut ctx = context.clone();
            ctx.event_id = Some(shared_event_id.clone());
            let hook_copy = hook.clone();
            let client = self.client.clone();
            let runs = self.runs.clone();

            // Contained execution: do not block request/action flow.
            tokio::spawn(async move {
                let hook_type = hook_copy.hook_type.trim().to_ascii_lowercase();
                let (attempts, error) = if hook_type == "webhook" {
                    if let Some(url) = hook_copy.url.clone() {
                        Self::post_webhook_with_retry(client, url, ctx.clone()).await
                    } else {
                        (0, Some("Webhook URL is missing".to_string()))
                    }
                } else {
                    (
                        0,
                        Some(format!("Unsupported hook type: {}", hook_copy.hook_type)),
                    )
                };

                let status = if error.is_some() { "failed" } else { "success" };
                if let Some(ref err) = error {
                    tracing::warn!(
                        "Hook run failed: hook_id={}, hook_name={}, event_id={}, error={}",
                        hook_copy.id,
                        hook_copy.name,
                        ctx.event_id.as_deref().unwrap_or_default(),
                        err
                    );
                }

                let report = HookRunReport {
                    id: uuid::Uuid::new_v4().to_string(),
                    event_id: ctx.event_id.clone().unwrap_or_default(),
                    hook_id: hook_copy.id.clone(),
                    hook_name: hook_copy.name.clone(),
                    trigger: ctx.trigger.clone(),
                    channel: ctx.channel.clone(),
                    action: ctx.action.clone(),
                    hook_type: hook_copy.hook_type.clone(),
                    target: hook_copy.url.clone(),
                    status: status.to_string(),
                    attempts,
                    error,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                Self::push_run(&runs, report).await;
            });
        }
    }
}
