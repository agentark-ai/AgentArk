use super::*;

pub(super) fn normalize_watcher_notification_text(text: &str, watcher_description: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut lines = trimmed.lines();
    let first_line = lines.next().unwrap_or_default().trim();
    let normalized_first_line = first_line
        .trim_matches('*')
        .trim_matches('#')
        .trim_matches(':')
        .trim();

    let body = if normalized_first_line.eq_ignore_ascii_case(watcher_description.trim()) {
        lines.collect::<Vec<_>>().join("\n")
    } else {
        trimmed.to_string()
    };

    body.trim().to_string()
}

pub(super) fn watcher_notification_value_preview(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Number(num) => Some(num.to_string()),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| safe_truncate(trimmed, 140))
        }
        serde_json::Value::Array(items) => (!items.is_empty()).then(|| {
            format!(
                "{} item{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            )
        }),
        serde_json::Value::Object(map) => (!map.is_empty()).then(|| {
            format!(
                "{} field{}",
                map.len(),
                if map.len() == 1 { "" } else { "s" }
            )
        }),
    }
}

pub(super) fn fallback_watcher_notification_from_structured_payload(
    watcher: &super::watcher::Watcher,
    payload: &serde_json::Value,
    image_attached: bool,
) -> Option<String> {
    let object = payload.as_object()?;
    let mut lines = Vec::new();
    for (key, value) in object {
        if key.starts_with('_') {
            continue;
        }
        let Some(preview) = watcher_notification_value_preview(value) else {
            continue;
        };
        lines.push(format!("- {}: {}", key.replace('_', " "), preview));
        if lines.len() >= 5 {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }

    let headline = object
        .get("message")
        .or_else(|| object.get("summary"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| safe_truncate(value, 220))
        .unwrap_or_else(|| format!("Watcher matched: {}", watcher.description.trim()));

    let mut sections = vec![headline, lines.join("\n")];
    if image_attached {
        sections.push("A snapshot is attached.".to_string());
    }
    Some(sections.join("\n\n"))
}

pub(super) fn fallback_watcher_notification_text(
    watcher: &super::watcher::Watcher,
    result: &str,
) -> String {
    let image_attached = watcher_result_output_files(result)
        .iter()
        .any(|path| watcher_output_file_is_image(path));
    if let Some(payload) = Agent::extract_structured_watcher_condition_payload(result) {
        if let Some(summary) =
            fallback_watcher_notification_from_structured_payload(watcher, &payload, image_attached)
        {
            return summary;
        }
    }

    let primary = automation_primary_result_text(result);
    let cleaned_result = primary
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.starts_with("AgentArk auto-installed sandbox dependencies:")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let result_text = cleaned_result.trim();
    if result_text.is_empty() {
        if image_attached {
            format!(
                "Watcher matched: {}\n\nA snapshot is attached.",
                watcher.description.trim()
            )
        } else {
            format!("Watcher matched: {}", watcher.description.trim())
        }
    } else {
        let mut message = format!(
            "Watcher matched: {}\n\n{}",
            watcher.description.trim(),
            result_text
        );
        if image_attached {
            message.push_str("\n\nA snapshot is attached.");
        }
        message
    }
}

#[derive(Debug, Clone)]
pub(super) struct WatcherNotificationImage {
    pub(super) web_path: String,
    pub(super) filename: String,
    pub(super) bytes: Vec<u8>,
}

/// Decide whether a candidate watcher-notification body is fit to surface to
/// the user, based on the *shape* of the text rather than predicted phrases.
///
/// Earlier revisions matched a curated list of failure-marker substrings
/// ("traceback", "raw result:", "exhausted its eligible model chain", …),
/// which broke whenever an upstream emitter rephrased its diagnostic output.
/// The structural checks below look at machine-output shape — pure JSON
/// payloads, stack-trace-frame layouts — which are invariant to wording
/// changes upstream and to user phrasing entirely.
///
/// Long-term, watcher emitters should attach a typed `kind` field at the
/// source so consumers do not have to inspect the body at all; this filter
/// is a structural fallback for text bodies that arrive without that typing.
pub(super) fn watcher_notification_text_is_useful(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 8 {
        return false;
    }

    // A bare structured payload (JSON object or array) is never a useful
    // notification body on its own — the user wants the summary, not the raw
    // event envelope.
    let starts_with_structured_open = trimmed.starts_with('{') || trimmed.starts_with('[');
    let ends_with_structured_close = trimmed.ends_with('}') || trimmed.ends_with(']');
    if starts_with_structured_open && ends_with_structured_close {
        return false;
    }

    // Stack-trace shape: at least half the sampled non-blank lines look like
    // stack-trace frames (indented + carrying a file:line reference, or
    // beginning with the language-agnostic frame marker `at `, or beginning
    // with the Python-style `File "…"`). These are *machine-output formats*,
    // not user phrasing — robust under rephrased error text.
    let mut sampled = 0usize;
    let mut traceback_like = 0usize;
    let mut python_file_frames = 0usize;
    for line in trimmed.lines().take(16) {
        let raw_line = line;
        let inner = raw_line.trim();
        if inner.is_empty() {
            continue;
        }
        sampled += 1;

        let starts_indented =
            raw_line.starts_with(' ') || raw_line.starts_with('\t');
        // Common source-file extension markers occurring with a colon-line
        // tail (file.ext:NN) — a structural feature of nearly every stack
        // trace format we're likely to receive.
        let has_file_line_marker = inner
            .split_whitespace()
            .any(|tok| tok.contains('.') && tok.contains(':'));
        let python_file_frame = inner.starts_with("File \"");
        let frame_prefix = inner.starts_with("at ") || python_file_frame;

        if (starts_indented && has_file_line_marker) || frame_prefix {
            traceback_like += 1;
        }
        if python_file_frame {
            python_file_frames += 1;
        }
    }
    if python_file_frames >= 2 {
        return false;
    }
    if sampled >= 3 && traceback_like * 2 >= sampled {
        return false;
    }

    true
}

pub(super) fn watcher_internal_match_text(
    watcher: &super::watcher::Watcher,
    result: &str,
    reason: &str,
) -> String {
    let primary = automation_primary_result_text(result);
    let cleaned = primary
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.starts_with("AgentArk auto-installed sandbox dependencies:")
        })
        .take(8)
        .collect::<Vec<_>>()
        .join("\n");
    if cleaned.trim().is_empty() {
        format!(
            "Watcher matched: {}\n\nExternal notification suppressed: {}.",
            watcher.description.trim(),
            reason
        )
    } else {
        format!(
            "Watcher matched: {}\n\nExternal notification suppressed: {}.\n\nPoll result:\n{}",
            watcher.description.trim(),
            reason,
            cleaned
        )
    }
}

