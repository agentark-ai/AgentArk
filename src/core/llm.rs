//! LLM client for agent reasoning

use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

use crate::core::agent::{ConversationMessage, StreamEvent};
use crate::core::llm_provider::{
    display_openai_base_url, force_refresh_codex_cli_api_key, is_codex_cli_base_url,
    openai_provider_label, resolve_openai_request_config, ResolvedOpenAiRequestConfig,
};

// OpenRouter enforces request affordability against the declared output budget.
// Cap only that provider by default so other OpenAI-compatible backends remain
// free to use their own native limits.
// No artificial output cap — let the model use its full native output limit.
// Previously set to 1024, which truncated app_deploy payloads mid-JSON.

/// Supported LLM providers
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum LlmProvider {
    Anthropic {
        api_key: String,
        model: String,
    },
    OpenAI {
        api_key: String,
        model: String,
        base_url: Option<String>,
    },
    Ollama {
        base_url: String,
        model: String,
    },
}

impl std::fmt::Debug for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic { model, .. } => f
                .debug_struct("LlmProvider::Anthropic")
                .field("api_key", &"[REDACTED]")
                .field("model", model)
                .finish(),
            Self::OpenAI {
                model, base_url, ..
            } => f
                .debug_struct("LlmProvider::OpenAI")
                .field("api_key", &"[REDACTED]")
                .field("model", model)
                .field("base_url", base_url)
                .finish(),
            Self::Ollama { base_url, model } => f
                .debug_struct("LlmProvider::Ollama")
                .field("base_url", base_url)
                .field("model", model)
                .finish(),
        }
    }
}

/// Attempt to repair truncated JSON by closing unclosed braces, brackets, and strings.
/// Returns `Some(Value)` if repair produces valid JSON, `None` otherwise.
fn repair_truncated_json(raw: &str) -> Option<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Only attempt repair on object-like or array-like starts.
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }

    // Walk through the string tracking open delimiters and string state.
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escape_next = false;

    for ch in trimmed.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.last() == Some(&ch) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }

    // Nothing to close — original should have parsed fine.
    if stack.is_empty() && !in_string {
        return None;
    }

    let mut repaired = trimmed.to_string();

    // Close open string if needed.
    if in_string {
        repaired.push('"');
    }

    // Trim any trailing comma or colon (incomplete key-value).
    let end = repaired.trim_end();
    if end.ends_with(',') || end.ends_with(':') {
        repaired = end.trim_end_matches([',', ':']).to_string();
    }

    // Close remaining open delimiters in reverse order.
    for closer in stack.into_iter().rev() {
        repaired.push(closer);
    }

    serde_json::from_str(&repaired).ok()
}

const MAX_LLM_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

async fn read_response_bytes_limited(
    response: reqwest::Response,
    provider: &str,
) -> Result<Vec<u8>> {
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_LLM_RESPONSE_BYTES as u64 {
            return Err(anyhow!(
                "{} response exceeded {} byte limit (content-length={})",
                provider,
                MAX_LLM_RESPONSE_BYTES,
                content_length
            ));
        }
    }

    let mut total = 0usize;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total = total.saturating_add(chunk.len());
        if total > MAX_LLM_RESPONSE_BYTES {
            return Err(anyhow!(
                "{} response exceeded {} byte limit",
                provider,
                MAX_LLM_RESPONSE_BYTES
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn read_response_text_limited(response: reqwest::Response, provider: &str) -> Result<String> {
    let bytes = read_response_bytes_limited(response, provider).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn read_response_json_limited<T: DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> Result<T> {
    let bytes = read_response_bytes_limited(response, provider).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn tool_argument_progress_step(tool_name: &str) -> usize {
    if tool_name.trim().eq_ignore_ascii_case("app_deploy") {
        250
    } else {
        800
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftFilePreview {
    file: String,
    content_snapshot: String,
    line_count: usize,
    total_lines: Option<usize>,
    done: bool,
}

fn parse_partial_tool_arguments(raw: &str) -> Option<(serde_json::Value, bool)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some((parsed, true));
    }
    repair_truncated_json(trimmed).map(|parsed| (parsed, false))
}

fn parse_tool_arguments_with_self_heal(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Null;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(parsed) => parsed,
        Err(_) => repair_truncated_json(trimmed)
            .unwrap_or_else(|| serde_json::Value::String(raw.to_string())),
    }
}

fn collect_app_deploy_partial_files(
    parsed: &serde_json::Value,
    done: bool,
) -> Vec<DraftFilePreview> {
    let Some(obj) = parsed.as_object() else {
        return Vec::new();
    };

    let mut out = Vec::new();

    if let Some(files) = obj.get("files").and_then(|value| value.as_object()) {
        for (file, value) in files {
            let Some(content) = value.as_str() else {
                continue;
            };
            if file.trim().is_empty() || content.is_empty() {
                continue;
            }
            let line_count = content.lines().count().max(1);
            out.push(DraftFilePreview {
                file: file.trim().to_string(),
                content_snapshot: content.to_string(),
                line_count,
                total_lines: done.then_some(line_count),
                done,
            });
        }
        return out;
    }

    if let Some(files) = obj.get("files").and_then(|value| value.as_array()) {
        for entry in files {
            let Some(file_obj) = entry.as_object() else {
                continue;
            };
            let file = file_obj
                .get("path")
                .and_then(|value| value.as_str())
                .or_else(|| file_obj.get("name").and_then(|value| value.as_str()))
                .unwrap_or("")
                .trim();
            let content = file_obj
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if file.is_empty() || content.is_empty() {
                continue;
            }
            let line_count = content.lines().count().max(1);
            out.push(DraftFilePreview {
                file: file.to_string(),
                content_snapshot: content.to_string(),
                line_count,
                total_lines: done.then_some(line_count),
                done,
            });
        }
    }

    out
}

fn collect_file_write_partial_file(
    parsed: &serde_json::Value,
    done: bool,
) -> Option<DraftFilePreview> {
    let obj = parsed.as_object()?;
    let file = obj
        .get("path")
        .and_then(|value| value.as_str())
        .or_else(|| obj.get("file_path").and_then(|value| value.as_str()))
        .or_else(|| obj.get("filename").and_then(|value| value.as_str()))
        .unwrap_or("")
        .trim();
    let content = obj
        .get("content")
        .and_then(|value| value.as_str())
        .or_else(|| obj.get("text").and_then(|value| value.as_str()))
        .or_else(|| obj.get("body").and_then(|value| value.as_str()))
        .unwrap_or("");
    if file.is_empty() || content.is_empty() {
        return None;
    }
    let line_count = content.lines().count().max(1);
    Some(DraftFilePreview {
        file: file.to_string(),
        content_snapshot: content.to_string(),
        line_count,
        total_lines: done.then_some(line_count),
        done,
    })
}

fn extract_partial_draft_files(tool_name: &str, raw_args: &str) -> Vec<DraftFilePreview> {
    let Some((parsed, done)) = parse_partial_tool_arguments(raw_args) else {
        return Vec::new();
    };

    if tool_name.trim().eq_ignore_ascii_case("app_deploy") {
        return collect_app_deploy_partial_files(&parsed, done);
    }
    if tool_name.trim().eq_ignore_ascii_case("file_write") {
        return collect_file_write_partial_file(&parsed, done)
            .into_iter()
            .collect();
    }
    Vec::new()
}

fn tool_argument_phase(tool_name: &str) -> (&'static str, &'static str) {
    if tool_name.trim().eq_ignore_ascii_case("app_deploy") {
        ("generating_files", "Generating files")
    } else if tool_name.trim().eq_ignore_ascii_case("file_write") {
        ("writing_files", "Drafting file")
    } else {
        ("preparing_tool", "Preparing tool")
    }
}

#[derive(Clone, Copy)]
enum ModelRequestMode {
    PrimaryAssistantTurn,
    Helper,
}

fn sanitize_model_request_bundle(
    mode: ModelRequestMode,
    system_prompt: &str,
    user_message: &str,
    history: &[ConversationMessage],
    policy: &crate::security::ModelPrivacyConfig,
    allow_sensitive_context: bool,
) -> (String, String, Vec<ConversationMessage>) {
    let system_context = match mode {
        ModelRequestMode::PrimaryAssistantTurn => crate::security::ModelInputContext::SystemPrompt,
        ModelRequestMode::Helper => crate::security::ModelInputContext::InternalHelperPrompt,
    };
    let user_context = match mode {
        ModelRequestMode::PrimaryAssistantTurn => {
            crate::security::ModelInputContext::CurrentUserMessage
        }
        ModelRequestMode::Helper => crate::security::ModelInputContext::InternalHelperPrompt,
    };

    (
        sanitize_model_request_text(
            system_prompt,
            system_context,
            policy,
            allow_sensitive_context,
        ),
        sanitize_model_request_text(user_message, user_context, policy, allow_sensitive_context),
        sanitize_model_request_history(history, policy, allow_sensitive_context),
    )
}

fn sanitize_model_request_text(
    text: &str,
    context: crate::security::ModelInputContext,
    policy: &crate::security::ModelPrivacyConfig,
    allow_sensitive_context: bool,
) -> String {
    let result =
        crate::security::sanitize_model_input_text(text, policy, context, allow_sensitive_context);
    crate::security::render_model_input_fallback(&result, context)
}

fn sanitize_model_request_history(
    history: &[ConversationMessage],
    policy: &crate::security::ModelPrivacyConfig,
    allow_sensitive_context: bool,
) -> Vec<ConversationMessage> {
    history
        .iter()
        .map(|message| ConversationMessage {
            role: message.role.clone(),
            content: sanitize_model_request_text(
                &message.content,
                crate::security::ModelInputContext::HistoryMessage,
                policy,
                allow_sensitive_context,
            ),
            _timestamp: message._timestamp,
        })
        .collect()
}

async fn emit_stream_tool_progress(
    token_tx: &Sender<StreamEvent>,
    name: &str,
    content: String,
    payload: serde_json::Value,
) {
    let _ = token_tx
        .send(StreamEvent::ToolProgress {
            name: name.to_string(),
            content,
            payload: Some(payload),
        })
        .await;
}

fn queue_stream_event(token_tx: &Sender<StreamEvent>, event: StreamEvent) {
    match token_tx.try_send(event) {
        Ok(_) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
            let fallback_tx = token_tx.clone();
            tokio::spawn(async move {
                let _ = fallback_tx.send(event).await;
            });
        }
    }
}

async fn emit_argument_phase_status(
    token_tx: &Sender<StreamEvent>,
    tool_name: &str,
    detail: String,
    elapsed_secs: u64,
) {
    let (phase, label) = tool_argument_phase(tool_name);
    emit_stream_tool_progress(
        token_tx,
        tool_name,
        detail.clone(),
        serde_json::json!({
            "kind": "phase_status",
            "phase": phase,
            "label": label,
            "detail": detail,
            "elapsed_secs": elapsed_secs,
            "stream_key": format!("phase-status:{}:{}", tool_name, phase),
        }),
    )
    .await;
}

async fn emit_partial_draft_file_previews(
    token_tx: &Sender<StreamEvent>,
    tool_name: &str,
    raw_args: &str,
    emitted_snapshots: &mut HashMap<String, (String, bool)>,
) {
    for preview in extract_partial_draft_files(tool_name, raw_args) {
        let stream_key = format!("draft-file:{}:{}", tool_name, preview.file);
        let previous = emitted_snapshots
            .get(&stream_key)
            .cloned()
            .unwrap_or_else(|| (String::new(), false));
        if preview.content_snapshot.len() <= previous.0.len() && (!preview.done || previous.1) {
            continue;
        }
        let delta = preview
            .content_snapshot
            .strip_prefix(&previous.0)
            .map(str::to_string)
            .unwrap_or_else(|| preview.content_snapshot.clone());
        let emit_snapshot = previous.0.is_empty() || delta == preview.content_snapshot;
        emitted_snapshots.insert(
            stream_key.clone(),
            (preview.content_snapshot.clone(), preview.done),
        );
        let file_name = preview.file.clone();

        let mut payload = serde_json::Map::new();
        payload.insert("kind".to_string(), serde_json::json!("draft_file"));
        payload.insert("file".to_string(), serde_json::json!(file_name.clone()));
        payload.insert(
            "phase".to_string(),
            serde_json::json!(tool_argument_phase(tool_name).0),
        );
        payload.insert("stream_key".to_string(), serde_json::json!(stream_key));
        payload.insert(
            if emit_snapshot {
                "content_snapshot".to_string()
            } else {
                "content_delta".to_string()
            },
            serde_json::json!(if emit_snapshot {
                preview.content_snapshot
            } else {
                delta
            }),
        );
        payload.insert("line".to_string(), serde_json::json!(preview.line_count));
        payload.insert("done".to_string(), serde_json::json!(preview.done));
        if let Some(total_lines) = preview.total_lines {
            payload.insert("total_lines".to_string(), serde_json::json!(total_lines));
        }

        emit_stream_tool_progress(
            token_tx,
            tool_name,
            format!("Drafting {}", file_name),
            serde_json::Value::Object(payload),
        )
        .await;
    }
}

fn normalize_openai_text_chunk(text: &str, trim: bool) -> Option<String> {
    if trim {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn extract_openai_text_from_value(value: &serde_json::Value, trim: bool) -> Option<String> {
    if let Some(text) = value.as_str() {
        return normalize_openai_text_chunk(text, trim);
    }

    if let Some(obj) = value.as_object() {
        if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
            if let Some(chunk) = normalize_openai_text_chunk(text, trim) {
                return Some(chunk);
            }
        }
        if let Some(content) = obj.get("content") {
            if let Some(text) = extract_openai_text_from_value(content, trim) {
                return Some(text);
            }
        }
    }

    if let Some(arr) = value.as_array() {
        let mut chunks: Vec<String> = Vec::new();
        for item in arr {
            if let Some(text) = item.as_str() {
                if let Some(chunk) = normalize_openai_text_chunk(text, trim) {
                    chunks.push(chunk);
                }
                continue;
            }

            let Some(obj) = item.as_object() else {
                continue;
            };
            let item_type = obj
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase());
            if let Some(t) = item_type.as_deref() {
                if t != "text"
                    && t != "input_text"
                    && t != "output_text"
                    && t != "content"
                    && t != "reasoning"
                {
                    continue;
                }
            }

            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                if let Some(chunk) = normalize_openai_text_chunk(text, trim) {
                    chunks.push(chunk);
                }
                continue;
            }
            if let Some(content) = obj.get("content") {
                if let Some(text) = extract_openai_text_from_value(content, trim) {
                    chunks.push(text);
                }
            }
        }
        if !chunks.is_empty() {
            return Some(if trim {
                chunks.join("\n")
            } else {
                chunks.concat()
            });
        }
    }

    None
}

fn extract_openai_message_text(value: &serde_json::Value) -> Option<String> {
    extract_openai_text_from_value(value, true)
}

fn extract_openai_delta_text(value: &serde_json::Value) -> Option<String> {
    extract_openai_text_from_value(value, false)
}

fn openai_responses_endpoint(config: &ResolvedOpenAiRequestConfig) -> String {
    format!("{}/responses", config.base_url.trim_end_matches('/'))
}

fn openai_responses_message(role: &str, content: &str) -> serde_json::Value {
    let normalized_role = match role {
        "assistant" => "assistant",
        "developer" => "developer",
        "system" => "developer",
        _ => "user",
    };
    let content_type = if normalized_role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    serde_json::json!({
        "type": "message",
        "role": normalized_role,
        "content": [{
            "type": content_type,
            "text": content,
        }],
    })
}

fn build_openai_responses_input(
    user_message: &str,
    history: &[ConversationMessage],
) -> Vec<serde_json::Value> {
    let mut input = Vec::new();
    for message in history
        .iter()
        .filter(|message| !(message.role == "user" && message.content == user_message))
    {
        let content = message.content.trim();
        if content.is_empty() {
            continue;
        }
        input.push(openai_responses_message(message.role.as_str(), content));
    }
    input.push(openai_responses_message("user", user_message));
    input
}

fn build_openai_responses_tools(actions: &[crate::actions::ActionDef]) -> Vec<serde_json::Value> {
    actions
        .iter()
        .map(|action| {
            serde_json::json!({
                "type": "function",
                "name": action.name,
                "description": action.description,
                "strict": false,
                "parameters": normalize_openai_tool_schema(&action.input_schema),
            })
        })
        .collect()
}

fn build_openai_responses_request(
    model: &str,
    system_prompt: &str,
    user_message: &str,
    history: &[ConversationMessage],
    actions: &[crate::actions::ActionDef],
    stream: bool,
) -> serde_json::Value {
    let tools = build_openai_responses_tools(actions);
    let mut request = serde_json::json!({
        "model": model,
        "instructions": system_prompt,
        "input": build_openai_responses_input(user_message, history),
        "stream": stream,
        "store": false,
    });
    if !tools.is_empty() {
        request["tools"] = serde_json::Value::Array(tools);
        request["tool_choice"] = serde_json::Value::String("auto".to_string());
        request["parallel_tool_calls"] = serde_json::Value::Bool(true);
    }
    request
}

fn openai_responses_tool_arguments(value: Option<&serde_json::Value>) -> serde_json::Value {
    match value {
        Some(serde_json::Value::String(raw)) => serde_json::from_str(raw)
            .unwrap_or_else(|_| serde_json::json!({ "_raw": raw })),
        Some(value) if value.is_object() || value.is_array() => value.clone(),
        Some(value) if value.is_null() => serde_json::json!({}),
        Some(value) => serde_json::json!({ "_raw": value }),
        None => serde_json::json!({}),
    }
}

fn openai_responses_usage(
    payload: &serde_json::Value,
    prompt_chars: usize,
    completion_chars: usize,
) -> Option<LlmTokenUsage> {
    let usage = payload.get("usage")?;
    let input_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| estimate_tokens_from_chars(prompt_chars));
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| estimate_tokens_from_chars(completion_chars));
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    Some(LlmTokenUsage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        estimated: false,
    })
}

