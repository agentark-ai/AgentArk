//! Thin ArkOrbit agent path.
//!
//! This path performs a direct streaming model call with orbit-scoped context.
//! It never invokes the main agent turn loop, intent planner, semantic router,
//! or tool-call envelope path.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::actions::{ActionDef, ActionSource};
use crate::core::{ConversationMessage, LlmClient, LlmResponse, StreamEvent, ToolCall};

use super::models::OrbitChatMessage;
use super::service::ArkOrbitService;
use super::store::{validate_readable_orbit_path, validate_writable_orbit_path};

const HISTORY_LIMIT: usize = 24;
const READ_ROUND_LIMIT: usize = 1;
const MAX_READ_BYTES: usize = 96 * 1024;
const ORBIT_OPERATIONS_ACTION: &str = "arkorbit_apply_operations";

#[derive(Debug, Clone)]
pub enum OrbitAgentEvent {
    Status {
        message: String,
    },
    Token(String),
    FileWritten {
        path: String,
        operation: OrbitFileOperation,
    },
    ReadRequested {
        path: String,
    },
    Usage(OrbitChatUsage),
    Done,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbitFileOperation {
    Wrote,
    Edited,
}

impl OrbitFileOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wrote => "wrote",
            Self::Edited => "edited",
        }
    }

    fn past_tense(self) -> &'static str {
        match self {
            Self::Wrote => "wrote",
            Self::Edited => "edited",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrbitChatUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub estimated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
}

