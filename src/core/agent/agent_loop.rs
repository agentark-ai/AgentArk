//! Thin chat turn entry point and shared helpers for the model-routed tool loop.

use super::*;
use std::collections::HashSet;

const AGENT_TURN_LOOP_MAX_ITERATIONS_DEFAULT: usize = 6;
const AGENT_TURN_LOOP_MAX_CANDIDATES_DEFAULT: usize = 5;
const AGENT_TURN_LOOP_BASE_TIMEOUT_MS_DEFAULT: u64 = 180_000;
const AGENT_TURN_LOOP_MAX_TIMEOUT_MS_DEFAULT: u64 = 420_000;

#[derive(Debug)]
pub(super) struct AgentLoopToolCallParse {
    pub(super) calls: Vec<crate::core::llm::ToolCall>,
    pub(super) rejected: Vec<String>,
}

fn env_usize(name: &str, default_value: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default_value)
        .clamp(min, max)
}

fn env_u64(name: &str, default_value: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default_value)
        .clamp(min, max)
}

pub(super) fn agent_loop_max_iterations() -> usize {
    env_usize(
        "AGENTARK_AGENT_TURN_LOOP_MAX_ITERATIONS",
        AGENT_TURN_LOOP_MAX_ITERATIONS_DEFAULT,
        1,
        24,
    )
}

pub(super) fn agent_loop_max_candidates() -> usize {
    env_usize(
        "AGENTARK_AGENT_TURN_LOOP_MAX_CANDIDATES",
        AGENT_TURN_LOOP_MAX_CANDIDATES_DEFAULT,
        1,
        16,
    )
}

pub(super) fn agent_loop_timeout_ms(
    prompt_chars: usize,
    action_count: usize,
    iteration: usize,
    _app_delivery_pending: bool,
) -> u64 {
    let base = env_u64(
        "AGENTARK_AGENT_TURN_LOOP_BASE_TIMEOUT_MS",
        AGENT_TURN_LOOP_BASE_TIMEOUT_MS_DEFAULT,
        30_000,
        600_000,
    );
    let max_budget = env_u64(
        "AGENTARK_AGENT_TURN_LOOP_MAX_TIMEOUT_MS",
        AGENT_TURN_LOOP_MAX_TIMEOUT_MS_DEFAULT,
        base,
        900_000,
    );
    let prompt_budget_ms = ((prompt_chars as u64) / 1_000).saturating_mul(4_000);
    let action_budget_ms = ((action_count as u64) / 12).saturating_mul(8_000);
    let continuation_budget_ms = iteration.saturating_sub(1) as u64 * 15_000;
    base.saturating_add(prompt_budget_ms)
        .saturating_add(action_budget_ms)
        .saturating_add(continuation_budget_ms)
        .min(max_budget)
}

pub(super) fn recent_artifacts_for_prompt(
    recent_artifacts: &[ConversationArtifactContext],
) -> Vec<serde_json::Value> {
    recent_artifacts
        .iter()
        .take(8)
        .map(|artifact| {
            serde_json::json!({
                "artifact_type": &artifact.artifact_type,
                "artifact_id": &artifact.artifact_id,
                "title": safe_truncate(&crate::security::redact_secret_input(&artifact.title).text, 180),
                "summary": safe_truncate(&crate::security::redact_secret_input(&artifact.summary).text, 320),
                "url": &artifact.url,
                "related_actions": &artifact.related_actions,
                "updated_at": &artifact.updated_at,
            })
        })
        .collect()
}

pub(super) fn attachment_hints_for_prompt(
    request_hints: &RequestExecutionHints,
) -> Vec<serde_json::Value> {
    request_hints
        .attachments
        .iter()
        .map(|attachment| {
            serde_json::json!({
                "upload_id": &attachment.upload_id,
                "kind": &attachment.kind,
                "content_type": attachment.content_type.as_ref(),
                "document_id": attachment.document_id.as_ref(),
            })
        })
        .collect()
}

