//! LLM client for agent reasoning

use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;

use crate::core::agent::{ConversationMessage, StreamEvent};

/// Supported LLM providers
#[derive(Debug, Clone, Serialize, Deserialize)]
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

fn is_codex_cli_base_url(base_url: Option<&str>) -> bool {
    base_url
        .map(|v| v.trim().eq_ignore_ascii_case("codex://cli"))
        .unwrap_or(false)
}

fn effective_openai_base_url(base_url: Option<&str>) -> &str {
    match base_url {
        Some(url) if is_codex_cli_base_url(Some(url)) => "https://api.openai.com/v1",
        Some(url) => url,
        None => "https://api.openai.com/v1",
    }
}

fn openai_provider_label(base_url: Option<&str>) -> &'static str {
    if is_codex_cli_base_url(base_url) {
        "openai-subscription"
    } else if base_url.unwrap_or("").is_empty() {
        "openai"
    } else {
        "openai-compatible"
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
    use super::normalize_openai_tool_schema;

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
                if let Some(url) = base_url {
                    if !is_codex_cli_base_url(Some(url.as_str())) {
                        env.insert("OPENAI_BASE_URL".into(), url.clone());
                    }
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
            base_url: "http://localhost:11434".to_string(),
            model: "llama3.2".to_string(),
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

impl LlmClient {
    /// Get the model name string for this client
    pub fn model_name(&self) -> &str {
        match &self.provider {
            LlmProvider::Anthropic { model, .. } => model,
            LlmProvider::OpenAI { model, .. } => model,
            LlmProvider::Ollama { model, .. } => model,
        }
    }

    pub fn new(provider: &LlmProvider) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
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
        // Call with empty history for backwards compatibility
        self.chat_with_history(system_prompt, user_message, &[], _memories, actions)
            .await
    }

    /// Simple chat with just system prompt and user message (no tools/actions)
    /// Used by browser automation loop and other subsystems that don't need tool calling
    pub async fn chat_with_system(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<LlmResponse> {
        self.chat(system_prompt, user_message, &[], &[]).await
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
                    system_prompt,
                    user_message,
                    history,
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
                    system_prompt,
                    user_message,
                    history,
                    actions,
                })
                .await
            }
            LlmProvider::Ollama { base_url, model } => {
                self.chat_ollama_with_history(base_url, model, system_prompt, user_message, history)
                    .await
            }
        };

        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => {
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
            max_tokens: u32,
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
            max_tokens: 4096,
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
            let error = response.text().await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }

        let response: AnthropicResponse = response.json().await?;

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

        let request = OpenAIRequest {
            model: model.to_string(),
            messages,
            tools,
        };

        let url = effective_openai_base_url(base_url);
        let endpoint = format!("{}/chat/completions", url);
        let is_openrouter = url.contains("openrouter");

        let mut last_err: Option<anyhow::Error> = None;
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
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json");

            if is_openrouter {
                req = req
                    .header("HTTP-Referer", "https://github.com/agentark-ai/AgentArk")
                    .header("X-Title", "AgentArk");
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
                let error = match response.bytes().await {
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

            let response_text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    let err = anyhow::Error::from(e);
                    if attempt + 1 < MAX_RETRY_ATTEMPTS && is_retryable_error(&err) {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            };
            let response_json: serde_json::Value =
                serde_json::from_str(&response_text).map_err(|e| {
                    let preview: String = response_text.chars().take(380).collect();
                    anyhow!(
                        "OpenAI-compatible response was not valid JSON: {}. Body preview: {}",
                        e,
                        preview
                    )
                })?;
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
                    let provider_label = openai_provider_label(base_url);
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
                        provider: provider_label.to_string(),
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
                        tracing::warn!(
                            "Schema mismatch but extracted fallback text ({}chars): {}",
                            text.len(),
                            e,
                        );
                        let provider_label = openai_provider_label(base_url);
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
                            provider: provider_label.to_string(),
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
                            let trimmed = raw.trim();
                            if trimmed.is_empty() {
                                serde_json::Value::Null
                            } else {
                                serde_json::from_str(trimmed)
                                    .unwrap_or_else(|_| serde_json::Value::String(raw))
                            }
                        }
                        Some(OpenAIFunctionArguments::Json(v)) => v,
                        None => serde_json::Value::Null,
                    },
                })
                .collect();

            let provider_label = openai_provider_label(base_url);

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
                provider: provider_label.to_string(),
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
    pub async fn chat_with_history_stream(
        &self,
        system_prompt: &str,
        user_message: &str,
        history: &[ConversationMessage],
        _memories: &[crate::memory::MemoryEntry],
        actions: &[crate::actions::ActionDef],
        token_tx: Sender<StreamEvent>,
    ) -> Result<LlmResponse> {
        match &self.provider {
            LlmProvider::Anthropic { api_key, model } => {
                self.chat_anthropic_with_history_stream(AnthropicStreamParams {
                    api_key,
                    model,
                    system_prompt,
                    user_message,
                    history,
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
                    system_prompt,
                    user_message,
                    history,
                    actions,
                    token_tx,
                })
                .await
            }
            LlmProvider::Ollama { base_url, model } => {
                self.chat_ollama_with_history_stream(
                    base_url,
                    model,
                    system_prompt,
                    user_message,
                    history,
                    token_tx,
                )
                .await
            }
        }
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
            let error = response.text().await?;
            return Err(anyhow!("Ollama API error: {}", error));
        }

        let response: OllamaResponse = response.json().await?;

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
            let error = response.text().await?;
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
                        let _ = token_tx.try_send(StreamEvent::Token(msg.content));
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

        let url = effective_openai_base_url(base_url);
        tracing::info!(
            "LLM stream → {} model={} msgs={} tools={}",
            url,
            model,
            messages.len(),
            tools.len()
        );

        let request = OpenAIRequest {
            model: model.to_string(),
            messages,
            tools,
            stream: true,
        };
        let send_start = std::time::Instant::now();
        let mut req = self
            .client
            .post(format!("{}/chat/completions", url))
            .timeout(std::time::Duration::from_secs(600))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json");

        // OpenRouter app identification headers
        if url.contains("openrouter") {
            req = req
                .header("HTTP-Referer", "https://github.com/agentark-ai/AgentArk")
                .header("X-Title", "AgentArk");
        }

        let response = match req.json(&request).send().await {
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

        let status = response.status();
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
            let error = match response.bytes().await {
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
        const TOOL_ARG_PROGRESS_STEP: usize = 4000;
        const INTER_CHUNK_TIMEOUT_SECS: u64 = 30;
        const FIRST_TOKEN_TIMEOUT_SECS: u64 = 300; // Slow models (GLM-5) may need minutes for TTFT

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
                if hb_tx.try_send(StreamEvent::Thinking(status)).is_err() {
                    break;
                }
            }
        });

        let mut buffer = String::new();
        let mut done = false;
        let mut consecutive_errors: u32 = 0;
        let mut stream = response.bytes_stream();
        let mut stream_broken = false;
        loop {
            // Use a much longer timeout while waiting for the first token
            let timeout_secs = if first_token {
                FIRST_TOKEN_TIMEOUT_SECS
            } else {
                INTER_CHUNK_TIMEOUT_SECS
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
                        INTER_CHUNK_TIMEOUT_SECS,
                    );
                    stream_broken = true;
                    break;
                }
            };
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
                            let _ = token_tx.try_send(StreamEvent::Token(tok));
                        }
                    }
                    if let Some(tcs) = choice.delta.tool_calls {
                        for tc in tcs {
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
                                let should_emit_progress = !entry.name.is_empty()
                                    && arg_chars > 0
                                    && (entry.last_progress_emit_chars == 0
                                        || arg_chars
                                            >= entry.last_progress_emit_chars
                                                + TOOL_ARG_PROGRESS_STEP);
                                if should_emit_progress {
                                    entry.last_progress_emit_chars = arg_chars;
                                    let progress_msg = if entry.name == "app_deploy" {
                                        format!("Generating deploy payload... {} chars", arg_chars)
                                    } else {
                                        format!(
                                            "Generating {} arguments... {} chars",
                                            entry.name, arg_chars
                                        )
                                    };
                                    let _ = token_tx.try_send(StreamEvent::ToolProgress {
                                        name: entry.name.clone(),
                                        content: progress_msg,
                                        payload: Some(serde_json::json!({
                                            "kind": "argument_stream",
                                            "chars": arg_chars,
                                        })),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            buffer = last.to_string();
            if done {
                break;
            }
        }

        // Ensure heartbeat is stopped after stream loop exits
        heartbeat_done.store(true, std::sync::atomic::Ordering::Relaxed);
        heartbeat_handle.abort();

        if stream_broken && !done {
            let has_content = !content.trim().is_empty();
            let has_tools =
                !tool_builders.is_empty() && tool_builders.values().any(|tb| !tb.name.is_empty());
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
                let args = if tb.args.trim().is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(&tb.args)
                        .unwrap_or_else(|_| serde_json::Value::String(tb.args.clone()))
                };
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

        let provider_label = openai_provider_label(base_url);

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
            provider: provider_label.to_string(),
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
            max_tokens: u32,
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
            max_tokens: 4096,
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
            let error = response.text().await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }

        let mut content = String::new();
        let mut tool_builders: HashMap<usize, ToolBuilder> = HashMap::new();
        const TOOL_ARG_PROGRESS_STEP: usize = 4000;

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
                                            let _ = token_tx.try_send(StreamEvent::Token(text));
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
                                        let _ = token_tx.try_send(StreamEvent::Token(text));
                                    }
                                }
                            } else if parsed.delta.delta_type == "input_json_delta" {
                                if let Some(partial) = parsed.delta.partial_json {
                                    let entry = tool_builders.entry(parsed.index).or_default();
                                    entry.input_json.push_str(&partial);
                                    let arg_chars = entry.input_json.chars().count();
                                    let should_emit_progress = !entry.name.is_empty()
                                        && arg_chars > 0
                                        && (entry.last_progress_emit_chars == 0
                                            || arg_chars
                                                >= entry.last_progress_emit_chars
                                                    + TOOL_ARG_PROGRESS_STEP);
                                    if should_emit_progress {
                                        entry.last_progress_emit_chars = arg_chars;
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
                                        let _ = token_tx.try_send(StreamEvent::ToolProgress {
                                            name: entry.name.clone(),
                                            content: progress_msg,
                                            payload: Some(serde_json::json!({
                                                "kind": "argument_stream",
                                                "chars": arg_chars,
                                            })),
                                        });
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