impl OrbitChatUsage {
    fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.total_tokens == 0
            && self.cost_usd.is_none()
            && self.duration_ms.unwrap_or(0) == 0
            && self.time_to_first_token_ms.unwrap_or(0) == 0
            && self.model.is_none()
    }

    fn merge(&mut self, next: OrbitChatUsage) {
        if let Some(model) = next.model {
            if !model.trim().is_empty() {
                self.model = Some(model);
            }
        }
        self.input_tokens = self.input_tokens.saturating_add(next.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(next.output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(next.total_tokens);
        self.estimated |= next.estimated;
        self.duration_ms = Some(
            self.duration_ms
                .unwrap_or(0)
                .saturating_add(next.duration_ms.unwrap_or(0)),
        )
        .filter(|value| *value > 0);
        if self.time_to_first_token_ms.is_none() {
            self.time_to_first_token_ms = next.time_to_first_token_ms;
        }
        self.cost_usd = match (self.cost_usd, next.cost_usd) {
            (Some(left), Some(right)) => Some(left + right),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        };
    }

    fn from_response(
        response: &LlmResponse,
        duration_ms: u64,
        time_to_first_token_ms: Option<u64>,
    ) -> Self {
        let usage = response.usage.as_ref();
        let input_tokens = usage.map(|usage| usage.prompt_tokens).unwrap_or(0);
        let output_tokens = usage.map(|usage| usage.completion_tokens).unwrap_or(0);
        let total_tokens = usage
            .map(|usage| usage.total_tokens)
            .filter(|value| *value > 0)
            .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
        Self {
            model: (!response.model.trim().is_empty()).then(|| response.model.clone()),
            input_tokens,
            output_tokens,
            total_tokens,
            cost_usd: usage.and_then(|usage| usage.cost_usd),
            estimated: usage.map(|usage| usage.estimated).unwrap_or(false),
            duration_ms: (duration_ms > 0).then_some(duration_ms),
            time_to_first_token_ms,
        }
    }
}

#[cfg(test)]
mod orbit_agent_extra_tests {
    use super::*;

    #[test]
    fn module_title_is_human_readable() {
        assert_eq!(title_from_module("weather-card"), "Weather Card");
        assert_eq!(title_from_module("daily_news"), "Daily News");
    }

    #[test]
    fn parses_structured_surgical_edit_arguments() {
        let parsed = parse_orbit_tool_arguments(&serde_json::json!({
            "operations": [{
                "operation": "edit",
                "path": "mod/weather/index.js",
                "find": "old",
                "replace": "new"
            }]
        }))
        .expect("structured arguments");
        assert_eq!(parsed.operations.len(), 1);
        assert_eq!(parsed.operations[0].operation, "edit");
        assert_eq!(parsed.operations[0].path, "mod/weather/index.js");
        assert_eq!(parsed.operations[0].find.as_deref(), Some("old"));
        assert_eq!(parsed.operations[0].replace.as_deref(), Some("new"));
    }

    #[test]
    fn surgical_edit_replaces_first_exact_match() {
        let updated = apply_surgical_edit("alpha old old", "old", "new").expect("edit");
        assert_eq!(updated, "alpha new old");
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OrbitToolArguments {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    operations: Vec<OrbitToolOperation>,
}

#[derive(Debug, Clone, Deserialize)]
struct OrbitToolOperation {
    #[serde(default)]
    operation: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    find: Option<String>,
    #[serde(default)]
    replace: Option<String>,
}

pub async fn stream_orbit_chat_turn(
    service: ArkOrbitService,
    llm: LlmClient,
    orbit_id: String,
    user_message: String,
    event_tx: mpsc::Sender<OrbitAgentEvent>,
) -> Result<()> {
    let _orbit = service
        .get_orbit(&orbit_id)
        .await?
        .ok_or_else(|| anyhow!("ArkOrbit: orbit '{}' not found", orbit_id))?;
    let mut history = load_history(&service, &orbit_id)?;
    append_message(&service, &orbit_id, "user", &user_message)?;
    let mut assistant_draft = AssistantMessageDraft::create(&service, &orbit_id, "")?;
    let mut assistant_visible = String::new();
    let mut read_context = Vec::new();
    let mut usage = OrbitChatUsage::default();

    for round in 0..=READ_ROUND_LIMIT {
        let system_prompt = build_system_prompt(&service, &orbit_id).await?;
        let current_user = if round == 0 {
            user_message.clone()
        } else {
            render_read_resume_message(&read_context)
        };
        let persist_prefix = assistant_visible.clone();
        let (visible, reads, _writes, turn_usage) = run_single_stream(
            &service,
            &llm,
            &orbit_id,
            &system_prompt,
            &current_user,
            &history,
            &event_tx,
            &mut assistant_draft,
            &persist_prefix,
            true,
        )
        .await?;
        usage.merge(turn_usage);
        assistant_visible.push_str(&visible);
        if reads.is_empty() || round == READ_ROUND_LIMIT {
            break;
        }
        read_context = satisfy_reads(&service, &orbit_id, &reads, &event_tx)?;
        history.push(ConversationMessage {
            role: "user".to_string(),
            content: current_user,
            _timestamp: chrono::Utc::now(),
        });
        history.push(ConversationMessage {
            role: "assistant".to_string(),
            content: visible,
            _timestamp: chrono::Utc::now(),
        });
    }

    if assistant_visible.trim().is_empty() {
        let system_prompt = build_system_prompt(&service, &orbit_id).await?;
        let repair_user = render_empty_turn_repair_message(&user_message, &read_context);
        let (visible, _reads, _writes, turn_usage) = run_single_stream(
            &service,
            &llm,
            &orbit_id,
            &system_prompt,
            &repair_user,
            &history,
            &event_tx,
            &mut assistant_draft,
            "",
            true,
        )
        .await?;
        usage.merge(turn_usage);
        assistant_visible.push_str(&visible);
    }

    if assistant_visible.trim().is_empty() {
        let message = "Orbit did not produce a visible answer or file operation for this turn.";
        let _ = event_tx
            .send(OrbitAgentEvent::Error(message.to_string()))
            .await;
        assistant_visible.push_str(message);
    }

    assistant_draft.persist_content(assistant_visible.trim())?;
    assistant_draft.persist_usage(&usage)?;
    if !usage.is_empty() {
        let _ = event_tx.send(OrbitAgentEvent::Usage(usage)).await;
    }
    let _ = event_tx.send(OrbitAgentEvent::Done).await;
    Ok(())
}

async fn run_single_stream(
    service: &ArkOrbitService,
    llm: &LlmClient,
    orbit_id: &str,
    system_prompt: &str,
    user_message: &str,
    history: &[ConversationMessage],
    event_tx: &mpsc::Sender<OrbitAgentEvent>,
    assistant_draft: &mut AssistantMessageDraft,
    persist_prefix: &str,
    use_file_operations: bool,
) -> Result<(String, Vec<String>, usize, OrbitChatUsage)> {
    let (token_tx, mut token_rx) = mpsc::channel::<StreamEvent>(128);
    let llm = llm.clone();
    let system_prompt = system_prompt.to_string();
    let user_message_owned = user_message.to_string();
    let history_owned = history.to_vec();
    let actions = if use_file_operations {
        vec![orbit_operations_action()]
    } else {
        Vec::new()
    };
    let handle = tokio::spawn(async move {
        llm.chat_with_history_stream(
            &system_prompt,
            &user_message_owned,
            &history_owned,
            &[],
            &actions,
            token_tx,
        )
        .await
    });

    let mut assistant_visible = String::new();
    let mut reads = Vec::new();
    let mut writes = 0usize;
    let mut saw_stream_token = false;
    let mut buffered_content = String::new();
    let started_at = std::time::Instant::now();
    let mut first_token_ms: Option<u64> = None;

    while let Some(event) = token_rx.recv().await {
        if let StreamEvent::Token(text) = event {
            saw_stream_token = true;
            first_token_ms.get_or_insert_with(|| {
                (started_at
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64)
                    .max(1)
            });
            if use_file_operations {
                buffered_content.push_str(&text);
            } else {
                emit_visible_text(
                    event_tx,
                    assistant_draft,
                    persist_prefix,
                    &mut assistant_visible,
                    &text,
                )
                .await?;
            }
        }
    }

    let response = handle.await??;
    let duration_ms = started_at
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let first_content_ms = first_token_ms.or_else(|| {
        (!response.content.is_empty() || !buffered_content.is_empty()).then_some(duration_ms.max(1))
    });
    let usage = OrbitChatUsage::from_response(&response, duration_ms, first_content_ms);
    if use_file_operations {
        let model_content = if response.content.is_empty() {
            buffered_content
        } else {
            response.content.clone()
        };
        let operation_payloads =
            collect_orbit_operation_payloads(&response.tool_calls, &model_content);
        if operation_payloads.is_empty() {
            if !model_content.trim().is_empty() {
                emit_visible_text(
                    event_tx,
                    assistant_draft,
                    persist_prefix,
                    &mut assistant_visible,
                    &model_content,
                )
                .await?;
            }
        } else {
            apply_orbit_operation_payloads(
                service,
                orbit_id,
                operation_payloads,
                event_tx,
                &mut assistant_visible,
                &mut reads,
                &mut writes,
                assistant_draft,
                persist_prefix,
            )
            .await?;
        }
    } else if !saw_stream_token && !response.content.is_empty() {
        emit_visible_text(
            event_tx,
            assistant_draft,
            persist_prefix,
            &mut assistant_visible,
            &response.content,
        )
            .await?;
    }
    Ok((assistant_visible, reads, writes, usage))
}

async fn emit_visible_text(
    event_tx: &mpsc::Sender<OrbitAgentEvent>,
    assistant_draft: &mut AssistantMessageDraft,
    persist_prefix: &str,
    assistant_visible: &mut String,
    text: &str,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    assistant_visible.push_str(text);
    assistant_draft.persist_content(&combine_visible_content(persist_prefix, assistant_visible))?;
    let _ = event_tx
        .send(OrbitAgentEvent::Token(text.to_string()))
        .await;
    Ok(())
}

async fn apply_orbit_operation_payloads(
    service: &ArkOrbitService,
    orbit_id: &str,
    payloads: Vec<serde_json::Value>,
    event_tx: &mpsc::Sender<OrbitAgentEvent>,
    assistant_visible: &mut String,
    reads: &mut Vec<String>,
    writes: &mut usize,
    assistant_draft: &mut AssistantMessageDraft,
    persist_prefix: &str,
) -> Result<()> {
    for payload in payloads {
        let args = parse_orbit_tool_arguments(&payload)?;
        if let Some(message) = args
            .message
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            emit_visible_text(
                event_tx,
                assistant_draft,
                persist_prefix,
                assistant_visible,
                message,
            )
            .await?;
            if !assistant_visible.ends_with('\n') {
                emit_visible_text(
                    event_tx,
                    assistant_draft,
                    persist_prefix,
                    assistant_visible,
                    "\n",
                )
                .await?;
            }
        }

        for operation in args.operations {
            let path = operation.path.trim().to_string();
            if path.is_empty() {
                return Err(anyhow!("ArkOrbit operation is missing a path"));
            }
            match normalize_orbit_operation_kind(&operation)? {
                OrbitStructuredOperationKind::Read => {
                    validate_readable_orbit_path(&path)?;
                    emit_status(
                        event_tx,
                        assistant_draft,
                        format_file_activity("reading", &path),
                    )
                    .await?;
                    reads.push(path.clone());
                    let _ = event_tx.send(OrbitAgentEvent::ReadRequested { path }).await;
                }
                OrbitStructuredOperationKind::Write => {
                    let Some(content) = operation.content else {
                        emit_status(
                            event_tx,
                            assistant_draft,
                            format!(
                                "The model selected {}, but did not include the JavaScript content yet. I'm requesting the complete file.",
                                path
                            ),
                        )
                        .await?;
                        continue;
                    };
                    validate_writable_orbit_path(&path)?;
                    emit_status(
                        event_tx,
                        assistant_draft,
                        format_file_activity("saving", &path),
                    )
                    .await?;
                    service.write_orbit_file(orbit_id, &path, &content)?;
                    upsert_widget_registry_for_module(service, orbit_id, &path)?;
                    let line = format_file_update_line(OrbitFileOperation::Wrote, &path);
                    append_visible_line(assistant_visible, &line);
                    assistant_draft.persist_content(&combine_visible_content(
                        persist_prefix,
                        assistant_visible,
                    ))?;
                    *writes += 1;
                    let _ = event_tx
                        .send(OrbitAgentEvent::FileWritten {
                            path: path.clone(),
                            operation: OrbitFileOperation::Wrote,
                        })
                        .await;
                    let _ = event_tx
                        .send(OrbitAgentEvent::Token(format!("{}\n", line)))
                        .await;
                }
                OrbitStructuredOperationKind::Edit => {
                    let Some(find) = operation.find else {
                        emit_status(
                            event_tx,
                            assistant_draft,
                            format!(
                                "The model selected {}, but did not include the edit target yet. I'm requesting a valid edit.",
                                path
                            ),
                        )
                        .await?;
                        continue;
                    };
                    let replace = operation.replace.unwrap_or_default();
                    validate_writable_orbit_path(&path)?;
                    emit_status(
                        event_tx,
                        assistant_draft,
                        format_file_activity("saving edits to", &path),
                    )
                    .await?;
                    let current = service.read_orbit_file_text(orbit_id, &path)?;
                    let updated = apply_surgical_edit(&current, &find, &replace)
                        .ok_or_else(|| anyhow!("Edit target was not found in {}", path))?;
                    service.write_orbit_file(orbit_id, &path, &updated)?;
                    upsert_widget_registry_for_module(service, orbit_id, &path)?;
                    let line = format_file_update_line(OrbitFileOperation::Edited, &path);
                    append_visible_line(assistant_visible, &line);
                    assistant_draft.persist_content(&combine_visible_content(
                        persist_prefix,
                        assistant_visible,
                    ))?;
                    *writes += 1;
                    let _ = event_tx
                        .send(OrbitAgentEvent::FileWritten {
                            path: path.clone(),
                            operation: OrbitFileOperation::Edited,
                        })
                        .await;
                    let _ = event_tx
                        .send(OrbitAgentEvent::Token(format!("{}\n", line)))
                        .await;
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrbitStructuredOperationKind {
    Read,
    Write,
    Edit,
}

fn orbit_operations_action() -> ActionDef {
    ActionDef {
        name: ORBIT_OPERATIONS_ACTION.to_string(),
        description: "Apply structured ArkOrbit file operations for the selected canvas. Use read before editing when file contents are needed; every write must include complete file content, and every edit must include an exact find snippet.".to_string(),
        version: "1.0.0".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Optional short user-visible acknowledgement or summary. Do not include file contents here."
                },
                "operations": {
                    "type": "array",
                    "description": "Ordered operations to apply inside the current orbit.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "enum": ["read", "write", "edit"],
                                "description": "read requests an existing file for a follow-up turn; write persists complete content; edit replaces the first exact find snippet."
                            },
                            "path": {
                                "type": "string",
                                "description": "Orbit-relative path. Writable roots: mod/, data/, assets/, index.html, orbit.json."
                            },
                            "content": {
                                "type": "string",
                                "description": "Required for write operations. Complete file contents to persist."
                            },
                            "find": {
                                "type": "string",
                                "description": "Required for edit operations. Exact existing snippet to replace."
                            },
                            "replace": {
                                "type": "string",
                                "description": "Replacement snippet for edit operations. Use an empty string to delete the find snippet."
                            }
                        },
                        "required": ["operation", "path"],
                        "allOf": [
                            {
                                "if": {
                                    "properties": { "operation": { "const": "write" } },
                                    "required": ["operation"]
                                },
                                "then": { "required": ["content"] }
                            },
                            {
                                "if": {
                                    "properties": { "operation": { "const": "edit" } },
                                    "required": ["operation"]
                                },
                                "then": { "required": ["find"] }
                            }
                        ]
                    }
                }
            },
            "required": ["operations"]
        }),
        capabilities: vec!["arkorbit".to_string(), "file_write".to_string()],
        sandbox_mode: None,
        source: ActionSource::System,
        file_path: None,
        authorization: Default::default(),
    }
}

