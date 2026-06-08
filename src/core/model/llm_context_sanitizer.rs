use crate::core::ConversationMessage;
use serde_json::{Map, Value};

const MAX_PROMPT_TEXT_CHARS: usize = 240_000;
const MAX_JSON_STRING_CHARS: usize = 24_000;
const MAX_BLOB_PREVIEW_CHARS: usize = 900;
const MAX_TOOL_HISTORY_ITEMS: usize = 12;
const MAX_ARRAY_ITEMS: usize = 80;
const MAX_OBJECT_FIELDS: usize = 120;
const MAX_JSON_DEPTH: usize = 10;
const INTERNAL_TOOL_CONTEXT_BEGIN: &str = "<agentark_internal_tool_context>";
const INTERNAL_TOOL_CONTEXT_END: &str = "</agentark_internal_tool_context>";
const LEGACY_TOOL_CALL_CONTEXT_HEADER: &str =
    "Previous tool call context for interpreting the following tool results:";
const LEGACY_TOOL_RESULT_HEADER: &str = "Tool result for";

pub(crate) fn wrap_internal_tool_context(body: &str) -> String {
    format!(
        "{INTERNAL_TOOL_CONTEXT_BEGIN}\n{}\n{INTERNAL_TOOL_CONTEXT_END}",
        body.trim()
    )
}

pub(crate) fn strip_internal_tool_transcript(input: &str) -> String {
    strip_internal_tool_transcript_impl(input, false, true)
}

pub(crate) fn strip_internal_tool_transcript_preserve_spacing(input: &str) -> String {
    strip_internal_tool_transcript_impl(input, false, false)
}

pub(crate) struct InternalToolTranscriptStreamFilter {
    pending: String,
}

impl InternalToolTranscriptStreamFilter {
    pub(crate) fn new() -> Self {
        Self {
            pending: String::new(),
        }
    }

    pub(crate) fn feed(&mut self, chunk: &str) -> String {
        if chunk.is_empty() {
            return String::new();
        }
        self.pending.push_str(chunk);
        drain_stream_internal_tool_transcript(&mut self.pending, false)
    }

    pub(crate) fn finish(&mut self) -> String {
        drain_stream_internal_tool_transcript(&mut self.pending, true)
    }
}

pub(crate) fn sanitize_prompt_text(input: &str) -> String {
    let normalized = normalize_prompt_text(input);
    if normalized.trim().is_empty() {
        return String::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(&normalized) {
        let sanitized = sanitize_json_value(&value, None, 0);
        return serde_json::to_string(&sanitized).unwrap_or_else(|_| {
            truncate_text(
                &compact_large_text_payloads(&normalized),
                MAX_PROMPT_TEXT_CHARS,
            )
        });
    }
    truncate_text(
        &compact_large_text_payloads(&normalized),
        MAX_PROMPT_TEXT_CHARS,
    )
}

pub(crate) fn sanitize_conversation_history(
    history: &[ConversationMessage],
) -> Vec<ConversationMessage> {
    history
        .iter()
        .filter_map(|message| {
            let role = message.role.trim();
            if !is_supported_message_role(role) {
                return None;
            }
            Some(ConversationMessage {
                role: role.to_string(),
                content: sanitize_prompt_text(&message.content),
                _timestamp: message._timestamp,
            })
        })
        .collect()
}

fn normalize_prompt_text(input: &str) -> String {
    let input = input.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(input.len());
    let mut blank_run = 0usize;
    for line in input.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            blank_run = blank_run.saturating_add(1);
            if blank_run <= 2 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        out.push_str(trimmed_end);
        out.push('\n');
    }
    out.trim().to_string()
}

fn drain_stream_internal_tool_transcript(pending: &mut String, finish: bool) -> String {
    let mut output = String::new();
    loop {
        if pending.is_empty() {
            break;
        }
        let Some(start) = find_internal_tool_block_start(pending) else {
            let keep = if finish {
                0
            } else {
                longest_internal_marker_prefix_suffix_len(pending)
            };
            let emit_len = pending.len().saturating_sub(keep);
            if emit_len > 0 {
                output.push_str(&pending[..emit_len]);
                pending.drain(..emit_len);
            }
            break;
        };
        if start > 0 {
            output.push_str(&pending[..start]);
            pending.drain(..start);
            continue;
        }
        if let Some(end) = internal_tool_block_end(pending) {
            pending.drain(..end);
            while pending.starts_with('\n') || pending.starts_with('\r') {
                pending.drain(..1);
            }
            continue;
        }
        if finish {
            pending.clear();
        }
        break;
    }
    output
}

