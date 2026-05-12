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

pub(super) fn looks_like_raw_structured_tool_output(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with(crate::runtime::TOOL_COMPLETION_MARKER)
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

fn summarize_tool_completion_value(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
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

    let status_key = status
        .unwrap_or("completed")
        .trim()
        .to_ascii_lowercase();
    let status_label = match status_key.as_str() {
        "completed" | "complete" | "succeeded" | "success" | "ok" | "executed" => "Completed",
        "needs_input" | "approval_required" => "Needs input",
        "failed" | "error" => "Failed",
        other if other.is_empty() => "Completed",
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
        },
    };
    lines.push(headline);

    append_structured_reference_lines(value, &mut lines);
    if let Some(data) = object.get("data") {
        append_structured_reference_lines(data, &mut lines);
    }

    lines.dedup();
    Some(lines.join("\n"))
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

fn append_structured_reference_lines(value: &serde_json::Value, lines: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let mut refs = object
        .iter()
        .filter_map(|(key, value)| {
            let normalized = key.trim().to_ascii_lowercase();
            if !(normalized == "id" || normalized.ends_with("_id")) {
                return None;
            }
            let value = value.as_str().map(str::trim).filter(|item| !item.is_empty())?;
            Some(format!("{}: {}", humanize_structured_key(key), value))
        })
        .collect::<Vec<_>>();
    refs.sort();
    refs.truncate(6);
    lines.extend(refs);
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

pub(super) fn looks_like_raw_source_or_markup_dump(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<HTML")
        || trimmed.starts_with("```")
        || trimmed.contains("</html>")
        || trimmed.contains("fn ")
        || trimmed.contains("class ")
}

#[allow(dead_code)]
pub(super) fn render_tool_completion_marker(tool: &str, status: &str, detail: &str) -> String {
    render_tool_completion_marker_with_data(tool, status, detail, serde_json::Value::Null)
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
}