pub(super) fn watcher_result_output_files(result: &str) -> Vec<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(result)
        .ok()
        .or_else(|| extract_json_object_from_text(result));
    parsed
        .as_ref()
        .and_then(|value| value.get("files"))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn watcher_output_file_is_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    matches!(
        lower.rsplit('.').next().unwrap_or_default(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    )
}

pub(super) fn parse_output_web_path(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim();
    let rest = trimmed.strip_prefix("/api/outputs/")?;
    let mut parts = rest.splitn(2, '/');
    let exec_id = parts.next()?.trim();
    let filename = parts.next()?.trim();
    if uuid::Uuid::parse_str(exec_id).is_err()
        || filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return None;
    }
    Some((exec_id.to_string(), filename.to_string()))
}

pub(super) fn normalize_automation_notification_channel(value: Option<&str>) -> String {
    match value
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| raw.to_ascii_lowercase())
    {
        Some(channel) if channel == "push" => "preferred".to_string(),
        Some(channel)
            if matches!(
                channel.as_str(),
                "app" | "app_notification" | "app_notifications" | "in_app"
            ) =>
        {
            String::new()
        }
        Some(channel) if matches!(channel.as_str(), "auto" | "default") => "preferred".to_string(),
        Some(channel) => channel,
        None => "preferred".to_string(),
    }
}

pub(super) fn watcher_delivery_label(notify_channel: &str) -> String {
    let normalized = notify_channel.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "In-app notification only".to_string()
    } else if normalized == "preferred" {
        "Preferred notification channel when connected, otherwise in-app".to_string()
    } else if is_external_notification_channel(&normalized) {
        notification_channel_display_name(&normalized).to_string()
    } else {
        normalized
    }
}