fn collect_openai_responses_text_from_content(content: &serde_json::Value) -> String {
    let Some(blocks) = content.as_array() else {
        return extract_openai_message_text(content).unwrap_or_default();
    };
    let mut text = String::new();
    for block in blocks {
        let block_type = block.get("type").and_then(|value| value.as_str()).unwrap_or("");
        if matches!(block_type, "output_text" | "input_text" | "text")
            || block.get("text").is_some()
        {
            if let Some(chunk) = block.get("text").and_then(|value| value.as_str()) {
                text.push_str(chunk);
            }
        }
    }
    text
}

fn parse_openai_responses_payload(
    payload: &serde_json::Value,
    prompt_chars: usize,
    fallback_content: &str,
    provider: &str,
    model: &str,
) -> Result<LlmResponse> {
    if let Some(error) = payload.get("error").filter(|value| !value.is_null()) {
        return Err(anyhow!("OpenAI Subscription returned an error payload: {}", error));
    }

    let mut content = payload
        .get("output_text")
        .and_then(extract_openai_message_text)
        .unwrap_or_default();
    let response_level_text_present = !content.is_empty();
    let mut reasoning: Option<String> = None;
    let mut tool_calls = Vec::new();

    if let Some(output) = payload.get("output").and_then(|value| value.as_array()) {
        for item in output {
            match item.get("type").and_then(|value| value.as_str()).unwrap_or("") {
                "message" => {
                    let item_text = item
                        .get("content")
                        .map(collect_openai_responses_text_from_content)
                        .unwrap_or_default();
                    if !response_level_text_present && !item_text.is_empty() {
                        content.push_str(&item_text);
                    }
                }
                "function_call" => {
                    let name = item
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim();
                    if name.is_empty() {
                        continue;
                    }
                    let fallback_id = format!("call_{}", tool_calls.len() + 1);
                    let id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or(fallback_id.as_str())
                        .to_string();
                    tool_calls.push(ToolCall {
                        id,
                        name: name.to_string(),
                        arguments: openai_responses_tool_arguments(item.get("arguments")),
                    });
                }
                "reasoning" => {
                    if let Some(summary) = item.get("summary").and_then(|value| value.as_array()) {
                        let mut text = String::new();
                        for chunk in summary {
                            if let Some(value) = chunk.get("text").and_then(|value| value.as_str())
                            {
                                text.push_str(value);
                            }
                        }
                        if !text.trim().is_empty() {
                            reasoning.get_or_insert_with(String::new).push_str(&text);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if content.is_empty() && !fallback_content.is_empty() {
        content = fallback_content.to_string();
    }
    if content.is_empty() && tool_calls.is_empty() {
        if let Some(text) = extract_text_from_any_json(payload) {
            content = text;
        }
    }
    if content.is_empty() && tool_calls.is_empty() {
        return Err(anyhow!(
            "OpenAI Subscription response did not contain assistant text or tool calls"
        ));
    }

    let completion_chars = content.len()
        + tool_calls
            .iter()
            .map(|call| call.name.len() + call.arguments.to_string().len())
            .sum::<usize>();
    let usage = openai_responses_usage(payload, prompt_chars, completion_chars).or_else(|| {
        let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
        let completion_tokens = estimate_tokens_from_chars(completion_chars);
        Some(LlmTokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            estimated: true,
        })
    });

    Ok(LlmResponse {
        content,
        tool_calls,
        reasoning,
        usage,
        provider: provider.to_string(),
        model: model.to_string(),
    })
}

/// Normalize tool JSON Schema for OpenAI-compatible function calling.
/// OpenAI requires `items` to be present for every array schema.
fn normalize_openai_tool_schema(schema: &serde_json::Value) -> serde_json::Value {
    let mut normalized = if schema.is_object() {
        schema.clone()
    } else {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    };
    normalize_openai_tool_schema_in_place(&mut normalized, true);
    normalized
}

fn append_schema_description_note(
    map: &mut serde_json::Map<String, serde_json::Value>,
    note: impl AsRef<str>,
) {
    let note = note.as_ref().trim();
    if note.is_empty() {
        return;
    }
    let existing = map
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if existing.contains(note) {
        return;
    }
    let merged = if existing.is_empty() {
        note.to_string()
    } else if existing.ends_with('.') || existing.ends_with('!') || existing.ends_with('?') {
        format!("{} {}", existing, note)
    } else {
        format!("{}. {}", existing, note)
    };
    map.insert("description".to_string(), serde_json::Value::String(merged));
}

fn merge_required_keys_into_map(
    map: &mut serde_json::Map<String, serde_json::Value>,
    required_keys: impl IntoIterator<Item = String>,
) {
    let mut merged: Vec<String> = map
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    for key in required_keys {
        if !merged.iter().any(|existing| existing == &key) {
            merged.push(key);
        }
    }
    if !merged.is_empty() {
        map.insert(
            "required".to_string(),
            serde_json::Value::Array(merged.into_iter().map(serde_json::Value::String).collect()),
        );
    }
}

fn collect_branch_required_sets(branches: &[serde_json::Value]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for branch in branches {
        let Some(obj) = branch.as_object() else {
            continue;
        };
        let Some(required) = obj.get("required").and_then(|v| v.as_array()) else {
            continue;
        };
        let keys: Vec<String> = required
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect();
        if !keys.is_empty() && !out.iter().any(|existing| existing == &keys) {
            out.push(keys);
        }
    }
    out
}

fn describe_required_branch_sets(mode: &str, branches: &[serde_json::Value]) -> Option<String> {
    let required_sets = collect_branch_required_sets(branches);
    if required_sets.is_empty() {
        return None;
    }

    if mode == "allOf" {
        let mut keys = Vec::new();
        for set in required_sets {
            for key in set {
                if !keys.iter().any(|existing| existing == &key) {
                    keys.push(key);
                }
            }
        }
        if keys.is_empty() {
            None
        } else {
            Some(format!(
                "Include these keys when needed: {}.",
                keys.join(", ")
            ))
        }
    } else {
        let mut single_keys = Vec::new();
        let mut all_single = true;
        for set in &required_sets {
            if set.len() != 1 {
                all_single = false;
                break;
            }
            let key = set[0].clone();
            if !single_keys.iter().any(|existing| existing == &key) {
                single_keys.push(key);
            }
        }
        if all_single && !single_keys.is_empty() {
            Some(format!(
                "Provide at least one of these keys: {}.",
                single_keys.join(", ")
            ))
        } else {
            let groups = required_sets
                .iter()
                .map(|set| set.join(", "))
                .collect::<Vec<_>>()
                .join(" | ");
            Some(format!("Valid key groups: {}.", groups))
        }
    }
}

fn merge_branch_object_shapes_into_root(
    map: &mut serde_json::Map<String, serde_json::Value>,
    mode: &str,
    branches: &[serde_json::Value],
) {
    let mut merged_properties = map
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut all_of_required = Vec::new();

    for branch in branches {
        let Some(obj) = branch.as_object() else {
            continue;
        };
        if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
            for (name, child) in props {
                merged_properties
                    .entry(name.clone())
                    .or_insert_with(|| child.clone());
            }
        }
        if mode == "allOf" {
            if let Some(required) = obj.get("required").and_then(|v| v.as_array()) {
                for key in required.iter().filter_map(|item| item.as_str()) {
                    if !all_of_required.iter().any(|existing| existing == key) {
                        all_of_required.push(key.to_string());
                    }
                }
            }
        }
    }

    map.insert(
        "properties".to_string(),
        serde_json::Value::Object(merged_properties),
    );
    if mode == "allOf" {
        merge_required_keys_into_map(map, all_of_required);
    }
}

fn normalize_type_array_in_place(map: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(type_arr) = map.get("type").and_then(|v| v.as_array()) else {
        return;
    };
    let mut variants = Vec::new();
    for item in type_arr {
        if let Some(kind) = item.as_str() {
            let lower = kind.trim().to_ascii_lowercase();
            if !lower.is_empty() && !variants.iter().any(|existing| existing == &lower) {
                variants.push(lower);
            }
        }
    }
    if variants.is_empty() {
        map.remove("type");
        return;
    }
    let non_null: Vec<String> = variants
        .iter()
        .filter(|kind| kind.as_str() != "null")
        .cloned()
        .collect();
    if non_null.len() == 1 {
        map.insert(
            "type".to_string(),
            serde_json::Value::String(non_null[0].clone()),
        );
        if variants.len() > 1 {
            append_schema_description_note(
                map,
                "Null is also acceptable when omitted by the caller.",
            );
        }
        return;
    }

    map.remove("type");
    append_schema_description_note(
        map,
        format!("Allowed value types: {}.", variants.join(", ")),
    );
}

fn normalize_openai_tool_schema_in_place(node: &mut serde_json::Value, is_root: bool) {
    match node {
        serde_json::Value::Object(map) => {
            normalize_type_array_in_place(map);

            if is_root {
                if map.get("type").and_then(|v| v.as_str()) != Some("object") {
                    map.insert(
                        "type".to_string(),
                        serde_json::Value::String("object".to_string()),
                    );
                }

                let combinator_keys = ["allOf", "anyOf", "oneOf"];
                for key in combinator_keys {
                    let branches = map
                        .get(key)
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if branches.is_empty() {
                        continue;
                    }
                    merge_branch_object_shapes_into_root(map, key, &branches);
                    if let Some(note) = describe_required_branch_sets(key, &branches) {
                        append_schema_description_note(map, note);
                    }
                }

                if let Some(enum_values) = map.get("enum").and_then(|v| v.as_array()) {
                    let variants = enum_values
                        .iter()
                        .filter_map(|item| match item {
                            serde_json::Value::String(s) => Some(s.clone()),
                            serde_json::Value::Number(n) => Some(n.to_string()),
                            serde_json::Value::Bool(b) => Some(b.to_string()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if !variants.is_empty() {
                        append_schema_description_note(
                            map,
                            format!("Allowed values: {}.", variants.join(", ")),
                        );
                    }
                }

                if map.contains_key("not") {
                    append_schema_description_note(
                        map,
                        "Avoid excluded argument combinations; use a straightforward JSON object.",
                    );
                }

                for key in ["allOf", "anyOf", "oneOf", "not", "enum"] {
                    map.remove(key);
                }
                if !map
                    .get("properties")
                    .map(|value| value.is_object())
                    .unwrap_or(false)
                {
                    map.insert(
                        "properties".to_string(),
                        serde_json::Value::Object(serde_json::Map::new()),
                    );
                }
            }

            if map.get("type").and_then(|v| v.as_str()) == Some("array")
                && !map.contains_key("items")
            {
                map.insert("items".to_string(), serde_json::json!({}));
            }

            if let Some(props) = map.get_mut("properties").and_then(|v| v.as_object_mut()) {
                for (_name, child) in props.iter_mut() {
                    normalize_openai_tool_schema_in_place(child, false);
                }
            }
            if let Some(items) = map.get_mut("items") {
                normalize_openai_tool_schema_in_place(items, false);
            }
            if let Some(additional) = map.get_mut("additionalProperties") {
                normalize_openai_tool_schema_in_place(additional, false);
            }
            if let Some(defs) = map.get_mut("$defs").and_then(|v| v.as_object_mut()) {
                for (_name, child) in defs.iter_mut() {
                    normalize_openai_tool_schema_in_place(child, false);
                }
            }
            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(arr) = map.get_mut(key).and_then(|v| v.as_array_mut()) {
                    for child in arr.iter_mut() {
                        normalize_openai_tool_schema_in_place(child, false);
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr.iter_mut() {
                normalize_openai_tool_schema_in_place(child, false);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_partial_draft_files, json_contains_tool_call_indicators,
        normalize_openai_tool_schema, openai_stream_data_has_terminal_finish_reason,
        parse_openai_responses_payload, parse_partial_tool_arguments,
    };

    #[test]
    fn normalize_openai_tool_schema_removes_top_level_anyof_requirements() {
        let normalized = normalize_openai_tool_schema(&serde_json::json!({
            "type": "object",
            "properties": {
                "app_id": { "type": "string" },
                "query": { "type": "string" }
            },
            "anyOf": [
                { "required": ["app_id"] },
                { "required": ["query"] }
            ]
        }));

        assert_eq!(
            normalized.get("type").and_then(|v| v.as_str()),
            Some("object")
        );
        assert!(normalized
            .get("properties")
            .and_then(|v| v.as_object())
            .is_some());
        assert!(normalized.get("anyOf").is_none());
        let description = normalized
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(description.contains("Provide at least one of these keys: app_id, query."));
    }

    #[test]
    fn normalize_openai_tool_schema_merges_top_level_branch_properties() {
        let normalized = normalize_openai_tool_schema(&serde_json::json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": { "url": { "type": "string" } },
                    "required": ["url"]
                },
                {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            ]
        }));

        let properties = normalized
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("properties");
        assert!(properties.contains_key("url"));
        assert!(properties.contains_key("path"));
        assert!(normalized.get("anyOf").is_none());
        assert_eq!(
            normalized.get("type").and_then(|v| v.as_str()),
            Some("object")
        );
    }

    #[test]
    fn normalize_openai_tool_schema_rewrites_type_arrays_to_descriptive_shape() {
        let normalized = normalize_openai_tool_schema(&serde_json::json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "additionalProperties": {
                        "type": ["string", "number", "boolean"]
                    }
                }
            }
        }));

        let config = normalized
            .get("properties")
            .and_then(|v| v.get("config"))
            .and_then(|v| v.as_object())
            .expect("config object");
        let additional = config
            .get("additionalProperties")
            .and_then(|v| v.as_object())
            .expect("additionalProperties object");
        assert!(additional.get("type").is_none());
        let description = additional
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(description.contains("Allowed value types: string, number, boolean."));
    }

    #[test]
    fn json_contains_tool_call_indicators_detects_nested_tool_calls() {
        let payload = serde_json::json!({
            "choices": [
                {
                    "message": {
                        "tool_calls": [
                            { "id": "call-1", "type": "function" }
                        ]
                    }
                }
            ]
        });

        assert!(json_contains_tool_call_indicators(&payload));
    }

    #[test]
    fn openai_responses_parser_ignores_null_error_field() {
        let payload = serde_json::json!({
            "error": null,
            "output": [{
                "type": "message",
                "content": [{ "type": "output_text", "text": "I'm AgentArk." }]
            }],
            "usage": { "input_tokens": 1, "output_tokens": 2, "total_tokens": 3 }
        });

        let response = parse_openai_responses_payload(
            &payload,
            1,
            "",
            "openai-subscription",
            "gpt-5.4",
        )
        .expect("null error is not an error payload");
        assert_eq!(response.content, "I'm AgentArk.");
    }

    #[test]
    fn extract_partial_draft_files_reads_partial_app_deploy_files() {
        let previews = extract_partial_draft_files(
            "app_deploy",
            r#"{"title":"Demo","files":{"src/App.tsx":"export default function App() {\n  return <main>Hello</main>;\n}"}}"#,
        );

        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].file, "src/App.tsx");
        assert!(previews[0].content_snapshot.contains("Hello"));
        assert_eq!(previews[0].line_count, 3);
        assert!(previews[0].done);
    }

    #[test]
    fn extract_partial_draft_files_reads_partial_file_write_content() {
        let previews = extract_partial_draft_files(
            "file_write",
            r#"{"path":"/app/data/apps/new/demo/server.js","content":"console.log('demo');\nstart();"}"#,
        );

        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].file, "/app/data/apps/new/demo/server.js");
        assert!(previews[0].content_snapshot.contains("start();"));
        assert_eq!(previews[0].line_count, 2);
        assert!(previews[0].done);
    }

    #[test]
    fn parse_partial_tool_arguments_repairs_truncated_json() {
        let parsed = parse_partial_tool_arguments(
            r#"{"path":"/app/data/apps/new/demo/index.html","content":"<main>Hello""#,
        )
        .expect("partial json should repair");

        assert!(!parsed.1);
        assert_eq!(
            parsed.0.get("path").and_then(|value| value.as_str()),
            Some("/app/data/apps/new/demo/index.html")
        );
        assert_eq!(
            parsed.0.get("content").and_then(|value| value.as_str()),
            Some("<main>Hello")
        );
    }

    #[test]
    fn openai_stream_terminal_finish_reason_detected() {
        assert!(openai_stream_data_has_terminal_finish_reason(
            r#"{"choices":[{"finish_reason":"stop","delta":{}}]}"#
        ));
        assert!(openai_stream_data_has_terminal_finish_reason(
            r#"{"choices":[{"finish_reason":"tool_calls","delta":{}}]}"#
        ));
        assert!(!openai_stream_data_has_terminal_finish_reason(
            r#"{"choices":[{"delta":{"content":"hello"}}]}"#
        ));
    }
}