pub(super) fn parse_agent_loop_tool_calls(
    response: &crate::core::llm::LlmResponse,
    allowed_action_names: &HashSet<String>,
) -> AgentLoopToolCallParse {
    let mut rejected = Vec::new();
    let mut calls = Vec::new();
    let streamed_app_blocks =
        crate::core::llm::stream_blocks::parse_stream_blocks_from_text(&response.content);

    for call in &response.tool_calls {
        if allowed_action_names.contains(&call.name) {
            calls.push(merge_streamed_app_blocks_into_tool_call(
                call.clone(),
                &streamed_app_blocks,
            ));
        } else {
            rejected.push(call.name.clone());
        }
    }

    if !calls.is_empty() {
        return AgentLoopToolCallParse { calls, rejected };
    }

    if allowed_action_names.contains("app_deploy") && streamed_app_blocks.has_operations() {
        calls.push(synthetic_app_deploy_call_from_stream_blocks(
            &streamed_app_blocks,
        ));
        return AgentLoopToolCallParse { calls, rejected };
    }

    let Some(payload) = super::tool_responses::extract_json_object_from_text(&response.content)
    else {
        return AgentLoopToolCallParse { calls, rejected };
    };
    let Some(tool_calls) = payload
        .get("agent_tool_calls")
        .and_then(|value| value.as_array())
    else {
        return AgentLoopToolCallParse { calls, rejected };
    };

    for item in tool_calls {
        let Some(name) = item.get("name").and_then(|value| value.as_str()) else {
            rejected.push("missing_tool_name".to_string());
            continue;
        };
        if !allowed_action_names.contains(name) {
            rejected.push(name.to_string());
            continue;
        }
        let arguments = item
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        calls.push(merge_streamed_app_blocks_into_tool_call(
            crate::core::llm::ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.to_string(),
                arguments,
            },
            &streamed_app_blocks,
        ));
    }

    AgentLoopToolCallParse { calls, rejected }
}

#[allow(dead_code)]
pub(super) fn tool_result_completion_success(result: &str) -> Option<bool> {
    let completion = crate::runtime::parse_watch_completion(result)
        .or_else(|| crate::runtime::parse_schedule_task_completion(result))
        .or_else(|| crate::runtime::parse_delegate_completion(result));
    if let Some(completion) = completion {
        let status = completion.status.trim();
        return Some(matches!(status, "completed" | "succeeded" | "success"));
    }
    let parsed = super::tool_responses::extract_json_object_from_text(result)?;
    match parsed.get("status").and_then(|value| value.as_str()) {
        Some("ok" | "completed" | "succeeded" | "success") => Some(true),
        Some("error" | "failed" | "needs_input" | "approval_required") => Some(false),
        _ => None,
    }
}

fn stream_block_files_json(
    blocks: &crate::core::llm::stream_blocks::ParsedStreamBlocks,
) -> serde_json::Value {
    serde_json::Value::Object(
        blocks
            .files
            .iter()
            .map(|(path, content)| (path.clone(), serde_json::Value::String(content.clone())))
            .collect(),
    )
}

fn stream_block_file_patches_json(
    blocks: &crate::core::llm::stream_blocks::ParsedStreamBlocks,
) -> serde_json::Value {
    serde_json::Value::Array(
        blocks
            .file_patches
            .iter()
            .map(|patch| {
                serde_json::json!({
                    "path": &patch.path,
                    "patch": &patch.patch,
                })
            })
            .collect(),
    )
}

fn app_delivery_call_has_deployable_source(value: &serde_json::Value) -> bool {
    value
        .get("files")
        .and_then(|files| files.as_object())
        .is_some_and(|files| !files.is_empty())
        || value
            .get("repo_url")
            .and_then(|url| url.as_str())
            .map(str::trim)
            .is_some_and(|url| !url.is_empty())
        || value
            .get("source_path")
            .and_then(|path| path.as_str())
            .map(str::trim)
            .is_some_and(|path| !path.is_empty())
}

fn append_stream_block_file_patches(
    arguments: &mut serde_json::Map<String, serde_json::Value>,
    blocks: &crate::core::llm::stream_blocks::ParsedStreamBlocks,
) {
    if blocks.file_patches.is_empty() {
        return;
    }
    let file_patches = arguments
        .entry("file_patches".to_string())
        .or_insert_with(|| serde_json::json!([]));
    let Some(items) = file_patches.as_array_mut() else {
        return;
    };
    for patch in &blocks.file_patches {
        if !items.iter().any(|item| {
            item.get("path").and_then(|value| value.as_str()) == Some(patch.path.as_str())
        }) {
            items.push(serde_json::json!({
                "path": &patch.path,
                "patch": &patch.patch,
            }));
        }
    }
}