fn longest_internal_marker_prefix_suffix_len(text: &str) -> usize {
    [
        INTERNAL_TOOL_CONTEXT_BEGIN,
        LEGACY_TOOL_CALL_CONTEXT_HEADER,
        LEGACY_TOOL_RESULT_HEADER,
    ]
    .into_iter()
    .flat_map(|marker| {
        text.char_indices().filter_map(move |(idx, _)| {
            let suffix = &text[idx..];
            (!suffix.is_empty() && marker.starts_with(suffix)).then_some(text.len() - idx)
        })
    })
    .max()
    .unwrap_or(0)
}

fn strip_internal_tool_transcript_impl(input: &str, streaming: bool, normalize: bool) -> String {
    let mut remaining = input;
    let mut output = String::with_capacity(input.len());
    while !remaining.is_empty() {
        let Some(start) = find_internal_tool_block_start(remaining) else {
            output.push_str(remaining);
            break;
        };
        output.push_str(&remaining[..start]);
        let block = &remaining[start..];
        match internal_tool_block_end(block) {
            Some(end) => {
                remaining = trim_leading_block_separator(&block[end..]);
            }
            None if streaming => {
                break;
            }
            None => {
                break;
            }
        }
    }
    if normalize {
        normalize_visible_text_after_internal_strip(&output)
    } else {
        output
    }
}

fn find_internal_tool_block_start(input: &str) -> Option<usize> {
    let tagged_or_context = [INTERNAL_TOOL_CONTEXT_BEGIN, LEGACY_TOOL_CALL_CONTEXT_HEADER]
        .into_iter()
        .filter_map(|needle| input.find(needle))
        .min();
    [tagged_or_context, find_legacy_tool_result_start(input)]
        .into_iter()
        .flatten()
        .min()
}

fn find_legacy_tool_result_start(input: &str) -> Option<usize> {
    let mut offset = 0usize;
    while offset < input.len() {
        let Some(relative) = input[offset..].find(LEGACY_TOOL_RESULT_HEADER) else {
            return None;
        };
        let absolute = offset + relative;
        if legacy_tool_result_header_is_internal(&input[absolute..]) {
            return Some(absolute);
        }
        offset = absolute + LEGACY_TOOL_RESULT_HEADER.len();
    }
    None
}

fn legacy_tool_result_header_is_internal(block: &str) -> bool {
    let header = block.lines().next().unwrap_or_default();
    header.contains("tool_call")
}

fn internal_tool_block_end(block: &str) -> Option<usize> {
    if block.starts_with(INTERNAL_TOOL_CONTEXT_BEGIN) {
        return block
            .find(INTERNAL_TOOL_CONTEXT_END)
            .map(|idx| idx + INTERNAL_TOOL_CONTEXT_END.len());
    }
    if block.starts_with(LEGACY_TOOL_CALL_CONTEXT_HEADER) {
        return legacy_tool_call_context_end(block);
    }
    if block.starts_with(LEGACY_TOOL_RESULT_HEADER) {
        return legacy_tool_result_end(block);
    }
    None
}

fn legacy_tool_call_context_end(block: &str) -> Option<usize> {
    let mut offset = 0usize;
    let mut consumed_header = false;
    for segment in block.split_inclusive('\n') {
        let line = segment.trim_end_matches(|ch| ch == '\r' || ch == '\n');
        let trimmed = line.trim();
        let next_offset = offset + segment.len();
        if !consumed_header {
            consumed_header = true;
            offset = next_offset;
            continue;
        }
        if trimmed.is_empty() || legacy_tool_context_line_is_internal(line) {
            offset = next_offset;
            continue;
        }
        return Some(offset);
    }
    if consumed_header {
        Some(block.len())
    } else {
        None
    }
}

fn legacy_tool_context_line_is_internal(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- `tool_call")
        || trimmed.starts_with("`tool_call")
        || trimmed.starts_with("tool_call")
        || trimmed.starts_with("called ")
}

fn legacy_tool_result_end(block: &str) -> Option<usize> {
    let header_end = block.find('\n').map(|idx| idx + 1).unwrap_or(block.len());
    let body = &block[header_end..];
    let body_trimmed = body.trim_start();
    let whitespace_before_body = body.len().saturating_sub(body_trimmed.len());
    if body_trimmed.starts_with('{') {
        return balanced_json_object_end(body_trimmed)
            .map(|json_end| header_end + whitespace_before_body + json_end);
    }
    if body_trimmed.starts_with('[') {
        return balanced_json_array_end(body_trimmed)
            .map(|json_end| header_end + whitespace_before_body + json_end);
    }
    Some(header_end)
}