impl LlmProvider {
    /// Generate environment variables for deployed apps that need LLM access.
    /// Uses standardized OpenAI-compatible env vars so any SDK (openai, langchain, etc.) works.
    pub fn app_env_vars(&self) -> std::collections::HashMap<String, String> {
        let mut env = std::collections::HashMap::new();
        match self {
            LlmProvider::Anthropic { api_key, model } => {
                env.insert("LLM_PROVIDER".into(), "anthropic".into());
                env.insert("ANTHROPIC_API_KEY".into(), api_key.clone());
                env.insert("LLM_MODEL".into(), model.clone());
            }
            LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            } => {
                env.insert(
                    "LLM_PROVIDER".into(),
                    openai_provider_label(base_url.as_deref()).to_string(),
                );
                env.insert("OPENAI_API_KEY".into(), api_key.clone());
                env.insert("LLM_MODEL".into(), model.clone());
                if let Some(url) = display_openai_base_url(base_url.as_ref()) {
                    env.insert("OPENAI_BASE_URL".into(), url);
                }
            }
            LlmProvider::Ollama { base_url, model } => {
                env.insert("LLM_PROVIDER".into(), "ollama".into());
                env.insert("OLLAMA_BASE_URL".into(), base_url.clone());
                // Also set OpenAI-compatible vars pointing to Ollama's OpenAI endpoint
                env.insert("OPENAI_BASE_URL".into(), format!("{}/v1", base_url));
                env.insert("OPENAI_API_KEY".into(), "ollama".into());
                env.insert("LLM_MODEL".into(), model.clone());
            }
        }
        env
    }
}

impl Default for LlmProvider {
    fn default() -> Self {
        Self::Ollama {
            base_url: String::new(),
            model: String::new(),
        }
    }
}

/// Tool call from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// LLM response
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    /// Reasoning/thinking content (from OpenRouter reasoning models, etc.)
    pub reasoning: Option<String>,
    /// Token usage when known; may be estimated for local providers/streaming.
    pub usage: Option<LlmTokenUsage>,
    /// Provider label used for this request (e.g. openai, openai-compatible, anthropic, ollama).
    pub provider: String,
    /// Model identifier used for this request.
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub estimated: bool,
}

fn estimate_tokens_from_chars(chars: usize) -> u64 {
    ((chars.saturating_add(3)) / 4) as u64
}

/// LLM client
#[derive(Clone)]
pub struct LlmClient {
    provider: LlmProvider,
    client: reqwest::Client,
}

struct OpenAiChatParams<'a> {
    api_key: &'a str,
    model: &'a str,
    base_url: Option<&'a str>,
    system_prompt: &'a str,
    user_message: &'a str,
    history: &'a [crate::core::agent::ConversationMessage],
    actions: &'a [crate::actions::ActionDef],
}

struct OpenAiStreamParams<'a> {
    api_key: &'a str,
    model: &'a str,
    base_url: Option<&'a str>,
    system_prompt: &'a str,
    user_message: &'a str,
    history: &'a [crate::core::agent::ConversationMessage],
    actions: &'a [crate::actions::ActionDef],
    token_tx: Sender<StreamEvent>,
}

struct AnthropicStreamParams<'a> {
    api_key: &'a str,
    model: &'a str,
    system_prompt: &'a str,
    user_message: &'a str,
    history: &'a [crate::core::agent::ConversationMessage],
    actions: &'a [crate::actions::ActionDef],
    token_tx: Sender<StreamEvent>,
}