pub(super) fn planner_integration_class_name(
    class: &crate::actions::PlannerIntegrationClass,
) -> &'static str {
    match class {
        crate::actions::PlannerIntegrationClass::Internal => "internal",
        crate::actions::PlannerIntegrationClass::Messaging => "messaging",
        crate::actions::PlannerIntegrationClass::Workspace => "workspace",
        crate::actions::PlannerIntegrationClass::Search => "search",
        crate::actions::PlannerIntegrationClass::Browser => "browser",
        crate::actions::PlannerIntegrationClass::Filesystem => "filesystem",
        crate::actions::PlannerIntegrationClass::App => "app",
        crate::actions::PlannerIntegrationClass::Code => "code",
        crate::actions::PlannerIntegrationClass::Network => "network",
        crate::actions::PlannerIntegrationClass::Commerce => "commerce",
        crate::actions::PlannerIntegrationClass::Analytics => "analytics",
        crate::actions::PlannerIntegrationClass::Media => "media",
        crate::actions::PlannerIntegrationClass::Unknown => "unknown",
    }
}

pub(super) fn planner_action_role_name(role: &crate::actions::PlannerActionRole) -> &'static str {
    match role {
        crate::actions::PlannerActionRole::Trigger => "trigger",
        crate::actions::PlannerActionRole::Delivery => "delivery",
        crate::actions::PlannerActionRole::DataSource => "data_source",
        crate::actions::PlannerActionRole::Mutation => "mutation",
        crate::actions::PlannerActionRole::Inspection => "inspection",
        crate::actions::PlannerActionRole::Orchestration => "orchestration",
        crate::actions::PlannerActionRole::Unknown => "unknown",
    }
}

#[allow(dead_code)]
pub(super) fn sanitize_integration_class_list(values: &[String]) -> Vec<String> {
    let allowed = [
        "internal",
        "messaging",
        "workspace",
        "search",
        "browser",
        "filesystem",
        "app",
        "code",
        "network",
        "commerce",
        "analytics",
        "media",
    ];
    let mut seen = HashSet::new();
    let mut cleaned = Vec::new();
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty()
            || !allowed.contains(&normalized.as_str())
            || !seen.insert(normalized.clone())
        {
            continue;
        }
        cleaned.push(normalized);
    }
    cleaned
}

pub(super) fn automation_notification_message_from_text(
    task_desc: &str,
    action_arguments: &serde_json::Value,
) -> String {
    if let Some(message) = action_arguments
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return message.to_string();
    }

    let trimmed = task_desc.trim();
    if trimmed.is_empty() {
        return "Reminder".to_string();
    }

    let lower = trimmed.to_ascii_lowercase();
    for prefix in [
        "notify user:",
        "notify me:",
        "message me:",
        "message user:",
        "alert me:",
        "tell me:",
        "ping me:",
    ] {
        if lower.starts_with(prefix) {
            let candidate = trimmed[prefix.len()..].trim();
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
    }

    for prefix in [
        "remind me to ",
        "remind me ",
        "notify me when ",
        "tell me when ",
    ] {
        if lower.starts_with(prefix) {
            let candidate = trimmed[prefix.len()..].trim();
            if !candidate.is_empty() {
                return format!("Reminder: {}", candidate);
            }
        }
    }

    format!("Reminder: {}", trimmed.trim_end_matches(['.', '!', '?']))
}

pub(super) fn schedule_action_is_internal_notification(action_name: &str) -> bool {
    matches!(
        action_name.trim().to_ascii_lowercase().as_str(),
        "notify_user" | "goal_reminder" | "goal_progress_report" | "daily_brief"
    )
}

pub(super) fn build_notify_user_action_arguments(
    existing_arguments: &serde_json::Value,
    task_desc: &str,
    delivery_channel: &str,
) -> serde_json::Value {
    let mut payload = existing_arguments.as_object().cloned().unwrap_or_default();
    payload.remove("query");
    payload.insert(
        "message".to_string(),
        serde_json::Value::String(automation_notification_message_from_text(
            task_desc,
            existing_arguments,
        )),
    );
    if delivery_channel.trim().is_empty() {
        payload.remove("report_to");
    } else {
        payload.insert(
            "report_to".to_string(),
            serde_json::Value::String(delivery_channel.trim().to_ascii_lowercase()),
        );
    }
    serde_json::Value::Object(payload)
}