fn balanced_json_object_end(text: &str) -> Option<usize> {
    balanced_json_end(text, b'{', b'}')
}

fn balanced_json_array_end(text: &str) -> Option<usize> {
    balanced_json_end(text, b'[', b']')
}

fn balanced_json_end(text: &str, open: u8, close: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.first().copied() != Some(open) {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
            continue;
        }
        if byte == open {
            depth = depth.saturating_add(1);
            continue;
        }
        if byte == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(idx + 1);
            }
        }
    }
    None
}

fn trim_leading_block_separator(text: &str) -> &str {
    let mut remaining = text;
    while remaining.starts_with('\n') || remaining.starts_with('\r') {
        remaining = &remaining[1..];
    }
    remaining
}

fn normalize_visible_text_after_internal_strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0usize;
    for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        if line.trim().is_empty() {
            blank_run = blank_run.saturating_add(1);
            if blank_run <= 2 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

fn sanitize_json_value(value: &Value, key: Option<&str>, depth: usize) -> Value {
    if depth >= MAX_JSON_DEPTH {
        return summarize_json_boundary(value);
    }
    match value {
        Value::Object(map) => sanitize_json_object(map, depth),
        Value::Array(items) => sanitize_json_array(items, key, depth),
        Value::String(text) => sanitize_json_string(text, key),
        Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
    }
}

fn sanitize_json_object(map: &Map<String, Value>, depth: usize) -> Value {
    if is_message_like_object(map) {
        return sanitize_message_like_object(map, depth);
    }

    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let omitted = keys.len().saturating_sub(MAX_OBJECT_FIELDS);
    let mut out = Map::new();
    for key in keys.into_iter().take(MAX_OBJECT_FIELDS) {
        if let Some(value) = map.get(&key) {
            out.insert(
                key.clone(),
                sanitize_json_value(value, Some(&key), depth + 1),
            );
        }
    }
    if omitted > 0 {
        out.insert(
            "_sanitizer_omitted_fields".to_string(),
            Value::Number(omitted.into()),
        );
    }
    Value::Object(out)
}

fn sanitize_json_array(items: &[Value], key: Option<&str>, depth: usize) -> Value {
    if key.is_some_and(is_tool_history_key) {
        return sanitize_tool_history_array(items, depth);
    }

    let message_like = items
        .iter()
        .filter_map(|item| item.as_object())
        .filter(|object| is_message_like_object(object))
        .count();
    if message_like > 0 {
        let mut selected = items
            .iter()
            .filter_map(|item| item.as_object())
            .filter_map(|object| sanitize_message_array_item(object, depth + 1))
            .rev()
            .take(MAX_ARRAY_ITEMS)
            .collect::<Vec<_>>();
        selected.reverse();
        return Value::Array(selected);
    }

    let omitted = items.len().saturating_sub(MAX_ARRAY_ITEMS);
    let mut out = items
        .iter()
        .take(MAX_ARRAY_ITEMS)
        .map(|item| sanitize_json_value(item, None, depth + 1))
        .collect::<Vec<_>>();
    if omitted > 0 {
        out.push(serde_json::json!({
            "_sanitizer_omitted_items": omitted
        }));
    }
    Value::Array(out)
}

fn sanitize_message_array_item(map: &Map<String, Value>, depth: usize) -> Option<Value> {
    let role = map.get("role").and_then(Value::as_str).unwrap_or_default();
    if !is_supported_message_role(role) {
        return None;
    }
    Some(sanitize_message_like_object(map, depth))
}

fn sanitize_message_like_object(map: &Map<String, Value>, depth: usize) -> Value {
    let role = map.get("role").and_then(Value::as_str).unwrap_or_default();
    if is_tool_or_function_role(role) && !has_tool_pairing_marker(map) {
        return serde_json::json!({
            "role": role.trim(),
            "content": compact_payload_summary(map.get("content")),
            "sanitized": {
                "reason": "orphan_tool_result",
                "fields": map.len()
            }
        });
    }
    sanitize_json_object_fields(map, depth)
}

fn sanitize_json_object_fields(map: &Map<String, Value>, depth: usize) -> Value {
    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let omitted = keys.len().saturating_sub(MAX_OBJECT_FIELDS);
    let mut out = Map::new();
    for key in keys.into_iter().take(MAX_OBJECT_FIELDS) {
        if let Some(value) = map.get(&key) {
            out.insert(
                key.clone(),
                sanitize_json_value(value, Some(&key), depth + 1),
            );
        }
    }
    if omitted > 0 {
        out.insert(
            "_sanitizer_omitted_fields".to_string(),
            Value::Number(omitted.into()),
        );
    }
    Value::Object(out)
}

fn sanitize_tool_history_array(items: &[Value], depth: usize) -> Value {
    let mut selected = items
        .iter()
        .rev()
        .take(MAX_TOOL_HISTORY_ITEMS)
        .map(|item| sanitize_json_value(item, None, depth + 1))
        .collect::<Vec<_>>();
    selected.reverse();
    let omitted = items.len().saturating_sub(selected.len());
    if omitted > 0 {
        selected.insert(
            0,
            serde_json::json!({
                "status": "older_tool_history_omitted",
                "omitted_entries": omitted,
                "reason": "prompt_context_sanitized"
            }),
        );
    }
    Value::Array(selected)
}

fn sanitize_json_string(text: &str, key: Option<&str>) -> Value {
    if key.is_some_and(is_large_payload_key) || looks_like_large_encoded_payload(text) {
        return serde_json::json!({
            "sanitized": true,
            "kind": "large_payload",
            "original_chars": text.chars().count(),
            "preview": truncate_text(text, MAX_BLOB_PREVIEW_CHARS)
        });
    }
    if text.chars().count() <= MAX_JSON_STRING_CHARS {
        return Value::String(text.trim().to_string());
    }
    Value::String(truncate_middle_text(text, MAX_JSON_STRING_CHARS))
}

fn compact_large_text_payloads(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(MAX_PROMPT_TEXT_CHARS));
    let mut token = String::new();
    let flush_token = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if looks_like_large_encoded_payload(token.as_str()) {
            out.push_str("[sanitized_large_payload:");
            out.push_str(&token.chars().count().to_string());
            out.push_str(" chars]");
        } else if token.chars().count() > MAX_JSON_STRING_CHARS {
            out.push_str(&truncate_middle_text(token.as_str(), MAX_JSON_STRING_CHARS));
        } else {
            out.push_str(token.as_str());
        }
        token.clear();
    };

    for ch in input.chars() {
        if ch.is_whitespace() {
            flush_token(&mut token, &mut out);
            out.push(ch);
        } else {
            token.push(ch);
        }
        if out.chars().count() >= MAX_PROMPT_TEXT_CHARS {
            break;
        }
    }
    flush_token(&mut token, &mut out);
    truncate_text(out.trim(), MAX_PROMPT_TEXT_CHARS)
}

