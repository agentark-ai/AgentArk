use super::*;

pub(super) fn extract_json_object_from_text(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return value.is_object().then_some(value);
    }

    let bytes = text.as_bytes();
    let mut start = None::<usize>;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            b'}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    let begin = start?;
                    let candidate = &text[begin..=index];
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
                        if value.is_object() {
                            return Some(value);
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }

    None
}

pub(super) fn tool_batch_output_failed(
    batch: &tool_execution::ToolExecutionBatch,
    index: usize,
) -> bool {
    batch
        .outcomes
        .get(index)
        .map(|outcome| outcome.status != crate::core::ToolOutcomeStatus::Success)
        .unwrap_or(false)
}

pub(super) fn tool_batch_output_succeeded(
    batch: &tool_execution::ToolExecutionBatch,
    index: usize,
) -> bool {
    !tool_batch_output_failed(batch, index)
}

pub(super) fn summarize_structured_tool_output_for_user(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(payload) = trimmed
        .trim_start()
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
    {
        let payload = payload.lines().next().unwrap_or(payload).trim();
        let value = serde_json::from_str::<serde_json::Value>(payload).ok()?;
        return summarize_tool_completion_value(&value);
    }

    let value = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    if let Some(result) = value.get("result").and_then(|inner| inner.as_str()) {
        if let Some(summary) = summarize_structured_tool_output_for_user(result) {
            return Some(summary);
        }
    }
    summarize_tool_completion_value(&value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StructuredToolOutcomeState {
    Success,
    Failure,
    NeedsInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StructuredToolOutcomeReport {
    pub state: StructuredToolOutcomeState,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub exit_code: Option<i64>,
}

pub(super) fn structured_tool_result_outcome(content: &str) -> Option<StructuredToolOutcomeReport> {
    let value = parse_structured_value_from_text(content)?;
    structured_tool_value_outcome(&value)
}

pub(super) fn structured_tool_value_outcome(
    value: &serde_json::Value,
) -> Option<StructuredToolOutcomeReport> {
    structured_tool_value_outcome_at_depth(value, 0)
}

#[cfg(test)]
pub(super) fn structured_tool_result_success(content: &str) -> Option<bool> {
    structured_tool_result_outcome(content)
        .map(|report| matches!(report.state, StructuredToolOutcomeState::Success))
}

#[cfg(test)]
pub(super) fn tool_result_completion_success(result: &str) -> Option<bool> {
    if let Some(success) = structured_tool_result_success(result) {
        return Some(success);
    }
    let completion = crate::runtime::parse_watch_completion(result)
        .or_else(|| crate::runtime::parse_schedule_task_completion(result))
        .or_else(|| crate::runtime::parse_delegate_completion(result));
    if let Some(completion) = completion {
        let status = completion.status.trim();
        return Some(matches!(status, "completed" | "succeeded" | "success"));
    }
    let parsed = extract_json_object_from_text(result)?;
    parsed
        .get("status")
        .and_then(|value| value.as_str())
        .and_then(structured_status_outcome_state)
        .map(|state| matches!(state, StructuredToolOutcomeState::Success))
}

pub(super) fn structured_tool_value_reports_degenerate_output(value: &serde_json::Value) -> bool {
    structured_tool_value_reports_degenerate_output_at_depth(value, 0)
}

fn structured_tool_value_reports_degenerate_output_at_depth(
    value: &serde_json::Value,
    depth: usize,
) -> bool {
    if depth > 4 {
        return false;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    structured_object_reports_degenerate_output(object)
        || object
            .get("result")
            .and_then(parse_nested_structured_value)
            .is_some_and(|nested| {
                structured_tool_value_reports_degenerate_output_at_depth(&nested, depth + 1)
            })
}

fn structured_object_reports_degenerate_output(
    object: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    if quality_value_is_degenerate(object.get("content_quality"))
        || quality_value_is_degenerate(object.get("body_quality"))
        || quality_value_is_degenerate(object.get("output_quality"))
    {
        return true;
    }

    object
        .get("data")
        .and_then(|data| data.as_object())
        .is_some_and(|data| {
            quality_value_is_degenerate(data.get("content_quality"))
                || quality_value_is_degenerate(data.get("body_quality"))
                || quality_value_is_degenerate(data.get("output_quality"))
        })
}

fn quality_value_is_degenerate(value: Option<&serde_json::Value>) -> bool {
    let Some(object) = value.and_then(|value| value.as_object()) else {
        return false;
    };
    object
        .get("degenerate")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn summarize_tool_completion_value(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    if let Some(summary) = summarize_structured_non_success_value(value) {
        return Some(summary);
    }
    let tool = object
        .get("tool")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty());
    let status = object
        .get("status")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty());
    let detail = object
        .get("detail")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty());

    if tool.is_none() && status.is_none() && detail.is_none() {
        return summarize_structured_collection_value(object);
    }

    let status_key = status.unwrap_or("completed").trim().to_ascii_lowercase();
    let is_success_status = matches!(
        normalize_structured_key(&status_key).as_str(),
        "ok" | "complete" | "completed" | "success" | "succeeded" | "executed" | ""
    );
    if is_success_status {
        if let Some(summary) = summarize_success_payload_value(value) {
            return Some(summary);
        }
    }
    let status_label = match status_key.as_str() {
        "completed" | "complete" | "succeeded" | "success" | "ok" | "executed" => "Completed",
        "needs_input" | "approval_required" => "Needs input",
        "failed" | "error" => "Failed",
        "" => "Completed",
        _ => "Status",
    };

    let mut lines = Vec::new();
    let headline = match detail {
        Some(detail) => format!("{status_label}: {detail}"),
        None => {
            if matches!(
                status_key.as_str(),
                "completed" | "complete" | "succeeded" | "success" | "ok" | "executed" | ""
            ) {
                if let Some(data_summary) = object
                    .get("data")
                    .and_then(|data| data.as_object())
                    .and_then(summarize_structured_collection_value)
                {
                    return Some(format!("{status_label}: {data_summary}"));
                }
            }
            match tool {
                Some(tool) => format!("{status_label}: `{tool}` returned a structured result."),
                None => format!("{status_label}."),
            }
        }
    };
    lines.push(headline);

    lines.dedup();
    Some(lines.join("\n"))
}

fn summarize_success_payload_value(value: &serde_json::Value) -> Option<String> {
    summarize_success_payload_value_at_depth(value, 0)
}

fn summarize_success_payload_value_at_depth(
    value: &serde_json::Value,
    depth: usize,
) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match value {
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some(parsed) = parse_structured_value_from_text(trimmed) {
                if !matches!(parsed, serde_json::Value::String(_)) {
                    if let Some(summary) =
                        summarize_success_payload_value_at_depth(&parsed, depth + 1)
                    {
                        return Some(summary);
                    }
                }
            }
            success_payload_text_candidate(trimmed).map(ToString::to_string)
        }
        serde_json::Value::Object(object) => {
            if let Some(summary) = object
                .get("data")
                .and_then(|data| data.as_object())
                .and_then(summarize_structured_collection_value)
            {
                return Some(summary);
            }
            let mut candidates = object
                .iter()
                .filter(|(key, _)| !structured_success_payload_metadata_key(key))
                .filter_map(|(key, value)| {
                    let summary = summarize_success_payload_value_at_depth(value, depth + 1)?;
                    let score = structured_success_payload_score(key, &summary);
                    Some((score, summary))
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| right.0.cmp(&left.0));
            candidates.into_iter().map(|(_, summary)| summary).next()
        }
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| summarize_success_payload_value_at_depth(item, depth + 1))
            .max_by_key(|summary| summary.chars().count()),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) | serde_json::Value::Null => None,
    }
}