fn collect_orbit_operation_payloads(
    tool_calls: &[ToolCall],
    model_content: &str,
) -> Vec<serde_json::Value> {
    let mut payloads = tool_calls
        .iter()
        .filter_map(|call| orbit_payload_from_tool_call(call))
        .collect::<Vec<_>>();
    if payloads.is_empty() {
        if let Some(payload) = orbit_payload_from_json_text(model_content) {
            payloads.push(payload);
        }
    }
    payloads
}

fn orbit_payload_from_tool_call(call: &ToolCall) -> Option<serde_json::Value> {
    let name = call.name.trim();
    if name.eq_ignore_ascii_case(ORBIT_OPERATIONS_ACTION) {
        return Some(call.arguments.clone());
    }
    if name.eq_ignore_ascii_case("arkorbit_file_write")
        || name.eq_ignore_ascii_case("orbit_file_write")
        || name.eq_ignore_ascii_case("file_write")
    {
        return legacy_file_write_payload(&call.arguments);
    }
    None
}

fn legacy_file_write_payload(arguments: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = arguments.as_object()?;
    let path = obj
        .get("path")
        .or_else(|| obj.get("file_path"))
        .and_then(|value| value.as_str())?;
    let content = obj
        .get("content")
        .or_else(|| obj.get("text"))
        .or_else(|| obj.get("body"))
        .and_then(|value| value.as_str())?;
    Some(serde_json::json!({
        "operations": [{
            "operation": "write",
            "path": path,
            "content": content
        }]
    }))
}