fn summarize_json_boundary(value: &Value) -> Value {
    match value {
        Value::Object(map) => serde_json::json!({
            "sanitized": true,
            "kind": "object_depth_boundary",
            "fields": map.len()
        }),
        Value::Array(items) => serde_json::json!({
            "sanitized": true,
            "kind": "array_depth_boundary",
            "items": items.len()
        }),
        Value::String(text) => sanitize_json_string(text, None),
        other => other.clone(),
    }
}

fn compact_payload_summary(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => Value::String(truncate_text(text, MAX_BLOB_PREVIEW_CHARS)),
        Some(Value::Object(map)) => serde_json::json!({"object_fields": map.len()}),
        Some(Value::Array(items)) => serde_json::json!({"array_items": items.len()}),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

fn is_message_like_object(map: &Map<String, Value>) -> bool {
    map.get("role").is_some() && map.get("content").is_some()
}

fn is_supported_message_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "system" | "user" | "assistant" | "tool" | "function"
    )
}

fn is_tool_or_function_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "tool" | "function"
    )
}

fn has_tool_pairing_marker(map: &Map<String, Value>) -> bool {
    map.get("tool_call_id").and_then(Value::as_str).is_some()
        || map.get("name").and_then(Value::as_str).is_some()
        || map.get("tool_name").and_then(Value::as_str).is_some()
}

fn is_tool_history_key(key: &str) -> bool {
    normalize_key(key) == "toolhistory"
}