pub(super) fn schedule_task_should_default_to_notify_user(
    task_desc: &str,
    explicit_action: Option<&str>,
    scheduled_for: Option<chrono::DateTime<chrono::Utc>>,
    existing_arguments: &serde_json::Value,
) -> bool {
    if scheduled_for.is_none() {
        return false;
    }
    if explicit_action
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }

    if let Some(arguments) = existing_arguments.as_object() {
        let has_non_notification_keys = arguments.keys().any(|key| {
            !matches!(
                key.as_str(),
                "query" | "report_to" | "title" | "message" | "_automation"
            )
        });
        if has_non_notification_keys {
            return false;
        }
    }

    let normalized = task_desc.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if [
        "remind",
        "notify",
        "meeting",
        "appointment",
        "birthday",
        "anniversary",
        "interview",
        "doctor",
        "dentist",
        "reservation",
        "flight",
        "train",
        "call with",
    ]
    .iter()
    .any(|token| normalized.contains(token))
    {
        return true;
    }

    let words = normalized
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'' && ch != '-')
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() || words.len() > 8 {
        return false;
    }

    let first = words[0];
    if matches!(
        first,
        "run"
            | "generate"
            | "create"
            | "send"
            | "check"
            | "scan"
            | "sync"
            | "backup"
            | "deploy"
            | "research"
            | "review"
            | "summarize"
            | "fetch"
            | "refresh"
            | "update"
            | "call"
            | "email"
            | "draft"
            | "write"
    ) {
        return false;
    }

    true
}

pub(super) fn build_current_time_action_arguments(
    existing_arguments: &serde_json::Value,
    timezone: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    let timezone_value = existing_arguments
        .get("timezone")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(timezone.map(str::trim).filter(|value| !value.is_empty()));
    if let Some(timezone_value) = timezone_value {
        payload.insert(
            "timezone".to_string(),
            serde_json::Value::String(timezone_value.to_string()),
        );
    }
    serde_json::Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_when_text_is_too_short() {
        assert!(!watcher_notification_text_is_useful(""));
        assert!(!watcher_notification_text_is_useful("       "));
        assert!(!watcher_notification_text_is_useful("short"));
    }

    #[test]
    fn rejects_pure_json_object_payload() {
        assert!(!watcher_notification_text_is_useful(
            r#"{"event":"matched","data":{"id":42,"label":"daily-import"}}"#
        ));
    }

    #[test]
    fn rejects_pure_json_array_payload() {
        assert!(!watcher_notification_text_is_useful(
            r#"[{"event":"matched"},{"event":"continued"}]"#
        ));
    }

    #[test]
    fn rejects_stack_trace_shape() {
        let text = "RuntimeError: failed to compute\n    at module.run (worker.py:148)\n    at supervisor.dispatch (router.py:91)\n    at main.spawn (entrypoint.py:14)";
        assert!(!watcher_notification_text_is_useful(text));
    }

    #[test]
    fn rejects_python_style_traceback() {
        let text = "Traceback caused at upstream\n  File \"job.py\", line 14, in run\n    raise ValueError(\"oops\")\n  File \"sup.py\", line 91, in dispatch\n    return self.run()";
        assert!(!watcher_notification_text_is_useful(text));
    }

    #[test]
    fn accepts_human_readable_summary() {
        let text =
            "Watcher matched: the daily import has finished and produced 42 new rows in the staging table.";
        assert!(watcher_notification_text_is_useful(text));
    }

    #[test]
    fn accepts_paraphrased_summaries_uniformly() {
        // The previous phrase-marker filter would have rejected anything
        // containing the curated noise tokens. Both of these phrasings carry
        // the same useful intent; the structural filter accepts both.
        let phrasing_a =
            "Heads up: the nightly job just wrapped up and 42 new orders are ready for review.";
        let phrasing_b =
            "Hey, the overnight batch is done and forty-two fresh orders are waiting on you.";
        assert!(watcher_notification_text_is_useful(phrasing_a));
        assert!(watcher_notification_text_is_useful(phrasing_b));
    }

    #[test]
    fn accepts_summary_that_happens_to_mention_a_traceback() {
        // Old curated-list behaviour rejected anything containing the word
        // "traceback". The new structural filter only rejects content that
        // *is shaped like* a traceback — a one-line summary with the word in
        // it is fine.
        let text =
            "The watcher captured a runtime error in the import job; full traceback is stored under run-2025-04-26-01.";
        assert!(watcher_notification_text_is_useful(text));
    }
}