fn orbit_payload_from_json_text(text: &str) -> Option<serde_json::Value> {
    let value = parse_json_payload_text(text)?;
    if value.get("operations").and_then(|v| v.as_array()).is_some() {
        return Some(value);
    }
    if let Some(operations) = value.get("arkorbit_operations").and_then(|v| v.as_array()) {
        return Some(serde_json::json!({
            "message": value.get("message").cloned().unwrap_or(serde_json::Value::Null),
            "operations": operations
        }));
    }
    let calls = value.get("agent_tool_calls").and_then(|v| v.as_array())?;
    for call in calls {
        let Some(name) = call.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let arguments = call
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let synthetic = ToolCall {
            id: "fallback_json".to_string(),
            name: name.to_string(),
            arguments,
        };
        if let Some(payload) = orbit_payload_from_tool_call(&synthetic) {
            return Some(payload);
        }
    }
    None
}

fn parse_json_payload_text(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    parse_fenced_json_payload(trimmed)
}

fn parse_fenced_json_payload(text: &str) -> Option<serde_json::Value> {
    let start = text.find("```")?;
    let after_ticks = &text[start + 3..];
    let newline = after_ticks.find('\n')?;
    let header = after_ticks[..newline].trim().to_ascii_lowercase();
    if !header.is_empty() && header != "json" {
        return None;
    }
    let body_start = start + 3 + newline + 1;
    let rest = &text[body_start..];
    let end = rest.find("```")?;
    serde_json::from_str(rest[..end].trim()).ok()
}