fn structured_success_payload_metadata_key(key: &str) -> bool {
    let normalized = normalize_structured_key(key);
    normalized.is_empty()
        || normalized == "ok"
        || normalized == "success"
        || normalized == "status"
        || normalized == "kind"
        || normalized == "type"
        || normalized == "tool"
        || normalized == "name"
        || normalized == "id"
        || normalized.ends_with("id")
        || normalized.ends_with("ids")
        || normalized.contains("token")
        || normalized.contains("cost")
        || normalized.contains("elapsed")
        || normalized.contains("duration")
        || normalized.contains("timestamp")
        || normalized.contains("telemetry")
        || normalized.contains("metric")
        || normalized.contains("usage")
        || normalized.contains("trace")
}

fn structured_success_payload_score(key: &str, value: &str) -> usize {
    let normalized = normalize_structured_key(key);
    let semantic_bonus = if normalized.contains("result")
        || normalized.contains("answer")
        || normalized.contains("response")
        || normalized.contains("summary")
        || normalized.contains("content")
        || normalized.contains("message")
        || normalized.contains("output")
    {
        2_000
    } else {
        0
    };
    semantic_bonus + value.chars().count().min(4_000)
}

fn success_payload_text_candidate(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with(crate::runtime::TOOL_COMPLETION_MARKER)
    {
        return None;
    }
    if trimmed.chars().count() < 8 && !trimmed.contains(char::is_whitespace) {
        return None;
    }
    Some(trimmed)
}

