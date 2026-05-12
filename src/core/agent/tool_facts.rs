use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolFactResourceKind {
    App,
    Watcher,
    BackgroundSession,
    Task,
    Approval,
    Conversation,
    File,
    Integration,
    External,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ToolResourceRef {
    pub kind: ToolFactResourceKind,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ToolFacts {
    pub action_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub resources: Vec<ToolResourceRef>,
    #[serde(default)]
    pub timestamps: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ToolFacts {
    pub(super) fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.reason.is_none()
            && self.resources.is_empty()
            && self.timestamps.is_empty()
            && self.notification_channel.is_none()
            && self.error_reason.is_none()
            && self.detail.is_none()
    }
}

pub(super) fn extract_tool_facts(
    action_name: &str,
    parsed_result: &serde_json::Value,
) -> ToolFacts {
    let mut facts = ToolFacts {
        action_name: action_name.to_string(),
        ..ToolFacts::default()
    };
    collect_value_facts(None, parsed_result, &mut facts, 0);
    dedup_resources(&mut facts.resources);
    facts.timestamps.sort();
    facts.timestamps.dedup();
    facts.timestamps.truncate(12);
    facts
}

pub(super) fn compact_tool_result_for_history(
    parsed_result: &serde_json::Value,
    facts: &ToolFacts,
) -> serde_json::Value {
    let serialized = serde_json::to_string(parsed_result).unwrap_or_else(|_| "null".to_string());
    if serialized.chars().count() <= 5_000 {
        return parsed_result.clone();
    }
    serde_json::json!({
        "compacted": true,
        "status": &facts.status,
        "reason": &facts.reason,
        "detail": &facts.detail,
        "resources": &facts.resources,
        "timestamps": &facts.timestamps,
        "notification_channel": &facts.notification_channel,
        "error_reason": &facts.error_reason,
        "preview": safe_truncate(&crate::security::redact_secret_input(&serialized).text, 2_500),
    })
}

fn collect_value_facts(
    key: Option<&str>,
    value: &serde_json::Value,
    facts: &mut ToolFacts,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (child_key, child_value) in map {
                collect_value_facts(Some(child_key), child_value, facts, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter().take(64) {
                collect_value_facts(key, item, facts, depth + 1);
            }
        }
        serde_json::Value::String(text) => {
            apply_scalar_fact(key, text, facts);
        }
        serde_json::Value::Number(number) => {
            apply_scalar_fact(key, &number.to_string(), facts);
        }
        serde_json::Value::Bool(value) => {
            apply_scalar_fact(key, &value.to_string(), facts);
        }
        serde_json::Value::Null => {}
    }
}

fn apply_scalar_fact(key: Option<&str>, value: &str, facts: &mut ToolFacts) {
    let Some(key) = key else {
        return;
    };
    let normalized = normalize_key(key);
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    if normalized == "status" {
        facts.status.get_or_insert_with(|| safe_truncate(value, 80));
    } else if normalized == "reason" {
        facts.reason.get_or_insert_with(|| safe_truncate(value, 120));
    } else if normalized == "detail" || normalized == "message" || normalized == "summary" {
        facts.detail.get_or_insert_with(|| safe_truncate(value, 260));
    } else if normalized == "error" || normalized == "errorreason" {
        facts.error_reason
            .get_or_insert_with(|| safe_truncate(value, 160));
    } else if normalized.contains("notifychannel")
        || normalized.contains("notificationchannel")
        || normalized == "deliverychannel"
        || normalized == "reportto"
    {
        facts
            .notification_channel
            .get_or_insert_with(|| safe_truncate(value, 80));
    } else if normalized.ends_with("at")
        || normalized.contains("timestamp")
        || normalized.contains("nextpoll")
        || normalized.contains("nextrun")
    {
        facts
            .timestamps
            .push((safe_truncate(key, 80), safe_truncate(value, 120)));
    }

    if normalized == "id" || normalized.ends_with("id") {
        facts.resources.push(ToolResourceRef {
            kind: resource_kind_from_key(&normalized),
            id: safe_truncate(value, 220),
            label: None,
        });
    }
}

fn resource_kind_from_key(normalized_key: &str) -> ToolFactResourceKind {
    if normalized_key.contains("backgroundsession") {
        ToolFactResourceKind::BackgroundSession
    } else if normalized_key.contains("watcher") || normalized_key.contains("monitor") {
        ToolFactResourceKind::Watcher
    } else if normalized_key.contains("task") {
        ToolFactResourceKind::Task
    } else if normalized_key.contains("approval") {
        ToolFactResourceKind::Approval
    } else if normalized_key.contains("conversation") || normalized_key.contains("thread") {
        ToolFactResourceKind::Conversation
    } else if normalized_key.contains("app") || normalized_key.contains("artifact") {
        ToolFactResourceKind::App
    } else if normalized_key.contains("file") || normalized_key.contains("document") {
        ToolFactResourceKind::File
    } else if normalized_key.contains("integration") || normalized_key.contains("account") {
        ToolFactResourceKind::Integration
    } else if normalized_key.contains("url") || normalized_key.contains("resource") {
        ToolFactResourceKind::External
    } else {
        ToolFactResourceKind::Unknown
    }
}

fn dedup_resources(resources: &mut Vec<ToolResourceRef>) {
    let mut seen = BTreeSet::new();
    resources.retain(|resource| {
        seen.insert(format!("{:?}:{}", &resource.kind, resource.id))
    });
    resources.truncate(16);
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_resource_ids_and_status_from_nested_tool_output() {
        let facts = extract_tool_facts(
            "watch",
            &serde_json::json!({
                "status": "completed",
                "data": {
                    "watcher_id": "w1",
                    "background_session_id": "s1",
                    "next_poll_at": "2026-05-11T10:00:00Z"
                }
            }),
        );

        assert_eq!(facts.status.as_deref(), Some("completed"));
        assert!(facts
            .resources
            .iter()
            .any(|resource| resource.kind == ToolFactResourceKind::Watcher && resource.id == "w1"));
        assert!(facts.timestamps.iter().any(|(_, value)| value.contains("2026")));
    }
}
