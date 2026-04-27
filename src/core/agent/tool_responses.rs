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

pub(super) fn humanize_tool_name(name: &str) -> String {
    name.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.extend(first.to_uppercase());
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