fn parse_structured_value_from_text(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(payload) = trimmed
        .trim_start()
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
    {
        let payload = payload.lines().next().unwrap_or(payload).trim();
        return serde_json::from_str::<serde_json::Value>(payload).ok();
    }
    serde_json::from_str::<serde_json::Value>(trimmed).ok()
}

fn structured_tool_value_outcome_at_depth(
    value: &serde_json::Value,
    depth: usize,
) -> Option<StructuredToolOutcomeReport> {
    if depth > 4 {
        return None;
    }
    let object = value.as_object()?;
    let own = structured_object_own_outcome(object);
    let nested = object
        .get("result")
        .and_then(parse_nested_structured_value)
        .and_then(|nested| structured_tool_value_outcome_at_depth(&nested, depth + 1));

    match (own, nested) {
        (Some(report), _) if report.state != StructuredToolOutcomeState::Success => Some(report),
        (Some(report), Some(nested)) if report.state == StructuredToolOutcomeState::Success => {
            Some(nested)
        }
        (Some(report), Some(_)) => Some(report),
        (Some(report), None) => Some(report),
        (None, Some(nested)) => Some(nested),
        (None, None) => None,
    }
}

fn structured_object_own_outcome(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<StructuredToolOutcomeReport> {
    let exit_code = find_i64_field(object, "exitcode");
    if exit_code.is_some_and(|code| code != 0) {
        return Some(StructuredToolOutcomeReport {
            state: StructuredToolOutcomeState::Failure,
            reason: structured_failure_reason(object).or_else(|| Some("nonzero_exit".to_string())),
            message: structured_failure_message(object),
            exit_code,
        });
    }

    if let Some(status) = find_string_field(object, "status") {
        if let Some(state) = state_from_structured_status(status) {
            if state != StructuredToolOutcomeState::Success {
                return Some(StructuredToolOutcomeReport {
                    state,
                    reason: structured_failure_reason(object)
                        .or_else(|| Some(normalize_structured_key(status))),
                    message: structured_failure_message(object),
                    exit_code,
                });
            }
            return Some(StructuredToolOutcomeReport {
                state: StructuredToolOutcomeState::Success,
                reason: None,
                message: None,
                exit_code,
            });
        }
    }

    if let Some(false) =
        find_bool_field(object, "success").or_else(|| find_bool_field(object, "ok"))
    {
        return Some(StructuredToolOutcomeReport {
            state: StructuredToolOutcomeState::Failure,
            reason: structured_failure_reason(object)
                .or_else(|| Some("reported_failure".to_string())),
            message: structured_failure_message(object),
            exit_code,
        });
    }

    if structured_error_field_is_present(object) {
        return Some(StructuredToolOutcomeReport {
            state: StructuredToolOutcomeState::Failure,
            reason: structured_failure_reason(object)
                .or_else(|| Some("reported_error".to_string())),
            message: structured_failure_message(object),
            exit_code,
        });
    }

    if find_bool_field(object, "success").or_else(|| find_bool_field(object, "ok")) == Some(true)
        || exit_code == Some(0)
    {
        return Some(StructuredToolOutcomeReport {
            state: StructuredToolOutcomeState::Success,
            reason: None,
            message: None,
            exit_code,
        });
    }

    None
}

fn parse_nested_structured_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::String(text) => parse_structured_value_from_text(text),
        serde_json::Value::Object(_) => Some(value.clone()),
        _ => None,
    }
}