/// Last-resort text extraction from any JSON structure.
/// Walks the JSON tree looking for text content in common LLM response shapes
/// (choices[].message.content, output_text, result, response, text, etc.)
fn extract_text_from_any_json(value: &serde_json::Value) -> Option<String> {
    // Try common top-level text fields
    for key in &[
        "output_text",
        "result",
        "response",
        "text",
        "answer",
        "generated_text",
    ] {
        if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    // Try choices[*].message.content (flexible — accept content as string or object with text)
    if let Some(choices) = value.get("choices").and_then(|v| v.as_array()) {
        for choice in choices {
            // Standard: choice.message.content
            if let Some(msg) = choice.get("message").or_else(|| choice.get("delta")) {
                if let Some(content) = msg.get("content") {
                    if let Some(s) = content.as_str() {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                    // Content as array of blocks (Anthropic-style)
                    if let Some(arr) = content.as_array() {
                        let mut text = String::new();
                        for block in arr {
                            if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                text.push_str(t);
                            }
                        }
                        let trimmed = text.trim().to_string();
                        if !trimmed.is_empty() {
                            return Some(trimmed);
                        }
                    }
                }
            }
            // Some models put text directly on the choice
            if let Some(s) = choice.get("text").and_then(|v| v.as_str()) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    // Try data.choices (nested wrapper)
    if let Some(data) = value.get("data") {
        return extract_text_from_any_json(data);
    }
    None
}

fn json_contains_tool_call_indicators(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            for key in [
                "tool_calls",
                "function_call",
                "tool_use",
                "tool_call",
                "tool_outputs",
            ] {
                if map.contains_key(key) {
                    return true;
                }
            }
            map.values().any(json_contains_tool_call_indicators)
        }
        serde_json::Value::Array(arr) => arr.iter().any(json_contains_tool_call_indicators),
        _ => false,
    }
}

/// Check if an error is transient and worth retrying (timeouts, connection issues, decode errors).
/// Returns false for HTTP 4xx client errors (auth, validation) which won't succeed on retry.
fn is_retryable_error(err: &anyhow::Error) -> bool {
    let msg = format!("{}", err);
    let msg_lower = msg.to_lowercase();
    // Check reqwest-specific error types
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if reqwest_err.is_timeout() || reqwest_err.is_connect() || reqwest_err.is_decode() {
            return true;
        }
        // Don't retry 4xx client errors
        if let Some(status) = reqwest_err.status() {
            if status.is_client_error() && status.as_u16() != 429 {
                return false;
            }
        }
    }
    // Check for common transient error strings
    msg_lower.contains("connection reset")
        || msg_lower.contains("broken pipe")
        || msg_lower.contains("error decoding response body")
        || msg_lower.contains("connection closed")
        || msg_lower.contains("stream ended unexpectedly")
        || msg_lower.contains("incomplete message")
}

/// Retry backoff delays in milliseconds for each attempt (attempt 1, 2, 3)
const RETRY_DELAYS_MS: [u64; 3] = [500, 1500, 3000];
const MAX_RETRY_ATTEMPTS: u32 = 3;
const DEFAULT_LLM_HTTP_TIMEOUT_SECS: u64 = 600;
const DEFAULT_LLM_STREAM_FIRST_TOKEN_TIMEOUT_SECS: u64 = 90;
const DEFAULT_LLM_STREAM_INTER_CHUNK_TIMEOUT_SECS: u64 = 60;
const DEFAULT_LLM_STREAM_TOTAL_TIMEOUT_SECS: u64 = 240;