fn is_large_payload_key(key: &str) -> bool {
    let key = normalize_key(key);
    key.contains("screenshot")
        || key.contains("base64")
        || key.contains("dataurl")
        || key.contains("blob")
        || key.contains("bytes")
        || key.contains("rawimage")
        || key.contains("binary")
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn looks_like_large_encoded_payload(text: &str) -> bool {
    let trimmed = text.trim();
    let len = trimmed.chars().count();
    if len < 4_096 {
        return false;
    }
    if trimmed.starts_with("data:") {
        return true;
    }
    let sample = trimmed.chars().take(8_192).collect::<String>();
    let encoded_chars = sample
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '+' | '/' | '=' | '-' | '_'))
        .count();
    let whitespace_chars = sample.chars().filter(|ch| ch.is_whitespace()).count();
    encoded_chars.saturating_mul(100) / sample.chars().count().max(1) >= 92
        && whitespace_chars.saturating_mul(100) / sample.chars().count().max(1) <= 2
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect()
    }
}

fn truncate_middle_text(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    let head = max_chars.saturating_mul(2) / 3;
    let tail = max_chars.saturating_sub(head).saturating_sub(80);
    let head_text = text.chars().take(head).collect::<String>();
    let tail_text = text
        .chars()
        .skip(len.saturating_sub(tail))
        .collect::<String>();
    format!(
        "{}...[sanitized {} omitted chars]...{}",
        head_text,
        len.saturating_sub(head).saturating_sub(tail),
        tail_text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_json_object_order_and_trims_payloads() {
        let raw = serde_json::json!({
            "b": "x",
            "a": {
                "screenshot_base64": "A".repeat(5000)
            }
        })
        .to_string();

        let sanitized = sanitize_prompt_text(&raw);

        assert!(sanitized.starts_with("{\"a\""));
        assert!(sanitized.contains("\"sanitized\":true"));
        assert!(sanitized.contains("\"original_chars\":5000"));
    }

    #[test]
    fn keeps_newest_tool_history_entries() {
        let raw = serde_json::json!({
            "tool_history": (0..20).map(|index| serde_json::json!({"tool": "t", "result": index})).collect::<Vec<_>>()
        })
        .to_string();

        let sanitized: Value = serde_json::from_str(&sanitize_prompt_text(&raw)).unwrap();
        let history = sanitized
            .get("tool_history")
            .and_then(Value::as_array)
            .unwrap();

        assert_eq!(history.len(), MAX_TOOL_HISTORY_ITEMS + 1);
        assert_eq!(
            history
                .first()
                .unwrap()
                .get("status")
                .and_then(Value::as_str),
            Some("older_tool_history_omitted")
        );
    }

    #[test]
    fn drops_unsupported_history_roles_and_compacts_orphan_tool_result() {
        let history = vec![
            ConversationMessage {
                role: "debug".to_string(),
                content: "internal".to_string(),
                _timestamp: chrono::Utc::now(),
            },
            ConversationMessage {
                role: "tool".to_string(),
                content: serde_json::json!({"role":"tool","content":"orphan"}).to_string(),
                _timestamp: chrono::Utc::now(),
            },
        ];

        let sanitized = sanitize_conversation_history(&history);

        assert_eq!(sanitized.len(), 1);
        assert!(sanitized[0].content.contains("orphan_tool_result"));
    }

    #[test]
    fn strips_tagged_internal_tool_context_from_visible_text() {
        let text = format!(
            "Working.\n\n{}\n\nStill working.",
            wrap_internal_tool_context("tool_result:\n{\"ok\":true}")
        );

        let visible = strip_internal_tool_transcript(&text);

        assert_eq!(visible, "Working.\n\nStill working.");
    }

    #[test]
    fn strips_legacy_internal_tool_scaffold_without_removing_public_prose() {
        let text = "I'll build it.\n\nPrevious tool call context for interpreting the following tool results:\n\n    tool_call_3 called file_write with chars: 4725; omitted: true; path: expense-tracker/server.js.\n\nLet me continue.\n\nTool result for tool_call_4:\n{\"ok\":true,\"message\":\"Saved managed file App.css.\"}\n\nDone.";

        let visible = strip_internal_tool_transcript(text);

        assert!(visible.contains("I'll build it."));
        assert!(visible.contains("Let me continue."));
        assert!(visible.contains("Done."));
        assert!(!visible.contains("tool_call_3"));
        assert!(!visible.contains("Saved managed file App.css"));
    }

    #[test]
    fn stream_filter_handles_split_internal_markers() {
        let mut filter = InternalToolTranscriptStreamFilter::new();
        let mut visible = String::new();
        visible.push_str(&filter.feed("Before.\n<agentark_internal_tool"));
        visible
            .push_str(&filter.feed("_context>\nsecret\n</agentark_internal_tool_context>\nAfter."));
        visible.push_str(&filter.finish());

        assert_eq!(visible, "Before.\nAfter.");
    }
}