pub(super) fn structured_status_outcome_state(status: &str) -> Option<StructuredToolOutcomeState> {
    let canonical = canonical_machine_status(status)?;
    if let Ok(status) = serde_json::from_value::<crate::core::ToolOutcomeStatus>(
        serde_json::Value::String(canonical.clone()),
    ) {
        return Some(match status {
            crate::core::ToolOutcomeStatus::Success => StructuredToolOutcomeState::Success,
            crate::core::ToolOutcomeStatus::NeedsInput => StructuredToolOutcomeState::NeedsInput,
            crate::core::ToolOutcomeStatus::RecoverableError
            | crate::core::ToolOutcomeStatus::FatalError
            | crate::core::ToolOutcomeStatus::Blocked
            | crate::core::ToolOutcomeStatus::Cancelled
            | crate::core::ToolOutcomeStatus::TimedOut
            | crate::core::ToolOutcomeStatus::NoHandler => StructuredToolOutcomeState::Failure,
        });
    }
    if let Ok(status) = serde_json::from_value::<crate::core::ExecutionRunStatus>(
        serde_json::Value::String(canonical.clone()),
    ) {
        return match status {
            crate::core::ExecutionRunStatus::Completed
            | crate::core::ExecutionRunStatus::Degraded => {
                Some(StructuredToolOutcomeState::Success)
            }
            crate::core::ExecutionRunStatus::NeedsInput
            | crate::core::ExecutionRunStatus::NeedsStrongerModel => {
                Some(StructuredToolOutcomeState::NeedsInput)
            }
            crate::core::ExecutionRunStatus::Blocked
            | crate::core::ExecutionRunStatus::PlatformFailed
            | crate::core::ExecutionRunStatus::Cancelled => {
                Some(StructuredToolOutcomeState::Failure)
            }
            crate::core::ExecutionRunStatus::Accepted
            | crate::core::ExecutionRunStatus::Routing
            | crate::core::ExecutionRunStatus::ModelSelection
            | crate::core::ExecutionRunStatus::Planning
            | crate::core::ExecutionRunStatus::ToolDispatch
            | crate::core::ExecutionRunStatus::Synthesis => None,
        };
    }
    if let Ok(status) = serde_json::from_value::<crate::core::UserFacingOutcomeStatus>(
        serde_json::Value::String(canonical.clone()),
    ) {
        return Some(match status {
            crate::core::UserFacingOutcomeStatus::Complete
            | crate::core::UserFacingOutcomeStatus::Degraded => StructuredToolOutcomeState::Success,
            crate::core::UserFacingOutcomeStatus::NeedsClarification
            | crate::core::UserFacingOutcomeStatus::NeedsPermission
            | crate::core::UserFacingOutcomeStatus::NeedsIntegration
            | crate::core::UserFacingOutcomeStatus::NeedsCredentials
            | crate::core::UserFacingOutcomeStatus::NeedsStrongerModel => {
                StructuredToolOutcomeState::NeedsInput
            }
            crate::core::UserFacingOutcomeStatus::ServiceUnavailable => {
                StructuredToolOutcomeState::Failure
            }
        });
    }

    match canonical.replace('_', "").as_str() {
        "ok" | "complete" | "completed" | "success" | "succeeded" | "executed" => {
            Some(StructuredToolOutcomeState::Success)
        }
        "approvalrequired" => Some(StructuredToolOutcomeState::NeedsInput),
        "error" | "failed" | "failure" => Some(StructuredToolOutcomeState::Failure),
        _ => None,
    }
}