fn llm_http_timeout_secs() -> u64 {
    std::env::var("AGENTARK_LLM_HTTP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs >= 30 && *secs <= 1800)
        .unwrap_or(DEFAULT_LLM_HTTP_TIMEOUT_SECS)
}

fn llm_stream_first_token_timeout_secs() -> u64 {
    std::env::var("AGENTARK_LLM_STREAM_FIRST_TOKEN_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs >= 30 && *secs <= 1800)
        .unwrap_or(DEFAULT_LLM_STREAM_FIRST_TOKEN_TIMEOUT_SECS)
}

fn llm_stream_inter_chunk_timeout_secs() -> u64 {
    std::env::var("AGENTARK_LLM_STREAM_INTER_CHUNK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs >= 5 && *secs <= 600)
        .unwrap_or(DEFAULT_LLM_STREAM_INTER_CHUNK_TIMEOUT_SECS)
}

fn llm_stream_total_timeout_secs() -> u64 {
    std::env::var("AGENTARK_LLM_STREAM_TOTAL_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs >= 30 && *secs <= 1800)
        .unwrap_or(DEFAULT_LLM_STREAM_TOTAL_TIMEOUT_SECS)
}

fn openai_stream_data_has_terminal_finish_reason(data: &str) -> bool {
    #[derive(Deserialize)]
    struct TerminalChunk {
        #[serde(default)]
        choices: Vec<TerminalChoice>,
    }

    #[derive(Deserialize)]
    struct TerminalChoice {
        #[serde(default)]
        finish_reason: Option<String>,
    }

    serde_json::from_str::<TerminalChunk>(data)
        .ok()
        .is_some_and(|chunk| {
            chunk
                .choices
                .iter()
                .filter_map(|choice| choice.finish_reason.as_deref())
                .any(|reason| !reason.trim().is_empty())
        })
}

impl LlmClient {
    /// Get the model name string for this client
    pub fn model_name(&self) -> &str {
        match &self.provider {
            LlmProvider::Anthropic { model, .. } => model,
            LlmProvider::OpenAI { model, .. } => model,
            LlmProvider::Ollama { model, .. } => model,
        }
    }

    pub fn requires_streaming_responses_api(&self) -> bool {
        matches!(
            &self.provider,
            LlmProvider::OpenAI {
                base_url: Some(base_url),
                ..
            } if is_codex_cli_base_url(base_url)
        )
    }

    pub fn provider_name(&self) -> &'static str {
        match &self.provider {
            LlmProvider::Anthropic { .. } => "anthropic",
            LlmProvider::OpenAI { base_url, .. } => openai_provider_label(base_url.as_deref()),
            LlmProvider::Ollama { .. } => "ollama",
        }
    }

    pub fn new(provider: &LlmProvider) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(llm_http_timeout_secs()))
            .build()?;

        Ok(Self {
            provider: provider.clone(),
            client,
        })
    }

    /// Send a chat request to the LLM
    pub async fn chat(
        &self,
        system_prompt: &str,
        user_message: &str,
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
    ) -> Result<LlmResponse> {
        self.chat_with_history_for_helper(
            system_prompt,
            user_message,
            &[],
            _memories,
            actions,
            &crate::security::ModelPrivacyConfig::default(),
            false,
        )
        .await
    }

    /// Simple chat with just system prompt and user message (no tools/actions)
    /// Used by browser automation loop and other subsystems that don't need tool calling
    pub async fn chat_with_system(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<LlmResponse> {
        self.chat_for_helper_request(
            system_prompt,
            user_message,
            &[],
            &[],
            &crate::security::ModelPrivacyConfig::default(),
            false,
        )
        .await
    }

    pub async fn chat_for_helper_request(
        &self,
        system_prompt: &str,
        user_message: &str,
        memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        self.chat_with_history_for_helper(
            system_prompt,
            user_message,
            &[],
            memories,
            actions,
            policy,
            allow_sensitive_context,
        )
        .await
    }

    /// Send a chat request with conversation history
    pub async fn chat_with_history(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
    ) -> Result<LlmResponse> {
        self.chat_with_history_for_helper(
            system_prompt,
            user_message,
            history,
            _memories,
            actions,
            &crate::security::ModelPrivacyConfig::default(),
            false,
        )
        .await
    }

    pub async fn chat_with_history_for_helper(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        self.chat_with_history_in_mode(
            ModelRequestMode::Helper,
            system_prompt,
            user_message,
            history,
            _memories,
            actions,
            policy,
            allow_sensitive_context,
        )
        .await
    }

    pub async fn chat_for_primary_assistant_turn(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        self.chat_with_history_in_mode(
            ModelRequestMode::PrimaryAssistantTurn,
            system_prompt,
            user_message,
            history,
            _memories,
            actions,
            policy,
            allow_sensitive_context,
        )
        .await
    }

    async fn chat_with_history_in_mode(
        &self,
        mode: ModelRequestMode,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        let (system_prompt, user_message, sanitized_history) = sanitize_model_request_bundle(
            mode,
            system_prompt,
            user_message,
            history,
            policy,
            allow_sensitive_context,
        );
        let history = sanitized_history;
        let (provider_name, model_name) = match &self.provider {
            LlmProvider::Anthropic { model, .. } => ("anthropic", model.as_str()),
            LlmProvider::OpenAI {
                model, base_url, ..
            } => (openai_provider_label(base_url.as_deref()), model.as_str()),
            LlmProvider::Ollama { model, .. } => ("ollama", model.as_str()),
        };

        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|m| m.content.len()).sum::<usize>();
        tracing::info!(
            "LLM call → provider={}, model={}, history={} msgs, tools={}, prompt=~{}chars",
            provider_name,
            model_name,
            history.len(),
            actions.len(),
            prompt_chars
        );

        let start = std::time::Instant::now();
        let result = match &self.provider {
            LlmProvider::Anthropic { api_key, model } => {
                self.chat_anthropic_with_history(
                    api_key,
                    model,
                    &system_prompt,
                    &user_message,
                    &history,
                    actions,
                )
                .await
            }
            LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            } => {
                self.chat_openai_with_history(OpenAiChatParams {
                    api_key,
                    model,
                    base_url: base_url.as_deref(),
                    system_prompt: &system_prompt,
                    user_message: &user_message,
                    history: &history,
                    actions,
                })
                .await
            }
            LlmProvider::Ollama { base_url, model } => {
                self.chat_ollama_with_history(
                    base_url,
                    model,
                    &system_prompt,
                    &user_message,
                    &history,
                )
                .await
            }
        };

        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => {
                crate::metrics::observe_llm_call(
                    provider_name,
                    model_name,
                    "ok",
                    elapsed,
                    resp.usage.as_ref().map(|usage| usage.prompt_tokens),
                    resp.usage.as_ref().map(|usage| usage.completion_tokens),
                );
                let preview: String = resp.content.chars().take(120).collect();
                tracing::info!(
                    "LLM done ← {}ms, response={}chars, tool_calls={}, preview=\"{}{}\"",
                    elapsed.as_millis(),
                    resp.content.len(),
                    resp.tool_calls.len(),
                    preview,
                    if resp.content.len() > 120 { "..." } else { "" }
                );
            }
            Err(e) => {
                crate::metrics::observe_llm_call(
                    provider_name,
                    model_name,
                    "error",
                    elapsed,
                    None,
                    None,
                );
                tracing::error!("LLM failed ← {}ms, error: {}", elapsed.as_millis(), e);
            }
        }

        result
    }

    async fn chat_anthropic_with_history(
        &self,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        history: &[crate::core::agent::ConversationMessage],
        actions: &[crate::actions::ActionDef],
    ) -> Result<LlmResponse> {
        #[derive(Serialize)]
        struct AnthropicRequest {
            model: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            system: String,
            messages: Vec<AnthropicMessage>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            tools: Vec<AnthropicTool>,
        }

        #[derive(Serialize)]
        struct AnthropicMessage {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct AnthropicTool {
            name: String,
            description: String,
            input_schema: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct AnthropicResponse {
            content: Vec<ContentBlock>,
            #[serde(default)]
            usage: Option<AnthropicUsage>,
        }

        #[derive(Deserialize)]
        struct AnthropicUsage {
            #[serde(default)]
            input_tokens: u64,
            #[serde(default)]
            output_tokens: u64,
        }

        #[derive(Deserialize)]
        #[serde(tag = "type")]
        enum ContentBlock {
            #[serde(rename = "text")]
            Text { text: String },
            #[serde(rename = "tool_use")]
            ToolUse {
                id: String,
                name: String,
                input: serde_json::Value,
            },
        }

        let tools: Vec<AnthropicTool> = actions
            .iter()
            .map(|s| AnthropicTool {
                name: s.name.clone(),
                description: s.description.clone(),
                input_schema: s.input_schema.clone(),
            })
            .collect();

        // Build messages array with history (exclude the last user message as we add it separately)
        let mut messages: Vec<AnthropicMessage> = history
            .iter()
            .filter(|m| !(m.role == "user" && m.content == user_message))
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        // Add the current user message
        messages.push(AnthropicMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });

        let request = AnthropicRequest {
            model: model.to_string(),
            max_tokens: None,
            system: system_prompt.to_string(),
            messages,
            tools,
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Anthropic API").await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }

        let response: AnthropicResponse =
            read_response_json_limited(response, "Anthropic API").await?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for block in response.content {
            match block {
                ContentBlock::Text { text } => {
                    content.push_str(&text);
                }
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
            }
        }

        let usage = response.usage.map(|u| LlmTokenUsage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: u.input_tokens + u.output_tokens,
            estimated: false,
        });

        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|m| m.content.len()).sum::<usize>();
        let usage = usage.or_else(|| {
            let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
            let completion_tokens = estimate_tokens_from_chars(content.len());
            Some(LlmTokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                estimated: true,
            })
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            reasoning: None,
            usage,
            provider: "anthropic".to_string(),
            model: model.to_string(),
        })
    }

    async fn chat_openai_codex_responses(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        actions: &[crate::actions::ActionDef],
        request_config: ResolvedOpenAiRequestConfig,
    ) -> Result<LlmResponse> {
        let (token_tx, mut token_rx) = tokio::sync::mpsc::channel(64);
        let drain_handle = tokio::spawn(async move {
            while token_rx.recv().await.is_some() {}
        });
        let result = self
            .chat_openai_codex_responses_stream(
                model,
                system_prompt,
                user_message,
                history,
                actions,
                token_tx,
                request_config,
            )
            .await;
        drain_handle.abort();
        result
    }

    async fn chat_openai_with_history(&self, params: OpenAiChatParams<'_>) -> Result<LlmResponse> {
        let api_key = params.api_key;
        let model = params.model;
        let base_url = params.base_url;
        let system_prompt = params.system_prompt;
        let user_message = params.user_message;
        let history = params.history;
        let actions = params.actions;

        #[derive(Serialize)]
        struct OpenAIRequest {
            model: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            messages: Vec<OpenAIMessage>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            tools: Vec<OpenAITool>,
        }

        #[derive(Serialize)]
        struct OpenAIMessage {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct OpenAITool {
            #[serde(rename = "type")]
            tool_type: String,
            function: OpenAIFunction,
        }

        #[derive(Serialize)]
        struct OpenAIFunction {
            name: String,
            description: String,
            parameters: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            #[serde(default)]
            choices: Vec<OpenAIChoice>,
            #[serde(default)]
            usage: Option<OpenAIUsage>,
        }

        #[derive(Deserialize)]
        struct OpenAIUsage {
            #[serde(default)]
            prompt_tokens: u64,
            #[serde(default)]
            completion_tokens: u64,
            #[serde(default)]
            total_tokens: u64,
        }

        #[derive(Deserialize)]
        struct OpenAIChoice {
            #[serde(default)]
            message: OpenAIResponseMessage,
        }

        #[derive(Deserialize, Default)]
        struct OpenAIResponseMessage {
            #[serde(default)]
            content: Option<serde_json::Value>,
            tool_calls: Option<Vec<OpenAIToolCall>>,
            /// OpenRouter reasoning content from reasoning-enabled models
            reasoning_content: Option<String>,
        }

        #[derive(Deserialize)]
        struct OpenAIToolCall {
            #[serde(default)]
            id: String,
            #[serde(default)]
            function: OpenAIFunctionCall,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OpenAIFunctionArguments {
            String(String),
            Json(serde_json::Value),
        }

        #[derive(Deserialize, Default)]
        struct OpenAIFunctionCall {
            #[serde(default)]
            name: String,
            #[serde(default)]
            arguments: Option<OpenAIFunctionArguments>,
        }

        let tools: Vec<OpenAITool> = actions
            .iter()
            .map(|s| OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIFunction {
                    name: s.name.clone(),
                    description: s.description.clone(),
                    parameters: normalize_openai_tool_schema(&s.input_schema),
                },
            })
            .collect();

        // Build messages with system prompt first
        let mut messages = vec![OpenAIMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        }];

        // Add conversation history (excluding the current message)
        for msg in history
            .iter()
            .filter(|m| !(m.role == "user" && m.content == user_message))
        {
            messages.push(OpenAIMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // Add current user message
        messages.push(OpenAIMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });

        let mut request_config =
            resolve_openai_request_config(&self.client, api_key, base_url).await?;
        if request_config.uses_codex_cli_oauth {
            return self
                .chat_openai_codex_responses(
                    model,
                    system_prompt,
                    user_message,
                    history,
                    actions,
                    request_config,
                )
                .await;
        }
        let endpoint = format!("{}/chat/completions", request_config.base_url);
        let request = OpenAIRequest {
            model: model.to_string(),
            max_tokens: None,
            messages,
            tools,
        };

        let mut last_err: Option<anyhow::Error> = None;
        let mut forced_oauth_refresh = false;
        for attempt in 0..MAX_RETRY_ATTEMPTS {
            if attempt > 0 {
                let delay = RETRY_DELAYS_MS[attempt as usize - 1];
                tracing::warn!(
                    "Non-streaming retry attempt {}/{} after {}ms delay (model={})",
                    attempt + 1,
                    MAX_RETRY_ATTEMPTS,
                    delay,
                    model,
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }

            let mut req = self
                .client
                .post(&endpoint)
                .header("Content-Type", "application/json");

            if !request_config.api_key.is_empty() {
                req = req.header(
                    "Authorization",
                    format!("Bearer {}", request_config.api_key),
                );
            }

            if request_config.is_openrouter {
                req = req
                    .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                    .header("X-Title", crate::branding::PRODUCT_NAME);
            }

            let response = match req.json(&request).send().await {
                Ok(r) => r,
                Err(e) => {
                    let err = anyhow::Error::from(e);
                    if attempt + 1 < MAX_RETRY_ATTEMPTS && is_retryable_error(&err) {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            };
            let status = response.status();

            if status == reqwest::StatusCode::UNAUTHORIZED
                && request_config.uses_codex_cli_oauth
                && !forced_oauth_refresh
            {
                let refreshed_api_key = force_refresh_codex_cli_api_key(&self.client)
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "OpenAI Subscription OAuth refresh did not return a usable access token"
                        )
                    })?;
                request_config.api_key = refreshed_api_key;
                forced_oauth_refresh = true;
                continue;
            }

            // Handle 429 Too Many Requests with Retry-After
            if status.as_u16() == 429 && attempt + 1 < MAX_RETRY_ATTEMPTS {
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2)
                    .min(30);
                tracing::warn!(
                    "Rate limited (429), waiting {}s before retry (model={})",
                    retry_after,
                    model,
                );
                tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;
                last_err = Some(anyhow!("OpenAI API rate limited (429)"));
                continue;
            }

            if !status.is_success() {
                let error =
                    match read_response_bytes_limited(response, "OpenAI-compatible API").await {
                        Ok(bytes) => {
                            let body = String::from_utf8_lossy(&bytes).trim().to_string();
                            if body.is_empty() {
                                "<empty body>".to_string()
                            } else {
                                body
                            }
                        }
                        Err(read_err) => format!("<failed to read error body: {}>", read_err),
                    };
                let err = anyhow!("OpenAI API error ({}): {}", status, error);
                if attempt + 1 < MAX_RETRY_ATTEMPTS && is_retryable_error(&err) {
                    last_err = Some(err);
                    continue;
                }
                return Err(err);
            }

            let response_text =
                match read_response_text_limited(response, "OpenAI-compatible API").await {
                    Ok(t) => t,
                    Err(e) => {
                        let err = e;
                        if attempt + 1 < MAX_RETRY_ATTEMPTS && is_retryable_error(&err) {
                            last_err = Some(err);
                            continue;
                        }
                        return Err(err);
                    }
                };
            let response_json: serde_json::Value = match serde_json::from_str(&response_text) {
                Ok(parsed) => parsed,
                Err(error) => {
                    if let Some(repaired) = repair_truncated_json(&response_text) {
                        tracing::warn!(
                            "Repaired malformed JSON response from {} for model {}",
                            request_config.provider_label,
                            model
                        );
                        repaired
                    } else {
                        let preview: String = response_text.chars().take(380).collect();
                        return Err(anyhow!(
                            "OpenAI-compatible response was not valid JSON: {}. Body preview: {}",
                            error,
                            preview
                        ));
                    }
                }
            };
            if response_json.get("choices").is_none() {
                if let Some(err_payload) = response_json.get("error") {
                    return Err(anyhow!(
                        "OpenAI-compatible API returned an error payload: {}",
                        err_payload
                    ));
                }
                if let Some(text) = response_json
                    .get("output_text")
                    .and_then(extract_openai_message_text)
                    .or_else(|| {
                        response_json
                            .get("message")
                            .and_then(extract_openai_message_text)
                    })
                {
                    if json_contains_tool_call_indicators(&response_json) {
                        let preview = serde_json::to_string(&response_json)
                            .unwrap_or_default()
                            .chars()
                            .take(380)
                            .collect::<String>();
                        return Err(anyhow!(
                            "OpenAI-compatible response contained tool-call fields outside the normal schema; refusing to flatten to plain text. Body preview: {}",
                            preview
                        ));
                    }
                    let prompt_chars = system_prompt.len()
                        + user_message.len()
                        + history.iter().map(|m| m.content.len()).sum::<usize>();
                    let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
                    let completion_tokens = estimate_tokens_from_chars(text.len());
                    return Ok(LlmResponse {
                        content: text.to_string(),
                        tool_calls: vec![],
                        reasoning: None,
                        usage: Some(LlmTokenUsage {
                            prompt_tokens,
                            completion_tokens,
                            total_tokens: prompt_tokens + completion_tokens,
                            estimated: true,
                        }),
                        provider: request_config.provider_label.to_string(),
                        model: model.to_string(),
                    });
                }
            }
            let response: OpenAIResponse = match serde_json::from_value(response_json.clone()) {
                Ok(r) => r,
                Err(e) => {
                    // Last-resort: try to extract any text content from the response JSON
                    // This handles non-standard models (GLM-5, etc.) that return unexpected schemas
                    let fallback_text = extract_text_from_any_json(&response_json);
                    if let Some(text) = fallback_text {
                        if json_contains_tool_call_indicators(&response_json) {
                            let preview = serde_json::to_string(&response_json)
                                .unwrap_or_default()
                                .chars()
                                .take(380)
                                .collect::<String>();
                            return Err(anyhow!(
                                "OpenAI-compatible response schema mismatch with tool-call fields present; refusing to flatten to plain text. {}. Body preview: {}",
                                e,
                                preview
                            ));
                        }
                        tracing::warn!(
                            "Schema mismatch but extracted fallback text ({}chars): {}",
                            text.len(),
                            e,
                        );
                        let prompt_chars = system_prompt.len()
                            + user_message.len()
                            + history.iter().map(|m| m.content.len()).sum::<usize>();
                        let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
                        let completion_tokens = estimate_tokens_from_chars(text.len());
                        return Ok(LlmResponse {
                            content: text,
                            tool_calls: vec![],
                            reasoning: None,
                            usage: Some(LlmTokenUsage {
                                prompt_tokens,
                                completion_tokens,
                                total_tokens: prompt_tokens + completion_tokens,
                                estimated: true,
                            }),
                            provider: request_config.provider_label.to_string(),
                            model: model.to_string(),
                        });
                    }
                    let preview = serde_json::to_string(&response_json)
                        .unwrap_or_default()
                        .chars()
                        .take(380)
                        .collect::<String>();
                    return Err(anyhow!(
                        "OpenAI-compatible response schema mismatch: {}. Body preview: {}",
                        e,
                        preview
                    ));
                }
            };
            let choice = response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("No response from OpenAI"))?;

            let content = choice
                .message
                .content
                .as_ref()
                .and_then(extract_openai_message_text)
                .unwrap_or_default();
            let reasoning = choice.message.reasoning_content;
            let tool_calls = choice
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(|tc| ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: match tc.function.arguments {
                        Some(OpenAIFunctionArguments::String(raw)) => {
                            parse_tool_arguments_with_self_heal(&raw)
                        }
                        Some(OpenAIFunctionArguments::Json(v)) => v,
                        None => serde_json::Value::Null,
                    },
                })
                .collect();

            let prompt_chars = system_prompt.len()
                + user_message.len()
                + history.iter().map(|m| m.content.len()).sum::<usize>();

            let usage = response.usage.map(|u| LlmTokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
                estimated: false,
            });
            let usage = usage.or_else(|| {
                let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
                let completion_tokens = estimate_tokens_from_chars(content.len());
                Some(LlmTokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                    estimated: true,
                })
            });

            return Ok(LlmResponse {
                content,
                tool_calls,
                reasoning,
                usage,
                provider: request_config.provider_label.to_string(),
                model: model.to_string(),
            });
        }

        Err(last_err.unwrap_or_else(|| {
            anyhow!(
                "Non-streaming LLM request failed after {} attempts",
                MAX_RETRY_ATTEMPTS
            )
        }))
    }

    /// Streaming chat with history. Sends token events when supported by the provider.
    #[allow(dead_code)]
    pub async fn chat_with_history_stream(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        token_tx: Sender<StreamEvent>,
    ) -> Result<LlmResponse> {
        self.chat_with_history_stream_for_helper(
            system_prompt,
            user_message,
            history,
            _memories,
            actions,
            token_tx,
            &crate::security::ModelPrivacyConfig::default(),
            false,
        )
        .await
    }

    #[allow(dead_code)]
    pub async fn chat_with_history_stream_for_helper(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        token_tx: Sender<StreamEvent>,
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        self.chat_with_history_stream_in_mode(
            ModelRequestMode::Helper,
            system_prompt,
            user_message,
            history,
            _memories,
            actions,
            token_tx,
            policy,
            allow_sensitive_context,
        )
        .await
    }

    pub async fn chat_for_primary_assistant_turn_stream(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        token_tx: Sender<StreamEvent>,
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        self.chat_with_history_stream_in_mode(
            ModelRequestMode::PrimaryAssistantTurn,
            system_prompt,
            user_message,
            history,
            _memories,
            actions,
            token_tx,
            policy,
            allow_sensitive_context,
        )
        .await
    }

    async fn chat_with_history_stream_in_mode(
        &self,
        mode: ModelRequestMode,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        token_tx: Sender<StreamEvent>,
        policy: &crate::security::ModelPrivacyConfig,
        allow_sensitive_context: bool,
    ) -> Result<LlmResponse> {
        let (system_prompt, user_message, sanitized_history) = sanitize_model_request_bundle(
            mode,
            system_prompt,
            user_message,
            history,
            policy,
            allow_sensitive_context,
        );
        let history = sanitized_history;
        let provider_name = self.provider_name().to_string();
        let model_name = self.model_name().to_string();
        let start = std::time::Instant::now();
        let result = match &self.provider {
            LlmProvider::Anthropic { api_key, model } => {
                self.chat_anthropic_with_history_stream(AnthropicStreamParams {
                    api_key,
                    model,
                    system_prompt: &system_prompt,
                    user_message: &user_message,
                    history: &history,
                    actions,
                    token_tx,
                })
                .await
            }
            LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            } => {
                self.chat_openai_with_history_stream(OpenAiStreamParams {
                    api_key,
                    model,
                    base_url: base_url.as_deref(),
                    system_prompt: &system_prompt,
                    user_message: &user_message,
                    history: &history,
                    actions,
                    token_tx,
                })
                .await
            }
            LlmProvider::Ollama { base_url, model } => {
                self.chat_ollama_with_history_stream(
                    base_url,
                    model,
                    &system_prompt,
                    &user_message,
                    &history,
                    token_tx,
                )
                .await
            }
        };
        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => {
                crate::metrics::observe_llm_call(
                    &provider_name,
                    &model_name,
                    "ok",
                    elapsed,
                    resp.usage.as_ref().map(|usage| usage.prompt_tokens),
                    resp.usage.as_ref().map(|usage| usage.completion_tokens),
                );
            }
            Err(_) => {
                crate::metrics::observe_llm_call(
                    &provider_name,
                    &model_name,
                    "error",
                    elapsed,
                    None,
                    None,
                );
            }
        }
        result
    }

    async fn chat_ollama_with_history(
        &self,
        base_url: &str,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        history: &[crate::core::agent::ConversationMessage],
    ) -> Result<LlmResponse> {
        #[derive(Serialize)]
        struct OllamaRequest {
            model: String,
            messages: Vec<OllamaMessage>,
            stream: bool,
        }

        #[derive(Serialize, Deserialize)]
        struct OllamaMessage {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            message: OllamaMessage,
            #[serde(default)]
            prompt_eval_count: Option<u64>,
            #[serde(default)]
            eval_count: Option<u64>,
        }

        // Build messages with system prompt first
        let mut messages = vec![OllamaMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        }];

        // Add conversation history
        for msg in history
            .iter()
            .filter(|m| !(m.role == "user" && m.content == user_message))
        {
            messages.push(OllamaMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // Add current user message
        messages.push(OllamaMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });

        let request = OllamaRequest {
            model: model.to_string(),
            messages,
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/api/chat", base_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Ollama API").await?;
            return Err(anyhow!("Ollama API error: {}", error));
        }

        let response: OllamaResponse = read_response_json_limited(response, "Ollama API").await?;

        let content = response.message.content;
        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|m| m.content.len()).sum::<usize>();
        let usage = match (response.prompt_eval_count, response.eval_count) {
            (Some(p), Some(c)) => Some(LlmTokenUsage {
                prompt_tokens: p,
                completion_tokens: c,
                total_tokens: p + c,
                estimated: false,
            }),
            _ => {
                let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
                let completion_tokens = estimate_tokens_from_chars(content.len());
                Some(LlmTokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                    estimated: true,
                })
            }
        };

        Ok(LlmResponse {
            content,
            tool_calls: vec![],
            reasoning: None,
            usage,
            provider: "ollama".to_string(),
            model: model.to_string(),
        })
    }

    async fn chat_ollama_with_history_stream(
        &self,
        base_url: &str,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        history: &[crate::core::agent::ConversationMessage],
        token_tx: Sender<StreamEvent>,
    ) -> Result<LlmResponse> {
        #[derive(Serialize)]
        struct OllamaRequest {
            model: String,
            messages: Vec<OllamaMessage>,
            stream: bool,
        }

        #[derive(Serialize, Deserialize)]
        struct OllamaMessage {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct OllamaStreamResponse {
            #[serde(default)]
            message: Option<OllamaMessage>,
            #[serde(default)]
            done: bool,
            #[serde(default)]
            error: Option<String>,
            #[serde(default)]
            prompt_eval_count: Option<u64>,
            #[serde(default)]
            eval_count: Option<u64>,
        }

        // Build messages with system prompt first
        let mut messages = vec![OllamaMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        }];

        // Add conversation history
        for msg in history
            .iter()
            .filter(|m| !(m.role == "user" && m.content == user_message))
        {
            messages.push(OllamaMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // Add current user message
        messages.push(OllamaMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });

        let request = OllamaRequest {
            model: model.to_string(),
            messages,
            stream: true,
        };

        let response = self
            .client
            .post(format!("{}/api/chat", base_url))
            .timeout(std::time::Duration::from_secs(600))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Ollama API").await?;
            return Err(anyhow!("Ollama API error: {}", error));
        }

        let mut content = String::new();
        let mut buffer = String::new();
        let mut done = false;
        let mut prompt_eval_count: Option<u64> = None;
        let mut eval_count: Option<u64> = None;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            let lines: Vec<&str> = buffer.split('\n').collect();
            let last = lines.last().copied().unwrap_or("");

            for line in lines.iter().take(lines.len().saturating_sub(1)) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parsed: OllamaStreamResponse = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(err) = parsed.error {
                    return Err(anyhow!("Ollama stream error: {}", err));
                }
                if let Some(msg) = parsed.message {
                    if !msg.content.is_empty() {
                        content.push_str(&msg.content);
                        queue_stream_event(&token_tx, StreamEvent::Token(msg.content));
                    }
                }
                if parsed.done {
                    prompt_eval_count = parsed.prompt_eval_count.or(prompt_eval_count);
                    eval_count = parsed.eval_count.or(eval_count);
                    done = true;
                    break;
                }
            }

            buffer = last.to_string();
            if done {
                break;
            }
        }

        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|m| m.content.len()).sum::<usize>();
        let usage = match (prompt_eval_count, eval_count) {
            (Some(p), Some(c)) => Some(LlmTokenUsage {
                prompt_tokens: p,
                completion_tokens: c,
                total_tokens: p + c,
                estimated: false,
            }),
            _ => {
                let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
                let completion_tokens = estimate_tokens_from_chars(content.len());
                Some(LlmTokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                    estimated: true,
                })
            }
        };

        Ok(LlmResponse {
            content,
            tool_calls: vec![],
            reasoning: None,
            usage,
            provider: "ollama".to_string(),
            model: model.to_string(),
        })
    }

    async fn chat_openai_codex_responses_stream(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        actions: &[crate::actions::ActionDef],
        token_tx: Sender<StreamEvent>,
        mut request_config: ResolvedOpenAiRequestConfig,
    ) -> Result<LlmResponse> {
        let endpoint = openai_responses_endpoint(&request_config);
        let request =
            build_openai_responses_request(model, system_prompt, user_message, history, actions, true);
        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|message| message.content.len()).sum::<usize>();
        let send_start = std::time::Instant::now();
        let mut forced_oauth_refresh = false;

        let response = loop {
            let response = self
                .client
                .post(&endpoint)
                .timeout(std::time::Duration::from_secs(600))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .bearer_auth(&request_config.api_key)
                .json(&request)
                .send()
                .await?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && !forced_oauth_refresh {
                let refreshed_api_key = force_refresh_codex_cli_api_key(&self.client)
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "OpenAI Subscription OAuth refresh did not return a usable access token"
                        )
                    })?;
                request_config.api_key = refreshed_api_key;
                forced_oauth_refresh = true;
                continue;
            }
            break response;
        };

        let status = response.status();
        if !status.is_success() {
            let error = match read_response_bytes_limited(response, "OpenAI Subscription").await {
                Ok(bytes) => {
                    let body = String::from_utf8_lossy(&bytes).trim().to_string();
                    if body.is_empty() {
                        "<empty body>".to_string()
                    } else {
                        body
                    }
                }
                Err(read_err) => format!("<failed to read error body: {}>", read_err),
            };
            return Err(anyhow!("OpenAI Subscription error ({}): {}", status, error));
        }

        let mut content = String::new();
        let mut reasoning: Option<String> = None;
        let mut completed_response: Option<serde_json::Value> = None;
        let mut first_token = true;
        let inter_chunk_timeout_secs = llm_stream_inter_chunk_timeout_secs();
        let first_token_timeout_secs = llm_stream_first_token_timeout_secs();
        let total_timeout_secs = llm_stream_total_timeout_secs();

        let heartbeat_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hb_done_clone = heartbeat_done.clone();
        let hb_tx = token_tx.clone();
        let hb_model = model.to_string();
        let hb_start = std::time::Instant::now();
        let heartbeat_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if hb_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let elapsed = hb_start.elapsed().as_secs();
                queue_stream_event(
                    &hb_tx,
                    StreamEvent::Thinking(format!(
                        "Waiting for {} to respond ({}s)...",
                        hb_model, elapsed
                    )),
                );
            }
        });

        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        loop {
            if send_start.elapsed().as_secs() >= total_timeout_secs {
                break;
            }
            let timeout_secs = if first_token {
                first_token_timeout_secs
            } else {
                inter_chunk_timeout_secs
            };
            let chunk = match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                stream.next(),
            )
            .await
            {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(error))) => return Err(error.into()),
                Ok(None) => break,
                Err(_) => break,
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            let lines: Vec<&str> = buffer.split('\n').collect();
            let last = lines.last().copied().unwrap_or("").to_string();

            for line in lines.iter().take(lines.len().saturating_sub(1)) {
                let line = line.trim_end_matches('\r').trim();
                if !line.starts_with("data:") {
                    continue;
                }
                let data = line.trim_start_matches("data:").trim();
                if data == "[DONE]" {
                    break;
                }
                let parsed: serde_json::Value = match serde_json::from_str(data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                match parsed.get("type").and_then(|value| value.as_str()).unwrap_or("") {
                    "response.output_text.delta" => {
                        if let Some(delta) = parsed.get("delta").and_then(|value| value.as_str()) {
                            if first_token {
                                first_token = false;
                                heartbeat_done.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            content.push_str(delta);
                            queue_stream_event(&token_tx, StreamEvent::Token(delta.to_string()));
                        }
                    }
                    "response.reasoning_summary_text.delta" => {
                        if let Some(delta) = parsed.get("delta").and_then(|value| value.as_str()) {
                            reasoning.get_or_insert_with(String::new).push_str(delta);
                        }
                    }
                    "response.completed" => {
                        completed_response = parsed.get("response").cloned();
                    }
                    _ => {}
                }
            }
            buffer = last;
        }
        heartbeat_done.store(true, std::sync::atomic::Ordering::Relaxed);
        heartbeat_handle.abort();

        if let Some(response_json) = completed_response {
            let mut parsed = parse_openai_responses_payload(
                &response_json,
                prompt_chars,
                &content,
                request_config.provider_label,
                model,
            )?;
            if parsed.reasoning.is_none() {
                parsed.reasoning = reasoning;
            }
            return Ok(parsed);
        }

        let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
        let completion_tokens = estimate_tokens_from_chars(content.len());
        Ok(LlmResponse {
            content,
            tool_calls: vec![],
            reasoning,
            usage: Some(LlmTokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                estimated: true,
            }),
            provider: request_config.provider_label.to_string(),
            model: model.to_string(),
        })
    }

    async fn chat_openai_with_history_stream(
        &self,
        params: OpenAiStreamParams<'_>,
    ) -> Result<LlmResponse> {
        let api_key = params.api_key;
        let model = params.model;
        let base_url = params.base_url;
        let system_prompt = params.system_prompt;
        let user_message = params.user_message;
        let history = params.history;
        let actions = params.actions;
        let token_tx = params.token_tx;

        use std::collections::HashMap;

        #[derive(Serialize)]
        struct OpenAIRequest {
            model: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            messages: Vec<OpenAIMessage>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            tools: Vec<OpenAITool>,
            stream: bool,
        }

        #[derive(Serialize)]
        struct OpenAIMessage {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct OpenAITool {
            #[serde(rename = "type")]
            tool_type: String,
            function: OpenAIFunction,
        }

        #[derive(Serialize)]
        struct OpenAIFunction {
            name: String,
            description: String,
            parameters: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct OpenAIStreamChunk {
            #[serde(default)]
            choices: Vec<OpenAIStreamChoice>,
        }

        #[derive(Deserialize)]
        struct OpenAIStreamChoice {
            #[serde(default)]
            delta: OpenAIStreamDelta,
            #[serde(default)]
            finish_reason: Option<String>,
        }

        #[derive(Deserialize, Default)]
        struct OpenAIStreamDelta {
            #[serde(default)]
            content: Option<serde_json::Value>,
            #[serde(default)]
            tool_calls: Option<Vec<OpenAIStreamToolCallDelta>>,
            #[serde(default)]
            reasoning_content: Option<String>,
        }

        #[derive(Deserialize)]
        struct OpenAIStreamToolCallDelta {
            index: usize,
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            function: Option<OpenAIStreamFunctionDelta>,
        }

        #[derive(Deserialize)]
        struct OpenAIStreamFunctionDelta {
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            arguments: Option<OpenAIStreamFunctionArguments>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OpenAIStreamFunctionArguments {
            String(String),
            Json(serde_json::Value),
        }

        #[derive(Default)]
        struct ToolBuilder {
            id: String,
            name: String,
            args: String,
            last_progress_emit_chars: usize,
            last_progress_emit_at: Option<std::time::Instant>,
            emitted_draft_snapshots: HashMap<String, (String, bool)>,
        }

        let tools: Vec<OpenAITool> = actions
            .iter()
            .map(|s| OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIFunction {
                    name: s.name.clone(),
                    description: s.description.clone(),
                    parameters: normalize_openai_tool_schema(&s.input_schema),
                },
            })
            .collect();

        // Build messages with system prompt first
        let mut messages = vec![OpenAIMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        }];

        // Add conversation history (excluding the current message)
        for msg in history
            .iter()
            .filter(|m| !(m.role == "user" && m.content == user_message))
        {
            messages.push(OpenAIMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // Add current user message
        messages.push(OpenAIMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });

        let mut request_config =
            resolve_openai_request_config(&self.client, api_key, base_url).await?;
        if request_config.uses_codex_cli_oauth {
            return self
                .chat_openai_codex_responses_stream(
                    model,
                    system_prompt,
                    user_message,
                    history,
                    actions,
                    token_tx,
                    request_config,
                )
                .await;
        }
        let url = request_config.base_url.clone();
        tracing::info!(
            "LLM stream → {} model={} msgs={} tools={}",
            url,
            model,
            messages.len(),
            tools.len()
        );

        let request = OpenAIRequest {
            model: model.to_string(),
            max_tokens: None,
            messages,
            tools,
            stream: true,
        };
        let send_start = std::time::Instant::now();
        let mut req = self
            .client
            .post(format!("{}/chat/completions", url))
            .timeout(std::time::Duration::from_secs(600))
            .header("Content-Type", "application/json");

        if !request_config.api_key.is_empty() {
            req = req.header(
                "Authorization",
                format!("Bearer {}", request_config.api_key),
            );
        }

        // OpenRouter app identification headers
        if request_config.is_openrouter {
            req = req
                .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                .header("X-Title", crate::branding::PRODUCT_NAME);
        }

        let mut response = match req.json(&request).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(
                    "LLM stream send failed after {}ms: {}",
                    send_start.elapsed().as_millis(),
                    e
                );
                return Err(e.into());
            }
        };

        let mut status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && request_config.uses_codex_cli_oauth {
            let refreshed_api_key = force_refresh_codex_cli_api_key(&self.client)
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "OpenAI Subscription OAuth refresh did not return a usable access token"
                    )
                })?;
            request_config.api_key = refreshed_api_key;

            let mut retry_req = self
                .client
                .post(format!("{}/chat/completions", request_config.base_url))
                .timeout(std::time::Duration::from_secs(600))
                .header("Content-Type", "application/json")
                .header(
                    "Authorization",
                    format!("Bearer {}", request_config.api_key),
                );
            if request_config.is_openrouter {
                retry_req = retry_req
                    .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                    .header("X-Title", crate::branding::PRODUCT_NAME);
            }
            response = retry_req.json(&request).send().await?;
            status = response.status();
        }
        tracing::info!(
            "LLM stream response status={} after {}ms",
            status,
            send_start.elapsed().as_millis()
        );

        // Handle 429 Too Many Requests for streaming
        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2)
                .min(30);
            tracing::warn!(
                "Stream rate limited (429), waiting {}s before error (model={})",
                retry_after,
                model,
            );
            tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;
            return Err(anyhow!(
                "OpenAI API rate limited (429), retried after {}s",
                retry_after
            ));
        }

        if !status.is_success() {
            let error = match read_response_bytes_limited(response, "OpenAI API").await {
                Ok(bytes) => {
                    let body = String::from_utf8_lossy(&bytes).trim().to_string();
                    if body.is_empty() {
                        "<empty body>".to_string()
                    } else {
                        body
                    }
                }
                Err(read_err) => format!("<failed to read error body: {}>", read_err),
            };
            tracing::error!(
                "LLM stream error status={}: {}",
                status,
                &error[..error.len().min(500)]
            );
            return Err(anyhow!("OpenAI API error: {}", error));
        }

        let mut content = String::new();
        let mut reasoning: Option<String> = None;
        let mut tool_builders: HashMap<usize, ToolBuilder> = HashMap::new();
        let mut first_token = true;
        let inter_chunk_timeout_secs = llm_stream_inter_chunk_timeout_secs();
        let first_token_timeout_secs = llm_stream_first_token_timeout_secs();
        let total_timeout_secs = llm_stream_total_timeout_secs();
        let mut last_meaningful_progress_at = std::time::Instant::now();

        // Spawn heartbeat: emit Thinking events every 5s while waiting for first token
        let heartbeat_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hb_done_clone = heartbeat_done.clone();
        let hb_tx = token_tx.clone();
        let hb_model = model.to_string();
        let hb_start = std::time::Instant::now();
        let heartbeat_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if hb_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let elapsed = hb_start.elapsed().as_secs();
                let status = format!("Waiting for {} to respond ({}s)...", hb_model, elapsed);
                queue_stream_event(&hb_tx, StreamEvent::Thinking(status));
            }
        });

        let mut buffer = String::new();
        let mut done = false;
        let mut consecutive_errors: u32 = 0;
        let mut stream = response.bytes_stream();
        let mut stream_broken = false;
        loop {
            if send_start.elapsed().as_secs() >= total_timeout_secs {
                tracing::warn!(
                    "Stream exceeded total timeout ({}s), breaking with partial response",
                    total_timeout_secs,
                );
                stream_broken = true;
                break;
            }
            // Use a much longer timeout while waiting for the first token
            let timeout_secs = if first_token {
                first_token_timeout_secs
            } else {
                inter_chunk_timeout_secs
            };
            let chunk = match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                stream.next(),
            )
            .await
            {
                Ok(Some(Ok(c))) => {
                    consecutive_errors = 0;
                    c
                }
                Ok(Some(Err(e))) => {
                    tracing::warn!("Stream chunk error (continuing): {}", e);
                    consecutive_errors += 1;
                    if consecutive_errors > 3 {
                        tracing::warn!("Too many consecutive stream chunk errors, breaking");
                        stream_broken = true;
                        break;
                    }
                    continue;
                }
                Ok(None) => break, // stream ended normally
                Err(_) => {
                    tracing::warn!(
                        "Stream stalled ({}s no data), breaking with partial response",
                        timeout_secs,
                    );
                    stream_broken = true;
                    break;
                }
            };
            let chunk_received_at = std::time::Instant::now();
            let mut chunk_had_meaningful_progress = false;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            let lines: Vec<&str> = buffer.split('\n').collect();
            let last = lines.last().copied().unwrap_or("");

            for line in lines.iter().take(lines.len().saturating_sub(1)) {
                let line = line.trim_end_matches('\r').trim();
                if !line.starts_with("data:") {
                    continue;
                }
                let data = line.trim_start_matches("data:").trim();
                if data == "[DONE]" {
                    done = true;
                    chunk_had_meaningful_progress = true;
                    break;
                }

                let parsed: OpenAIStreamChunk = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                for choice in parsed.choices {
                    if let Some(rc) = choice.delta.reasoning_content {
                        let r = reasoning.get_or_insert_with(String::new);
                        r.push_str(&rc);
                        chunk_had_meaningful_progress = true;
                    }
                    if let Some(content_delta) = choice.delta.content {
                        if let Some(tok) = extract_openai_delta_text(&content_delta) {
                            if first_token {
                                tracing::info!(
                                    "LLM stream first token after {}ms",
                                    send_start.elapsed().as_millis()
                                );
                                first_token = false;
                                // Stop the heartbeat now that real tokens are flowing
                                heartbeat_done.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            content.push_str(&tok);
                            queue_stream_event(&token_tx, StreamEvent::Token(tok));
                            chunk_had_meaningful_progress = true;
                        }
                    }
                    if let Some(tcs) = choice.delta.tool_calls {
                        if first_token {
                            tracing::info!(
                                "LLM stream first tool delta after {}ms",
                                send_start.elapsed().as_millis()
                            );
                            first_token = false;
                            // Stop the heartbeat now that usable output is flowing.
                            heartbeat_done.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        if !tcs.is_empty() {
                            chunk_had_meaningful_progress = true;
                        }
                        for tc in tcs {
                            let progress_update = {
                                let entry = tool_builders.entry(tc.index).or_default();
                                if entry.id.is_empty() {
                                    if let Some(id) = tc.id {
                                        entry.id = id;
                                    }
                                }
                                if let Some(func) = tc.function {
                                    if entry.name.is_empty() {
                                        if let Some(name) = func.name {
                                            entry.name = name;
                                        }
                                    }
                                    if let Some(args) = func.arguments {
                                        match args {
                                            OpenAIStreamFunctionArguments::String(chunk) => {
                                                entry.args.push_str(&chunk);
                                            }
                                            OpenAIStreamFunctionArguments::Json(value) => {
                                                if entry.args.is_empty() {
                                                    entry.args = value.to_string();
                                                }
                                            }
                                        }
                                    }
                                    let arg_chars = entry.args.chars().count();
                                    let progress_step = tool_argument_progress_step(&entry.name);
                                    let now = std::time::Instant::now();
                                    let should_emit_progress = !entry.name.is_empty()
                                        && arg_chars > 0
                                        && (entry.last_progress_emit_chars == 0
                                            || arg_chars
                                                >= entry.last_progress_emit_chars + progress_step
                                            || entry
                                                .last_progress_emit_at
                                                .map(|last_emit| {
                                                    now.duration_since(last_emit).as_secs() >= 3
                                                        && arg_chars
                                                            > entry.last_progress_emit_chars
                                                })
                                                .unwrap_or(false));
                                    if should_emit_progress {
                                        entry.last_progress_emit_chars = arg_chars;
                                        entry.last_progress_emit_at = Some(now);
                                        let progress_msg = if entry.name == "app_deploy" {
                                            format!(
                                                "Generating deploy payload... {} chars",
                                                arg_chars
                                            )
                                        } else {
                                            format!(
                                                "Generating {} arguments... {} chars",
                                                entry.name, arg_chars
                                            )
                                        };
                                        Some((
                                            entry.name.clone(),
                                            entry.args.clone(),
                                            progress_msg,
                                            send_start.elapsed().as_secs(),
                                            arg_chars,
                                        ))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            };
                            if let Some((
                                tool_name,
                                raw_args,
                                progress_msg,
                                elapsed_secs,
                                arg_chars,
                            )) = progress_update
                            {
                                let stage = if tool_name == "app_deploy" {
                                    "payload_build"
                                } else {
                                    "argument_build"
                                };
                                emit_stream_tool_progress(
                                    &token_tx,
                                    &tool_name,
                                    progress_msg.clone(),
                                    serde_json::json!({
                                        "kind": "argument_stream",
                                        "stage": stage,
                                        "chars": arg_chars,
                                        "elapsed_secs": elapsed_secs,
                                        "stream_key": format!("argument-stream:{}", tool_name),
                                    }),
                                )
                                .await;
                                emit_argument_phase_status(
                                    &token_tx,
                                    &tool_name,
                                    progress_msg,
                                    elapsed_secs,
                                )
                                .await;
                                if let Some(entry) = tool_builders.get_mut(&tc.index) {
                                    emit_partial_draft_file_previews(
                                        &token_tx,
                                        &tool_name,
                                        &raw_args,
                                        &mut entry.emitted_draft_snapshots,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    if let Some(finish_reason) = choice.finish_reason.as_deref() {
                        if !finish_reason.trim().is_empty() {
                            tracing::info!(
                                "LLM stream finish_reason={} after {}ms",
                                finish_reason,
                                send_start.elapsed().as_millis()
                            );
                            chunk_had_meaningful_progress = true;
                            done = true;
                        }
                    }
                }
            }

            buffer = last.to_string();
            if chunk_had_meaningful_progress {
                last_meaningful_progress_at = chunk_received_at;
            }
            if done {
                break;
            }
            let allowed_idle_secs = if first_token {
                first_token_timeout_secs
            } else {
                inter_chunk_timeout_secs
            };
            if last_meaningful_progress_at.elapsed().as_secs() >= allowed_idle_secs {
                tracing::warn!(
                    "Stream received bytes but no meaningful SSE progress for {}s, breaking with partial response",
                    allowed_idle_secs,
                );
                stream_broken = true;
                break;
            }
        }

        let trailing = buffer.trim_end_matches('\r').trim();
        if !done && trailing.starts_with("data:") {
            let trailing_data = trailing.trim_start_matches("data:").trim();
            if trailing_data == "[DONE]"
                || openai_stream_data_has_terminal_finish_reason(trailing_data)
            {
                tracing::info!(
                    "LLM stream terminal marker recovered from trailing buffer after {}ms",
                    send_start.elapsed().as_millis()
                );
                done = true;
            }
        }

        // Ensure heartbeat is stopped after stream loop exits
        heartbeat_done.store(true, std::sync::atomic::Ordering::Relaxed);
        heartbeat_handle.abort();

        let has_content = !content.trim().is_empty();
        let has_tools =
            !tool_builders.is_empty() && tool_builders.values().any(|tb| !tb.name.is_empty());

        if !done && !stream_broken && !has_content && !has_tools {
            return Err(anyhow!(
                "Stream ended without content or tool calls after {}ms",
                send_start.elapsed().as_millis()
            ));
        }

        if stream_broken && !done {
            if has_content || has_tools {
                tracing::warn!(
                    "Stream broke prematurely but we have partial data (content={}chars, tools={}), returning partial response",
                    content.len(),
                    tool_builders.len(),
                );
            } else {
                return Err(anyhow!(
                    "Stream broke with no usable content after {}ms",
                    send_start.elapsed().as_millis()
                ));
            }
        }

        tracing::info!(
            "LLM stream done ← {}ms, content={}chars, tool_builders={}, clean={}",
            send_start.elapsed().as_millis(),
            content.len(),
            tool_builders.len(),
            done && !stream_broken,
        );

        let mut tool_calls: Vec<(usize, ToolCall)> = tool_builders
            .into_iter()
            .map(|(idx, tb)| {
                let args = parse_tool_arguments_with_self_heal(&tb.args);
                (
                    idx,
                    ToolCall {
                        id: if tb.id.is_empty() {
                            uuid::Uuid::new_v4().to_string()
                        } else {
                            tb.id
                        },
                        name: tb.name,
                        arguments: args,
                    },
                )
            })
            .collect();
        tool_calls.sort_by_key(|(idx, _)| *idx);
        let tool_calls: Vec<ToolCall> = tool_calls.into_iter().map(|(_, tc)| tc).collect();

        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|m| m.content.len()).sum::<usize>();
        let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
        let completion_tokens = estimate_tokens_from_chars(content.len());
        let usage = Some(LlmTokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            estimated: true,
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            reasoning,
            usage,
            provider: request_config.provider_label.to_string(),
            model: model.to_string(),
        })
    }

    async fn chat_anthropic_with_history_stream(
        &self,
        params: AnthropicStreamParams<'_>,
    ) -> Result<LlmResponse> {
        let api_key = params.api_key;
        let model = params.model;
        let system_prompt = params.system_prompt;
        let user_message = params.user_message;
        let history = params.history;
        let actions = params.actions;
        let token_tx = params.token_tx;

        use std::collections::HashMap;

        #[derive(Serialize)]
        struct AnthropicRequest {
            model: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            system: String,
            messages: Vec<AnthropicMessage>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            tools: Vec<AnthropicTool>,
            stream: bool,
        }

        #[derive(Serialize)]
        struct AnthropicMessage {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct AnthropicTool {
            name: String,
            description: String,
            input_schema: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct ContentBlockStartEvent {
            index: usize,
            content_block: AnthropicContentBlock,
        }

        #[derive(Deserialize)]
        struct ContentBlockDeltaEvent {
            index: usize,
            delta: AnthropicDelta,
        }

        #[derive(Deserialize)]
        struct AnthropicDelta {
            #[serde(rename = "type")]
            delta_type: String,
            #[serde(default)]
            text: Option<String>,
            #[serde(default)]
            partial_json: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(tag = "type")]
        enum AnthropicContentBlock {
            #[serde(rename = "text")]
            Text {
                #[serde(default)]
                text: Option<String>,
            },
            #[serde(rename = "tool_use")]
            ToolUse {
                id: String,
                name: String,
                #[serde(default)]
                input: Option<serde_json::Value>,
            },
        }

        #[derive(Default)]
        struct ToolBuilder {
            id: String,
            name: String,
            input_json: String,
            input_value: Option<serde_json::Value>,
            last_progress_emit_chars: usize,
            last_progress_emit_at: Option<std::time::Instant>,
            emitted_draft_snapshots: HashMap<String, (String, bool)>,
        }

        let tools: Vec<AnthropicTool> = actions
            .iter()
            .map(|s| AnthropicTool {
                name: s.name.clone(),
                description: s.description.clone(),
                input_schema: s.input_schema.clone(),
            })
            .collect();

        // Build messages array with history (exclude the last user message as we add it separately)
        let mut messages: Vec<AnthropicMessage> = history
            .iter()
            .filter(|m| !(m.role == "user" && m.content == user_message))
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        // Add the current user message
        messages.push(AnthropicMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });

        let request = AnthropicRequest {
            model: model.to_string(),
            max_tokens: None,
            system: system_prompt.to_string(),
            messages,
            tools,
            stream: true,
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .timeout(std::time::Duration::from_secs(600))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = read_response_text_limited(response, "Anthropic API").await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }

        let mut content = String::new();
        let mut tool_builders: HashMap<usize, ToolBuilder> = HashMap::new();
        let stream_started = std::time::Instant::now();

        let mut buffer = String::new();
        let mut current_event: Option<String> = None;
        let mut done = false;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            let lines: Vec<&str> = buffer.split('\n').collect();
            let last = lines.last().copied().unwrap_or("");

            for line in lines.iter().take(lines.len().saturating_sub(1)) {
                let line = line.trim_end_matches('\r');
                if line.starts_with("event:") {
                    current_event = Some(line.trim_start_matches("event:").trim().to_string());
                    continue;
                }
                if !line.starts_with("data:") {
                    continue;
                }
                let data = line.trim_start_matches("data:").trim();
                let Some(ev) = current_event.take() else {
                    continue;
                };
                if data.is_empty() {
                    continue;
                }

                match ev.as_str() {
                    "content_block_start" => {
                        if let Ok(parsed) = serde_json::from_str::<ContentBlockStartEvent>(data) {
                            match parsed.content_block {
                                AnthropicContentBlock::Text { text } => {
                                    if let Some(text) = text {
                                        if !text.is_empty() {
                                            content.push_str(&text);
                                            queue_stream_event(&token_tx, StreamEvent::Token(text));
                                        }
                                    }
                                }
                                AnthropicContentBlock::ToolUse { id, name, input } => {
                                    let entry = tool_builders.entry(parsed.index).or_default();
                                    entry.id = id;
                                    entry.name = name;
                                    entry.input_value = input;
                                }
                            }
                        }
                    }
                    "content_block_delta" => {
                        if let Ok(parsed) = serde_json::from_str::<ContentBlockDeltaEvent>(data) {
                            if parsed.delta.delta_type == "text_delta" {
                                if let Some(text) = parsed.delta.text {
                                    if !text.is_empty() {
                                        content.push_str(&text);
                                        queue_stream_event(&token_tx, StreamEvent::Token(text));
                                    }
                                }
                            } else if parsed.delta.delta_type == "input_json_delta" {
                                if let Some(partial) = parsed.delta.partial_json {
                                    let progress_update = {
                                        let entry = tool_builders.entry(parsed.index).or_default();
                                        entry.input_json.push_str(&partial);
                                        let arg_chars = entry.input_json.chars().count();
                                        let progress_step =
                                            tool_argument_progress_step(&entry.name);
                                        let now = std::time::Instant::now();
                                        let should_emit_progress = !entry.name.is_empty()
                                            && arg_chars > 0
                                            && (entry.last_progress_emit_chars == 0
                                                || arg_chars
                                                    >= entry.last_progress_emit_chars
                                                        + progress_step
                                                || entry
                                                    .last_progress_emit_at
                                                    .map(|last_emit| {
                                                        now.duration_since(last_emit).as_secs() >= 3
                                                            && arg_chars
                                                                > entry.last_progress_emit_chars
                                                    })
                                                    .unwrap_or(false));
                                        if should_emit_progress {
                                            entry.last_progress_emit_chars = arg_chars;
                                            entry.last_progress_emit_at = Some(now);
                                            let progress_msg = if entry.name == "app_deploy" {
                                                format!(
                                                    "Generating deploy payload... {} chars",
                                                    arg_chars
                                                )
                                            } else {
                                                format!(
                                                    "Generating {} arguments... {} chars",
                                                    entry.name, arg_chars
                                                )
                                            };
                                            Some((
                                                entry.name.clone(),
                                                entry.input_json.clone(),
                                                progress_msg,
                                                stream_started.elapsed().as_secs(),
                                                arg_chars,
                                            ))
                                        } else {
                                            None
                                        }
                                    };
                                    if let Some((
                                        tool_name,
                                        raw_input_json,
                                        progress_msg,
                                        elapsed_secs,
                                        arg_chars,
                                    )) = progress_update
                                    {
                                        let stage = if tool_name == "app_deploy" {
                                            "payload_build"
                                        } else {
                                            "argument_build"
                                        };
                                        emit_stream_tool_progress(
                                            &token_tx,
                                            &tool_name,
                                            progress_msg.clone(),
                                            serde_json::json!({
                                                "kind": "argument_stream",
                                                "stage": stage,
                                                "chars": arg_chars,
                                                "elapsed_secs": elapsed_secs,
                                                "stream_key": format!("argument-stream:{}", tool_name),
                                            }),
                                        )
                                        .await;
                                        emit_argument_phase_status(
                                            &token_tx,
                                            &tool_name,
                                            progress_msg,
                                            elapsed_secs,
                                        )
                                        .await;
                                        if let Some(entry) = tool_builders.get_mut(&parsed.index) {
                                            emit_partial_draft_file_previews(
                                                &token_tx,
                                                &tool_name,
                                                &raw_input_json,
                                                &mut entry.emitted_draft_snapshots,
                                            )
                                            .await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "message_stop" => {
                        done = true;
                        break;
                    }
                    _ => {}
                }
            }

            buffer = last.to_string();
            if done {
                break;
            }
        }

        let tool_calls = tool_builders
            .into_iter()
            .filter_map(|(_idx, tb)| {
                if tb.name.is_empty() {
                    return None;
                }
                let args = if !tb.input_json.trim().is_empty() {
                    serde_json::from_str(&tb.input_json)
                        .ok()
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    tb.input_value.unwrap_or(serde_json::Value::Null)
                };
                Some(ToolCall {
                    id: if tb.id.is_empty() {
                        uuid::Uuid::new_v4().to_string()
                    } else {
                        tb.id
                    },
                    name: tb.name,
                    arguments: args,
                })
            })
            .collect();

        let prompt_chars = system_prompt.len()
            + user_message.len()
            + history.iter().map(|m| m.content.len()).sum::<usize>();
        let prompt_tokens = estimate_tokens_from_chars(prompt_chars);
        let completion_tokens = estimate_tokens_from_chars(content.len());
        let usage = Some(LlmTokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            estimated: true,
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            reasoning: None,
            usage,
            provider: "anthropic".to_string(),
            model: model.to_string(),
        })
    }
}