fn append_stream_block_delete_paths(
    arguments: &mut serde_json::Map<String, serde_json::Value>,
    blocks: &crate::core::llm::stream_blocks::ParsedStreamBlocks,
) {
    if blocks.delete_paths.is_empty() {
        return;
    }
    let delete_paths = arguments
        .entry("delete_paths".to_string())
        .or_insert_with(|| serde_json::json!([]));
    let Some(items) = delete_paths.as_array_mut() else {
        return;
    };
    for path in &blocks.delete_paths {
        if !items
            .iter()
            .any(|item| item.as_str() == Some(path.as_str()))
        {
            items.push(serde_json::Value::String(path.clone()));
        }
    }
}

fn merge_streamed_app_blocks_into_tool_call(
    mut call: crate::core::llm::ToolCall,
    blocks: &crate::core::llm::stream_blocks::ParsedStreamBlocks,
) -> crate::core::llm::ToolCall {
    if call.name != "app_deploy" || !blocks.has_operations() {
        return call;
    }
    let mut arguments = call.arguments.as_object().cloned().unwrap_or_default();
    arguments.insert(
        "_streamed_app_delivery".to_string(),
        serde_json::json!(true),
    );
    let existing = serde_json::Value::Object(arguments.clone());
    if !blocks.files.is_empty() && !app_delivery_call_has_deployable_source(&existing) {
        arguments.insert("files".to_string(), stream_block_files_json(blocks));
    }
    append_stream_block_file_patches(&mut arguments, blocks);
    append_stream_block_delete_paths(&mut arguments, blocks);
    if !arguments.contains_key("mode") {
        if blocks.delete_orphans {
            arguments.insert("mode".to_string(), serde_json::json!("replace"));
        } else if !blocks.file_patches.is_empty()
            || arguments
                .get("app_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        {
            arguments.insert("mode".to_string(), serde_json::json!("patch"));
        }
    }
    call.arguments = serde_json::Value::Object(arguments);
    call
}

fn synthetic_app_deploy_call_from_stream_blocks(
    blocks: &crate::core::llm::stream_blocks::ParsedStreamBlocks,
) -> crate::core::llm::ToolCall {
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        "_streamed_app_delivery".to_string(),
        serde_json::json!(true),
    );
    if !blocks.files.is_empty() {
        arguments.insert("files".to_string(), stream_block_files_json(blocks));
    }
    if !blocks.file_patches.is_empty() {
        arguments.insert(
            "file_patches".to_string(),
            stream_block_file_patches_json(blocks),
        );
        arguments.insert("mode".to_string(), serde_json::json!("patch"));
    }
    if !blocks.delete_paths.is_empty() {
        arguments.insert(
            "delete_paths".to_string(),
            serde_json::Value::Array(
                blocks
                    .delete_paths
                    .iter()
                    .map(|path| serde_json::Value::String(path.clone()))
                    .collect(),
            ),
        );
    }
    if blocks.delete_orphans {
        arguments.insert("delete_orphans".to_string(), serde_json::json!(true));
        arguments.insert("mode".to_string(), serde_json::json!("replace"));
    }
    crate::core::llm::ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: "app_deploy".to_string(),
        arguments: serde_json::Value::Object(arguments),
    }
}

pub(super) fn agent_loop_processed_message(
    response: String,
    conversation_id: Option<&str>,
    run_status: &str,
    degradation: Vec<crate::core::DegradationNote>,
    user_outcome: Option<crate::core::UserFacingOutcome>,
    trace_steps: Vec<crate::core::ExecutionStep>,
    turn_records: Vec<AgentTurnRecord>,
    turn_plan: Option<crate::core::ExecutionPlan>,
) -> ProcessedMessage {
    ProcessedMessage {
        response,
        conversation_id: conversation_id.map(ToString::to_string),
        conversation_title: None,
        run_id: None,
        run_status: Some(run_status.to_string()),
        trace_id: None,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        choices: Vec::new(),
        degradation: degradation.clone(),
        attempted_models: user_outcome
            .as_ref()
            .map(|outcome| outcome.attempted_models.clone())
            .unwrap_or_default(),
        user_outcome,
        trace_steps,
        turn_records,
        turn_plan,
    }
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_agent_turn_loop_for_chat(
        &self,
        channel: &str,
        message: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        request_hints: &RequestExecutionHints,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> anyhow::Result<ProcessedMessage> {
        super::turn_loop::run(
            self,
            channel,
            message,
            conversation_id,
            project_id,
            request_hints,
            stream_tx,
        )
        .await
    }
}