fn state_from_structured_status(status: &str) -> Option<StructuredToolOutcomeState> {
    structured_status_outcome_state(status)
}

fn canonical_machine_status(status: &str) -> Option<String> {
    let mut output = String::new();
    let mut last_separator = false;
    for ch in status.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_separator = false;
        } else if (ch == '_' || ch == '-' || ch.is_ascii_whitespace())
            && !output.is_empty()
            && !last_separator
        {
            output.push('_');
            last_separator = true;
        }
    }
    while output.ends_with('_') {
        output.pop();
    }
    (!output.is_empty()).then_some(output)
}

fn structured_error_field_is_present(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.iter().any(|(key, value)| {
        normalize_structured_key(key) == "error" && structured_value_has_content(value)
    })
}

fn structured_failure_message(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    for key in ["message", "detail", "summary", "error", "stderr", "output"] {
        if let Some(value) = find_field_value(object, key).and_then(structured_display_value) {
            return Some(value);
        }
    }
    None
}

fn structured_failure_reason(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    find_string_field(object, "reason")
        .map(normalize_structured_key)
        .filter(|value| !value.is_empty())
}

fn summarize_structured_non_success_value(value: &serde_json::Value) -> Option<String> {
    let report = structured_tool_value_outcome(value)?;
    if report.state == StructuredToolOutcomeState::Success {
        return None;
    }

    let label = match report.state {
        StructuredToolOutcomeState::Success => return None,
        StructuredToolOutcomeState::Failure => "Failed",
        StructuredToolOutcomeState::NeedsInput => "Needs input",
    };

    let mut lines = Vec::new();
    if let Some(exit_code) = report.exit_code {
        lines.push(format!("{label}: exited with code {exit_code}."));
        if let Some(message) = report.message {
            append_structured_message_lines("Output", &message, &mut lines);
        }
    } else if let Some(message) = report.message {
        append_structured_message_lines(label, &message, &mut lines);
    } else if let Some(reason) = report.reason {
        lines.push(format!("{label}: {}.", humanize_structured_key(&reason)));
    } else {
        lines.push(format!(
            "{label}: the tool reported that the action did not complete."
        ));
    }

    Some(lines.join("\n"))
}

fn append_structured_message_lines(label: &str, message: &str, lines: &mut Vec<String>) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }
    if trimmed.contains('\n') {
        lines.push(format!("{label}:"));
        lines.push(trimmed.to_string());
    } else {
        lines.push(format!("{label}: {trimmed}"));
    }
}

fn find_field_value<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    normalized_key: &str,
) -> Option<&'a serde_json::Value> {
    object
        .iter()
        .find(|(key, _)| normalize_structured_key(key) == normalized_key)
        .map(|(_, value)| value)
}

fn find_string_field<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    normalized_key: &str,
) -> Option<&'a str> {
    find_field_value(object, normalized_key).and_then(|value| value.as_str())
}

fn find_bool_field(
    object: &serde_json::Map<String, serde_json::Value>,
    normalized_key: &str,
) -> Option<bool> {
    find_field_value(object, normalized_key).and_then(|value| value.as_bool())
}

fn find_i64_field(
    object: &serde_json::Map<String, serde_json::Value>,
    normalized_key: &str,
) -> Option<i64> {
    let value = find_field_value(object, normalized_key)?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

fn structured_display_value(value: &serde_json::Value) -> Option<String> {
    if !structured_value_has_content(value) {
        return None;
    }
    let raw = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
        serde_json::Value::Null => return None,
    };
    if raw.trim().is_empty() {
        return None;
    }
    let redacted = crate::security::redact_secret_input(&raw).text;
    Some(truncate_structured_failure_display_value(&redacted))
}