fn parse_orbit_tool_arguments(value: &serde_json::Value) -> Result<OrbitToolArguments> {
    let normalized = normalize_orbit_tool_arguments_value(value)?;
    Ok(serde_json::from_value(normalized)?)
}

fn normalize_orbit_tool_arguments_value(value: &serde_json::Value) -> Result<serde_json::Value> {
    if let Some(raw) = value.as_str() {
        return serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|error| anyhow!("Invalid ArkOrbit operation JSON string: {}", error));
    }
    if value.get("operations").and_then(|v| v.as_array()).is_some() {
        return Ok(value.clone());
    }
    if let Some(operations) = value.get("arkorbit_operations").and_then(|v| v.as_array()) {
        return Ok(serde_json::json!({
            "message": value.get("message").cloned().unwrap_or(serde_json::Value::Null),
            "operations": operations
        }));
    }
    Err(anyhow!("Invalid ArkOrbit operation payload"))
}

fn normalize_orbit_operation_kind(
    operation: &OrbitToolOperation,
) -> Result<OrbitStructuredOperationKind> {
    let raw = operation.operation.trim().to_ascii_lowercase();
    match raw.as_str() {
        "read" => Ok(OrbitStructuredOperationKind::Read),
        "write" | "create" | "replace" => Ok(OrbitStructuredOperationKind::Write),
        "edit" | "patch" | "update" => Ok(OrbitStructuredOperationKind::Edit),
        "" if operation.content.is_some() => Ok(OrbitStructuredOperationKind::Write),
        "" if operation.find.is_some() => Ok(OrbitStructuredOperationKind::Edit),
        _ => Err(anyhow!(
            "Unknown ArkOrbit operation '{}'",
            operation.operation
        )),
    }
}

fn append_visible_line(assistant_visible: &mut String, line: &str) {
    if !assistant_visible.is_empty() && !assistant_visible.ends_with('\n') {
        assistant_visible.push('\n');
    }
    assistant_visible.push_str(line);
    assistant_visible.push('\n');
}

fn format_file_update_line(operation: OrbitFileOperation, path: &str) -> String {
    format!("I {} {}.", operation.past_tense(), path)
}

fn format_file_activity(action: &str, path: &str) -> String {
    match file_kind_label(path) {
        Some(kind) => format!("I'm {} the {} file: {}", action, kind, path),
        None => format!("I'm {} this file: {}", action, path),
    }
}

fn file_kind_label(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".jsx") {
        Some("JavaScript")
    } else if lower.ends_with(".json") || lower.ends_with(".jsonl") {
        Some("JSON")
    } else if lower.ends_with(".css") {
        Some("CSS")
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        Some("HTML")
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        Some("Markdown")
    } else {
        None
    }
}

fn apply_surgical_edit(current: &str, find: &str, replace: &str) -> Option<String> {
    if find.is_empty() {
        return None;
    }
    if current.contains(find) {
        return Some(current.replacen(find, replace, 1));
    }
    let trimmed_find = trim_one_outer_newline(find);
    if trimmed_find != find && !trimmed_find.is_empty() && current.contains(trimmed_find) {
        let trimmed_replace = trim_one_outer_newline(replace);
        return Some(current.replacen(trimmed_find, trimmed_replace, 1));
    }
    None
}

fn trim_one_outer_newline(value: &str) -> &str {
    let without_leading = value
        .strip_prefix("\r\n")
        .or_else(|| value.strip_prefix('\n'))
        .unwrap_or(value);
    without_leading
        .strip_suffix("\r\n")
        .or_else(|| without_leading.strip_suffix('\n'))
        .unwrap_or(without_leading)
}

fn satisfy_reads(
    service: &ArkOrbitService,
    orbit_id: &str,
    reads: &[String],
    event_tx: &mpsc::Sender<OrbitAgentEvent>,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for path in reads {
        let body = match service.read_orbit_file_text(orbit_id, path) {
            Ok(body) => body,
            Err(error) => {
                let message = format!("Could not read {}: {}", path, error);
                let _ = event_tx.try_send(OrbitAgentEvent::Error(message.clone()));
                return Err(anyhow!(message));
            }
        };
        let truncated = if body.len() > MAX_READ_BYTES {
            body.chars().take(MAX_READ_BYTES).collect::<String>()
        } else {
            body
        };
        out.push((path.clone(), truncated));
    }
    Ok(out)
}

