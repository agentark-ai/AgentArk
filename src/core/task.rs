//! Task queue for autonomous execution

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Task approval policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskApproval {
    /// Execute immediately without approval
    Auto,
    /// Legacy mode retained for compatibility; normalized to explicit approval at runtime
    NotifyThenExecute { delay_seconds: u64 },
    /// Require explicit user approval
    RequireApproval,
}

/// Task status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    AwaitingApproval,
    Paused,
    InProgress,
    Completed,
    Failed { error: String },
    Cancelled,
}

/// A task for the agent to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub description: String,
    pub action: String,
    pub arguments: serde_json::Value,
    pub approval: TaskApproval,
    pub capabilities: Vec<String>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub cron: Option<String>,
    pub result: Option<String>,
    pub proof_id: Option<Uuid>,
    /// User or LLM-assigned priority (0.0-1.0)
    pub priority: Option<f32>,
    /// Computed urgency based on deadline proximity (0.0-1.0)
    pub urgency: Option<f32>,
    /// LLM-scored importance (0.0-1.0)
    pub importance: Option<f32>,
    /// Eisenhower quadrant: 1=urgent+important, 2=important, 3=urgent, 4=neither
    pub eisenhower_quadrant: Option<u8>,
}

fn strip_automation_meta(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (key, inner) in map {
                if key == "_automation" || key == "_approval" {
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

fn task_topic_signature(arguments: &serde_json::Value, description: &str) -> String {
    let cleaned = strip_automation_meta(arguments);
    let preferred = ["query", "url", "topic", "target", "app_id", "goal_id", "id"]
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

fn task_report_channel(arguments: &serde_json::Value) -> String {
    strip_automation_meta(arguments)
        .get("report_to")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn task_semantic_signature(task: &Task) -> String {
    let cleaned = strip_automation_meta(&task.arguments);
    format!(
        "{}|{}|{}",
        task.action.trim().to_ascii_lowercase(),
        task_report_channel(&cleaned),
        task_topic_signature(&cleaned, &task.description)
    )
}

pub fn normalized_task_approval(approval: &TaskApproval) -> TaskApproval {
    match approval {
        TaskApproval::Auto => TaskApproval::Auto,
        TaskApproval::RequireApproval | TaskApproval::NotifyThenExecute { .. } => {
            TaskApproval::RequireApproval
        }
    }
}

pub fn task_requires_explicit_approval(approval: &TaskApproval) -> bool {
    matches!(
        normalized_task_approval(approval),
        TaskApproval::RequireApproval
    )
}

pub fn status_for_task_approval(approval: &TaskApproval) -> TaskStatus {
    if task_requires_explicit_approval(approval) {
        TaskStatus::AwaitingApproval
    } else {
        TaskStatus::Pending
    }
}

pub fn task_request_signature_from_fields(
    action_name: &str,
    description: &str,
    arguments: &serde_json::Value,
    cron_expr: Option<&str>,
    at_time: Option<&str>,
) -> String {
    let cleaned = strip_automation_meta(arguments);
    let report_to = cleaned
        .get("report_to")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let schedule = if let Some(cron) = cron_expr {
        format!("cron:{}", normalize_signature_text(cron))
    } else if let Some(at) = at_time {
        format!("at:{}", normalize_signature_text(at))
    } else {
        "once".to_string()
    };
    format!(
        "{}|{}|{}|{}",
        action_name.trim().to_ascii_lowercase(),
        report_to,
        schedule,
        task_topic_signature(&cleaned, description)
    )
}

pub fn tasks_are_semantically_similar(existing: &Task, candidate: &Task) -> bool {
    if !existing.action.eq_ignore_ascii_case(&candidate.action) {
        return false;
    }
    if task_report_channel(&existing.arguments) != task_report_channel(&candidate.arguments) {
        return false;
    }

    let existing_topic = task_topic_signature(&existing.arguments, &existing.description);
    let candidate_topic = task_topic_signature(&candidate.arguments, &candidate.description);
    topics_are_similar(&existing_topic, &candidate_topic)
}

impl Task {
    pub fn new(description: String, action: String, arguments: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            description,
            action,
            arguments,
            approval: TaskApproval::Auto,
            capabilities: vec![],
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            scheduled_for: None,
            cron: None,
            result: None,
            proof_id: None,
            priority: None,
            urgency: None,
            importance: None,
            eisenhower_quadrant: None,
        }
    }
}

/// Queue of tasks for autonomous execution
pub struct TaskQueue {
    tasks: Vec<Task>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self { tasks: vec![] }
    }

    pub fn add(&mut self, task: Task) {
        self.tasks.push(task);
    }

    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        before != self.tasks.len()
    }

    pub fn all(&self) -> &[Task] {
        &self.tasks
    }
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_task_approval_maps_notify_to_require() {
        assert!(matches!(
            normalized_task_approval(&TaskApproval::NotifyThenExecute { delay_seconds: 30 }),
            TaskApproval::RequireApproval
        ));
    }

    #[test]
    fn task_signature_ignores_approval_metadata() {
        let mut task = Task::new(
            "Delete junk email".to_string(),
            "gmail_delete".to_string(),
            serde_json::json!({
                "query": "label:junk older_than:30d",
                "_approval": {
                    "title": "Delete junk email",
                    "reason": "External mailbox mutation"
                }
            }),
        );
        task.approval = TaskApproval::RequireApproval;
        let left = task_semantic_signature(&task);

        task.arguments = serde_json::json!({
            "query": "label:junk older_than:30d"
        });
        let right = task_semantic_signature(&task);

        assert_eq!(left, right);
    }
}