fn structured_value_has_content(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(items) => !items.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn truncate_structured_failure_display_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= 1_200 {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(1_197).collect::<String>();
    out.push_str("...");
    out
}

fn normalize_structured_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn summarize_structured_collection_value(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let (collection_key, items) = object.iter().find_map(|(key, value)| {
        let items = value.as_array()?;
        if items.is_empty() || items.iter().any(|item| item.is_object()) {
            Some((key.as_str(), items.as_slice()))
        } else {
            None
        }
    })?;
    let source = object
        .get("backend")
        .or_else(|| {
            object
                .get("retrieval")
                .and_then(|value| value.get("source"))
        })
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(humanize_structured_value)
        .unwrap_or_else(|| "The tool".to_string());
    let count = object
        .get("retrieval")
        .and_then(|value| value.get("count"))
        .and_then(|value| value.as_u64())
        .unwrap_or(items.len() as u64);
    let collection_label = humanize_structured_key(collection_key).to_ascii_lowercase();
    let item_label = if count == 1 {
        singular_collection_label(&collection_label)
    } else {
        collection_label
    };
    let mut lines = vec![format!("{source} returned {count} {item_label}.")];

    for item in items.iter().take(10) {
        if let Some(row) = summarize_collection_item(item) {
            lines.push(format!("- {row}"));
        }
    }
    if (items.len() as u64) > 10 {
        lines.push(format!(
            "... {} more item{} not shown.",
            items.len() - 10,
            if items.len() - 10 == 1 { "" } else { "s" }
        ));
    }

    Some(lines.join("\n"))
}

fn summarize_collection_item(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    let mut fields = object
        .iter()
        .filter_map(|(key, value)| scalar_display_value(value).map(|display| (key, display)))
        .filter(|(_, display)| !display.trim().is_empty())
        .take(8)
        .map(|(key, display)| format!("{}: {}", humanize_structured_key(key), display))
        .collect::<Vec<_>>();
    if fields.is_empty() {
        return None;
    }
    fields.sort();
    Some(fields.join("; "))
}

fn scalar_display_value(value: &serde_json::Value) -> Option<String> {
    let raw = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            return None;
        }
    };
    let redacted = crate::security::redact_secret_input(&raw).text;
    Some(truncate_structured_display_value(&redacted))
}

fn truncate_structured_display_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= 180 {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(177).collect::<String>();
    out.push_str("...");
    out
}

fn humanize_structured_value(value: &str) -> String {
    humanize_structured_key(value)
}

fn singular_collection_label(label: &str) -> String {
    label
        .strip_suffix("ies")
        .map(|prefix| format!("{prefix}y"))
        .or_else(|| label.strip_suffix('s').map(ToString::to_string))
        .unwrap_or_else(|| label.to_string())
}

fn humanize_structured_key(key: &str) -> String {
    let mut out = key
        .split(|ch: char| ch == '_' || ch == '-' || ch.is_whitespace())
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if out.is_empty() {
        out = "ID".to_string();
    }
    out
}