fn render_read_resume_message(reads: &[(String, String)]) -> String {
    let files = reads
        .iter()
        .map(|(path, body)| {
            serde_json::json!({
                "path": path,
                "content": body,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_string_pretty(&serde_json::json!({ "files": files }))
        .unwrap_or_else(|_| "{\"files\":[]}".to_string());
    format!(
        "The requested orbit file contents are available below as JSON. Continue the same task using these files. If the user's intent is inspection, explanation, or diagnosis, answer directly in plain prose. If additional orbit file reads, writes, or edits are needed, call {} with the next operations.\n\n{}",
        ORBIT_OPERATIONS_ACTION, payload
    )
}

fn render_empty_turn_repair_message(user_message: &str, reads: &[(String, String)]) -> String {
    let mut message = format!(
        "Complete the user's Orbit request now. The previous response produced neither visible text nor a concrete file operation.\n\nOriginal request:\n{}\n\nIf orbit file reads, writes, or edits are needed to satisfy the request, call {} with concrete operations. If no file operation is needed, answer directly in plain prose. Do not call the tool with an empty operations array, and do not return an empty response.",
        user_message, ORBIT_OPERATIONS_ACTION
    );
    if !reads.is_empty() {
        message.push_str("\n\nAlready-read orbit file contents:\n");
        message.push_str(&render_read_context_json(reads));
    }
    message
}

fn render_read_context_json(reads: &[(String, String)]) -> String {
    let files = reads
        .iter()
        .map(|(path, body)| {
            serde_json::json!({
                "path": path,
                "content": body,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({ "files": files }))
        .unwrap_or_else(|_| "{\"files\":[]}".to_string())
}

fn upsert_widget_registry_for_module(
    service: &ArkOrbitService,
    orbit_id: &str,
    path: &str,
) -> Result<()> {
    let Some(module) = path
        .strip_prefix("mod/")
        .and_then(|value| value.strip_suffix("/index.js"))
    else {
        return Ok(());
    };
    if module.trim().is_empty() || module.contains('/') {
        return Ok(());
    }

    let mut widgets = service
        .read_orbit_file_text(orbit_id, "data/widgets.json")
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| {
            if let Some(list) = value.as_array() {
                Some(list.clone())
            } else {
                value
                    .get("widgets")
                    .and_then(|widgets| widgets.as_array())
                    .cloned()
            }
        })
        .unwrap_or_default();

    let exists = widgets.iter().any(|widget| {
        widget
            .get("module")
            .and_then(|value| value.as_str())
            .map(|value| value == module)
            .unwrap_or(false)
    });
    if !exists {
        let offset = widgets.len() as i64;
        widgets.push(serde_json::json!({
            "id": module,
            "module": module,
            "title": title_from_module(module),
            "left": 100 + offset * 380,
            "top": 80 + offset * 40,
            "width": 340
        }));
    }
    service.write_orbit_file(
        orbit_id,
        "data/widgets.json",
        &serde_json::to_string_pretty(&widgets)?,
    )
}

fn title_from_module(module: &str) -> String {
    module
        .split(|ch| ch == '-' || ch == '_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn build_system_prompt(
    service: &ArkOrbitService,
    orbit_id: &str,
) -> Result<String> {
    let orbit = service
        .get_orbit(orbit_id)
        .await?
        .ok_or_else(|| anyhow!("ArkOrbit: orbit '{}' not found", orbit_id))?;
    let files = service.list_orbit_files(orbit_id)?;
    let file_tree = files
        .iter()
        .map(|entry| format!("- {} ({} bytes)", entry.path, entry.bytes))
        .collect::<Vec<_>>()
        .join("\n");
    let catalog = service.l0_skill_catalog();
    let instructions = orbit.agent_instructions.unwrap_or_default();
    let now_utc = chrono::Utc::now();
    let current_datetime = now_utc.to_rfc3339();
    let current_date = now_utc.format("%A, %B %d, %Y").to_string();
    let operation_protocol = format!(
        "File operation protocol:\n\
- Use the structured {action} tool for every orbit file read, write, or edit.\n\
- If native tool calling is unavailable, return JSON only with this exact fallback shape: {{\"agent_tool_calls\":[{{\"name\":\"{action}\",\"arguments\":{{\"message\":\"short acknowledgement\",\"operations\":[{{\"operation\":\"write\",\"path\":\"mod/<short-widget-id>/index.js\",\"content\":\"complete module\"}}]}}}}]}}.\n\
- For an existing widget/file, use a read operation first if the exact current contents are needed, then use an edit operation with the smallest exact find/replace snippet.\n\
- For a new widget, use a write operation with complete file contents.\n\
- Do not emit XML-style file commands such as <file>, <edit>, or <read>; prose is not a file operation protocol.",
        action = ORBIT_OPERATIONS_ACTION
    );
    Ok(format!(
        "You are the agent inside an ArkOrbit canvas. The user owns this canvas.\n\
Files outside this orbit are off-limits.\n\
Writable paths are structurally limited to mod/, data/, assets/, index.html, and orbit.json.\n\
Current date/time:\n- UTC: {}\n- Date: {}\n\n\
{}\n\n\
Available L0 widgets and runtime notes:\n{}\n\n\
Canvas behavior:\n\
- index.html is a stable canvas host. Do not rewrite it for ordinary widget requests.\n\
- For a new widget, write one small JavaScript module at mod/<short-widget-id>/index.js.\n\
- The module must export render(el, ctx = {{}}). The host automatically registers, mounts, reloads, and makes it draggable.\n\
- Every write operation must include the complete JavaScript file content in the content field. Never call a write operation with only a path.\n\
- Widget left/top/width/height values in data/widgets.json are user layout state. Preserve them for existing widgets unless the user asks to move, resize, rearrange, or replace the whole canvas.\n\
- For an edit to an existing widget, first read the target file if needed, then replace only the smallest exact snippet that satisfies the request.\n\
- When the user's intent is to replace the whole canvas state, treat the current widget registry as disposable: write the desired final widget registry and the needed modules directly, and do not read existing files unless the final result depends on their current contents.\n\
- When the user wants a previously available widget brought back into the canvas, first check whether its module still exists. If it exists, read or edit data/widgets.json and add a registry entry for that module. If it was deleted, recreate the module from the user's request and conversation context.\n\
- Do not re-emit a whole existing widget file for a small edit. Replace only the smallest exact snippet that satisfies the request.\n\
- Keep generated widget modules browser-only and self-contained. Put styling inside the rendered subtree or a small injected style element.\n\n\
Live data rules:\n\
- Render the widget shell immediately, then fetch/update data asynchronously so a new widget is visible right away.\n\
- For public HTTPS feeds, news, RSS, pricing, weather, or market data, prefer ctx.fetchText(url), ctx.fetchJson(url), or ctx.fetchPublic(url) instead of direct browser fetch(url). Direct cross-origin browser fetches often fail because of CORS.\n\
- For general latest-news widgets, do not default to Reddit, X/Twitter, forum posts, or social-media search unless the user explicitly asks for that source. Prefer public news/RSS/search feeds from news providers or aggregators, label the source in the UI, and show a clear error if a public source is unavailable.\n\
- Do not use JSONP or script-tag injection for live news data. Use ctx.fetchText/ctx.fetchJson and parse the response safely in the widget.\n\
- Use only public unauthenticated URLs in browser widgets. Never embed API keys, bearer tokens, cookies, or secrets. If a source needs credentials, show a clear non-secret setup/error state instead of hardcoding credentials.\n\
- For auto-refresh widgets, perform the first fetch immediately and then use setInterval for the requested cadence; return a cleanup function that clears the interval.\n\n\
Orbit metadata:\n- id: {}\n- name: {}\n- instructions: {}\n\n\
Current orbit files:\n{}\n\n\
Execution rules:\n\
- If the user wants the canvas state to be different, make the necessary file changes in the same turn.\n\
- Start the visible response with one short natural acknowledgement tailored to the user's request, for example: Got it, I'll build the weather widget for you.\n\
- Do not ask for confirmation before writing orbit files unless a safety-critical detail is missing.\n\
- Resolve the user's intended timeframe before using time-sensitive data: explicit dates, months, years, events, or phrases like \"March 2020\" override today's date. If no timeframe is given, default to the current date/time above.\n\
- Treat \"live\" as live for the user's requested timeframe. For example, \"live corona dashboard for March 2020\" means data from March 2020, not today's data.\n\
- For current, recent, latest, pricing, market, news, weather, or other time-sensitive data, label the widget with the resolved timeframe. Do not invent an older snapshot date when the user did not ask for one.\n\
- Do not claim data is live unless the widget actually fetches a live public source at runtime. If live data is not available, label values as approximate/example data for the resolved timeframe and tell the user what source should be checked.\n\
- For widget creation, emit JavaScript only at mod/<short-widget-id>/index.js unless the user explicitly asks for assets or data.\n\
- Do not say a file was created, updated, edited, written, or placed unless you call the matching structured operation in the same response.\n\
- After file operations, summarize briefly in plain prose for a human, including what changed and which files were touched.",
        current_datetime,
        current_date,
        operation_protocol,
        catalog,
        orbit.id,
        orbit.name,
        if instructions.trim().is_empty() {
            "(none)"
        } else {
            instructions.trim()
        },
        if file_tree.trim().is_empty() {
            "(none)"
        } else {
            &file_tree
        }
    ))
}

fn messages_path(service: &ArkOrbitService, orbit_id: &str) -> Result<std::path::PathBuf> {
    Ok(service.orbit_dir(orbit_id)?.join("messages.jsonl"))
}

fn append_message(
    service: &ArkOrbitService,
    orbit_id: &str,
    role: &str,
    content: &str,
) -> Result<OrbitChatMessage> {
    let path = messages_path(service, orbit_id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let message = OrbitChatMessage {
        id: Uuid::new_v4().to_string(),
        role: role.to_string(),
        content: content.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        cost_usd: None,
        estimated: None,
        duration_ms: None,
        time_to_first_token_ms: None,
    };
    let mut line = serde_json::to_string(&message)?;
    line.push('\n');
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(message)
}

struct AssistantMessageDraft {
    path: std::path::PathBuf,
    message: OrbitChatMessage,
    has_visible_content: bool,
}

impl AssistantMessageDraft {
    fn create(service: &ArkOrbitService, orbit_id: &str, content: &str) -> Result<Self> {
        let path = messages_path(service, orbit_id)?;
        let message = append_message(service, orbit_id, "assistant", content)?;
        Ok(Self {
            path,
            message,
            has_visible_content: false,
        })
    }

    fn persist_status_if_empty(&mut self, content: &str) -> Result<()> {
        if self.has_visible_content {
            return Ok(());
        }
        self.persist_content_internal(content, false)
    }

    fn persist_content(&mut self, content: &str) -> Result<()> {
        self.persist_content_internal(content, !content.trim().is_empty())
    }

    fn persist_usage(&mut self, usage: &OrbitChatUsage) -> Result<()> {
        if usage.is_empty() {
            return Ok(());
        }
        self.message.model = usage.model.clone();
        self.message.input_tokens = (usage.input_tokens > 0).then_some(usage.input_tokens);
        self.message.output_tokens = (usage.output_tokens > 0).then_some(usage.output_tokens);
        self.message.total_tokens = (usage.total_tokens > 0).then_some(usage.total_tokens);
        self.message.cost_usd = usage.cost_usd;
        self.message.estimated =
            (usage.input_tokens > 0 || usage.output_tokens > 0 || usage.total_tokens > 0)
                .then_some(usage.estimated);
        self.message.duration_ms = usage.duration_ms;
        self.message.time_to_first_token_ms = usage.time_to_first_token_ms;
        rewrite_message_by_id(&self.path, &self.message)
    }

    fn persist_content_internal(&mut self, content: &str, visible: bool) -> Result<()> {
        self.message.content = content.to_string();
        if visible {
            self.has_visible_content = true;
        }
        rewrite_message_by_id(&self.path, &self.message)
    }
}

async fn emit_status(
    event_tx: &mpsc::Sender<OrbitAgentEvent>,
    assistant_draft: &mut AssistantMessageDraft,
    message: String,
) -> Result<()> {
    assistant_draft.persist_status_if_empty(&message)?;
    let _ = event_tx.send(OrbitAgentEvent::Status { message }).await;
    Ok(())
}

fn rewrite_message_by_id(path: &std::path::Path, replacement: &OrbitChatMessage) -> Result<()> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut replaced = false;
    let mut lines = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let is_target = serde_json::from_str::<OrbitChatMessage>(line)
            .map(|message| message.id == replacement.id)
            .unwrap_or(false);
        if is_target {
            lines.push(serde_json::to_string(replacement)?);
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(serde_json::to_string(replacement)?);
    }
    let mut next = lines.join("\n");
    next.push('\n');
    std::fs::write(path, next)?;
    Ok(())
}

fn combine_visible_content(prefix: &str, current: &str) -> String {
    let prefix = prefix.trim_end();
    let current = current.trim_start();
    if prefix.is_empty() {
        current.trim().to_string()
    } else if current.is_empty() {
        prefix.trim().to_string()
    } else {
        format!("{}\n{}", prefix, current).trim().to_string()
    }
}

fn load_history(service: &ArkOrbitService, orbit_id: &str) -> Result<Vec<ConversationMessage>> {
    let path = messages_path(service, orbit_id)?;
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut messages = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: OrbitChatMessage = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(target: "arkorbit.chat", error = %error, "Skipping malformed orbit chat line");
                continue;
            }
        };
        messages.push(ConversationMessage {
            role: parsed.role,
            content: parsed.content,
            _timestamp: chrono::DateTime::parse_from_rfc3339(&parsed.created_at)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        });
    }
    let keep_from = messages.len().saturating_sub(HISTORY_LIMIT);
    Ok(messages.into_iter().skip(keep_from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_json_extracts_operation_payload() {
        let payload = orbit_payload_from_json_text(
            r#"{"agent_tool_calls":[{"name":"arkorbit_apply_operations","arguments":{"message":"Done.","operations":[{"operation":"write","path":"mod/a/index.js","content":"export function render() {}"}]}}]}"#,
        )
        .expect("payload");
        let parsed = parse_orbit_tool_arguments(&payload).expect("arguments");
        assert_eq!(parsed.message.as_deref(), Some("Done."));
        assert_eq!(parsed.operations.len(), 1);
        assert_eq!(parsed.operations[0].operation, "write");
        assert_eq!(parsed.operations[0].path, "mod/a/index.js");
    }

    #[test]
    fn legacy_file_write_tool_call_maps_to_structured_write() {
        let call = ToolCall {
            id: "1".to_string(),
            name: "arkorbit_file_write".to_string(),
            arguments: serde_json::json!({
                "path": "mod/a/index.js",
                "content": "export function render() {}"
            }),
        };
        let payload = orbit_payload_from_tool_call(&call).expect("payload");
        let parsed = parse_orbit_tool_arguments(&payload).expect("arguments");
        assert_eq!(parsed.operations[0].operation, "write");
        assert_eq!(parsed.operations[0].path, "mod/a/index.js");
    }

    #[test]
    fn operation_kind_can_be_inferred_from_write_content() {
        let operation = OrbitToolOperation {
            operation: String::new(),
            path: "mod/a/index.js".to_string(),
            content: Some("export function render() {}".to_string()),
            find: None,
            replace: None,
        };
        assert_eq!(
            normalize_orbit_operation_kind(&operation).expect("kind"),
            OrbitStructuredOperationKind::Write
        );
    }

    #[test]
    fn read_resume_context_is_json_not_file_tags() {
        let rendered = render_read_resume_message(&[(
            "mod/a/index.js".to_string(),
            "export function render() {}".to_string(),
        )]);
        assert!(!rendered.contains("<file-content"));
        assert!(rendered.contains("\"path\": \"mod/a/index.js\""));
        assert!(rendered.contains("\"files\""));
    }
}