pub(super) fn render_tool_completion_marker_with_data(
    tool: &str,
    status: &str,
    detail: &str,
    data: serde_json::Value,
) -> String {
    let payload = serde_json::json!({
        "tool": tool,
        "status": status,
        "detail": if detail.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(detail.trim().to_string())
        },
        "data": data,
    });
    format!("{}{}", crate::runtime::TOOL_COMPLETION_MARKER, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_structured_collection_outputs_for_user() {
        let output = serde_json::json!({
            "backend": "Google Drive",
            "results": [
                {
                    "name": "roadmap.pdf",
                    "owner": "alex@example.com",
                    "modifiedTime": "2026-05-11T12:00:00Z",
                    "mimeType": "application/pdf"
                }
            ],
            "retrieval": {
                "count": 1,
                "page_size": 10
            }
        })
        .to_string();

        let summary = summarize_structured_tool_output_for_user(&output).unwrap();
        assert!(summary.contains("Google Drive returned 1 result."));
        assert!(summary.contains("Name: roadmap.pdf"));
        assert!(summary.contains("Owner: alex@example.com"));
    }

    #[test]
    fn ok_envelope_prefers_inner_structured_collection_summary() {
        let inner = serde_json::json!({
            "backend": "Connected Source",
            "results": [
                {
                    "title": "First item",
                    "status": "visible"
                }
            ],
            "retrieval": {
                "count": 1
            }
        })
        .to_string();
        let envelope = serde_json::json!({
            "status": "ok",
            "tool": "connector_read",
            "result": inner
        })
        .to_string();

        let summary = summarize_structured_tool_output_for_user(&envelope).unwrap();
        assert!(summary.contains("Connected Source returned 1 result."));
        assert!(!summary.contains("returned a structured result"));
    }

    #[test]
    fn ok_envelope_surfaces_nested_structured_failure() {
        let inner = serde_json::json!({
            "output": "ERROR: credentials unavailable\nREGISTRATION_FAILED: HTTP 409",
            "error": null,
            "exit_code": 1
        })
        .to_string();
        let envelope = serde_json::json!({
            "status": "ok",
            "tool": "external_action",
            "result": inner
        })
        .to_string();

        let summary = summarize_structured_tool_output_for_user(&envelope).unwrap();
        assert!(summary.contains("Failed: exited with code 1."));
        assert!(summary.contains("REGISTRATION_FAILED: HTTP 409"));
        assert!(!summary.contains("returned a structured result"));
        assert_eq!(structured_tool_result_success(&envelope), Some(false));
    }

    #[test]
    fn blocked_tool_status_is_not_treated_as_successful_raw_output() {
        let envelope = serde_json::json!({
            "status": "blocked",
            "tool": "policy_checked_action",
            "reason": "safety_blocked",
            "message": "The action was blocked by policy before execution."
        })
        .to_string();

        let summary = summarize_structured_tool_output_for_user(&envelope).unwrap();

        assert_eq!(structured_tool_result_success(&envelope), Some(false));
        assert!(summary.contains("Failed: The action was blocked by policy before execution."));
        assert!(!summary.contains("\"status\""));
    }

    #[test]
    fn degenerate_quality_flags_are_shared_structural_failures() {
        let envelope = serde_json::json!({
            "status": "completed",
            "tool": "page_fetch",
            "data": {
                "content_quality": {
                    "degenerate": true,
                    "reason": "empty_body"
                }
            }
        })
        .to_string();

        let value = serde_json::from_str::<serde_json::Value>(&envelope).unwrap();
        assert_eq!(structured_tool_result_success(&envelope), Some(true));
        assert!(structured_tool_value_reports_degenerate_output(&value));
    }

    #[test]
    fn project_machine_statuses_drive_structured_outcome_classification() {
        assert_eq!(
            structured_status_outcome_state(crate::core::ToolOutcomeStatus::Success.as_str()),
            Some(StructuredToolOutcomeState::Success)
        );
        assert_eq!(
            structured_status_outcome_state(crate::core::ToolOutcomeStatus::Blocked.as_str()),
            Some(StructuredToolOutcomeState::Failure)
        );
        assert_eq!(
            structured_status_outcome_state(crate::core::ToolOutcomeStatus::NeedsInput.as_str()),
            Some(StructuredToolOutcomeState::NeedsInput)
        );
        assert_eq!(
            structured_status_outcome_state(
                crate::core::ExecutionRunStatus::PlatformFailed.as_str()
            ),
            Some(StructuredToolOutcomeState::Failure)
        );
    }

    #[test]
    fn successful_tool_envelope_prefers_user_facing_result_text() {
        let envelope = serde_json::json!({
            "ok": true,
            "status": "completed",
            "kind": "delegate",
            "delegation_id": "1bf3b334-9689-469e-b8e3-d0e9128d9813",
            "agents_used": ["Helix", "Atlas", "Vanta"],
            "final_result": "Launch with a WhatsApp-first onboarding wedge, INR tiered pricing, and a compliance-forward trust package."
        })
        .to_string();

        let summary = summarize_structured_tool_output_for_user(&envelope).unwrap();

        assert!(summary.starts_with("Launch with a WhatsApp-first onboarding wedge"));
        assert!(!summary.contains("Delegation"));
        assert!(!summary.contains("1bf3b334"));
    }
}
