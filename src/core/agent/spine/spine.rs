use super::operational::OperationalEvent;
use super::spine_prompt_bundle::{
    self, ALLOWED_EVOLVABLE_SPINE_FRAGMENT_IDS, SPINE_PROMPT_BUNDLE_VERSION,
};
use super::spine_request::*;
use super::*;
use crate::actions::ActionAuthorization;
use async_trait::async_trait;
use futures::future::join_all;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const PRIMITIVE_NAMES: [&str; 16] = [
    "search",
    "fetch",
    "browse",
    "code_exec",
    "pdf_generate",
    "app_deploy",
    "file_read",
    "file_search",
    "file_write",
    "file_patch",
    "file_delete",
    "skill_manage",
    "resource_rw",
    "memory_rw",
    "action_call",
    "delegate",
];
const ACTION_DIRECTORY_CONTEXT_MAX_ENTRIES: usize = 24;
const ACTION_DIRECTORY_CONTEXT_MAX_NAMES: usize = 192;
const ACTION_DIRECTORY_SCHEMA_FIELD_LIMIT: usize = 8;
const ACTION_DIRECTORY_FIELD_MAX_CHARS: usize = 360;
const SPINE_REQUEST_CONTEXT_DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 32_000;
const SPINE_REQUEST_CONTEXT_BUDGET_RATIO_PERCENT: usize = 3;
const SPINE_REQUEST_CONTEXT_MIN_PREVIEW_CHARS: usize = 512;

#[derive(Debug, Clone, Copy)]
struct ResourceAdapterSpec {
    kind: &'static str,
    ops: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
enum ResourcePayloadRequirement {
    None,
    CustomApiAcquisitionSpec,
    CustomMessagingChannelSpec,
    ExtensionPackInstallTarget,
    ExtensionPackId,
    McpServerConfig,
    ResourceId,
}

#[derive(Debug, Clone, Copy)]
enum ResourcePayloadTransform {
    StripResourceFields,
    ExtensionPack,
    McpManage,
}

#[derive(Debug, Clone, Copy)]
struct ResourceActionContract {
    kind: &'static str,
    ops: &'static [&'static str],
    action_name: &'static str,
    requirement: ResourcePayloadRequirement,
    transform: ResourcePayloadTransform,
    selected_action_for_resolution: Option<&'static str>,
}

const RESOURCE_RW_ADAPTERS: &[ResourceAdapterSpec] = &[
    ResourceAdapterSpec {
        kind: "file",
        ops: &["create", "read", "update", "delete", "list", "status"],
    },
    ResourceAdapterSpec {
        kind: "app_service",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "pause", "resume", "stop",
        ],
    },
    ResourceAdapterSpec {
        kind: "watcher",
        ops: &[
            "create",
            "read",
            "update",
            "delete",
            "list",
            "status",
            "pause",
            "resume",
            "stop",
            "cancel",
            "update_delivery",
        ],
    },
    ResourceAdapterSpec {
        kind: "scheduled_task",
        ops: &[
            "create",
            "read",
            "update",
            "delete",
            "list",
            "status",
            "pause",
            "resume",
            "stop",
            "cancel",
            "update_delivery",
        ],
    },
    ResourceAdapterSpec {
        kind: "notification",
        ops: &["create", "update"],
    },
    ResourceAdapterSpec {
        kind: "background_session",
        ops: &[
            "read",
            "delete",
            "list",
            "status",
            "pause",
            "resume",
            "stop",
            "cancel",
            "update_delivery",
        ],
    },
    ResourceAdapterSpec {
        kind: "browser_profile",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "launch", "close", "resolve",
        ],
    },
    ResourceAdapterSpec {
        kind: "goal",
        ops: &["create", "read", "update", "delete", "list", "status"],
    },
    ResourceAdapterSpec {
        kind: "dashboard",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "pause", "resume", "stop",
        ],
    },
    ResourceAdapterSpec {
        kind: "conversation",
        ops: &["read", "list"],
    },
    ResourceAdapterSpec {
        kind: "activity",
        ops: &["read", "list", "status", "refresh"],
    },
    ResourceAdapterSpec {
        kind: "integration",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "install", "connect", "test",
        ],
    },
    ResourceAdapterSpec {
        kind: "custom_api",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "install", "connect", "test",
            "enable", "disable",
        ],
    },
    ResourceAdapterSpec {
        kind: "custom_messaging_channel",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "enable", "disable", "test",
        ],
    },
    ResourceAdapterSpec {
        kind: "extension_pack",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "install", "connect", "enable",
            "disable", "test",
        ],
    },
    ResourceAdapterSpec {
        kind: "mcp_server",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "install", "connect", "refresh",
        ],
    },
    ResourceAdapterSpec {
        kind: "skill",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "install", "enable", "disable",
            "test",
        ],
    },
    ResourceAdapterSpec {
        kind: "skill_marketplace",
        ops: &[
            "create", "read", "update", "delete", "list", "status", "refresh", "enable", "disable",
        ],
    },
];

const RESOURCE_ACTION_CONTRACTS: &[ResourceActionContract] = &[
    ResourceActionContract {
        kind: "custom_api",
        ops: &["create", "update", "install", "connect"],
        action_name: "capability_acquire",
        requirement: ResourcePayloadRequirement::CustomApiAcquisitionSpec,
        transform: ResourcePayloadTransform::StripResourceFields,
        selected_action_for_resolution: Some("capability_acquire"),
    },
    ResourceActionContract {
        kind: "custom_api",
        ops: &["delete", "enable", "disable"],
        action_name: "custom_api_manage",
        requirement: ResourcePayloadRequirement::ResourceId,
        transform: ResourcePayloadTransform::StripResourceFields,
        selected_action_for_resolution: Some("custom_api_manage"),
    },
    ResourceActionContract {
        kind: "custom_messaging_channel",
        ops: &["create", "update"],
        action_name: "custom_messaging_channel_upsert",
        requirement: ResourcePayloadRequirement::CustomMessagingChannelSpec,
        transform: ResourcePayloadTransform::StripResourceFields,
        selected_action_for_resolution: Some("custom_messaging_channel_upsert"),
    },
    ResourceActionContract {
        kind: "custom_messaging_channel",
        ops: &["delete", "enable", "disable", "test"],
        action_name: "custom_messaging_channel_manage",
        requirement: ResourcePayloadRequirement::ResourceId,
        transform: ResourcePayloadTransform::StripResourceFields,
        selected_action_for_resolution: Some("custom_messaging_channel_manage"),
    },
    ResourceActionContract {
        kind: "extension_pack",
        ops: &["create", "update", "install"],
        action_name: "extension_pack_install",
        requirement: ResourcePayloadRequirement::ExtensionPackInstallTarget,
        transform: ResourcePayloadTransform::ExtensionPack,
        selected_action_for_resolution: Some("extension_pack_install"),
    },
    ResourceActionContract {
        kind: "extension_pack",
        ops: &["connect"],
        action_name: "extension_pack_connect",
        requirement: ResourcePayloadRequirement::ExtensionPackId,
        transform: ResourcePayloadTransform::ExtensionPack,
        selected_action_for_resolution: Some("extension_pack_connect"),
    },
    ResourceActionContract {
        kind: "extension_pack",
        ops: &["enable", "disable"],
        action_name: "extension_pack_set_enabled",
        requirement: ResourcePayloadRequirement::ExtensionPackId,
        transform: ResourcePayloadTransform::ExtensionPack,
        selected_action_for_resolution: Some("extension_pack_set_enabled"),
    },
    ResourceActionContract {
        kind: "extension_pack",
        ops: &["delete"],
        action_name: "extension_pack_delete",
        requirement: ResourcePayloadRequirement::ExtensionPackId,
        transform: ResourcePayloadTransform::ExtensionPack,
        selected_action_for_resolution: Some("extension_pack_delete"),
    },
    ResourceActionContract {
        kind: "extension_pack",
        ops: &["test"],
        action_name: "extension_pack_test_connection",
        requirement: ResourcePayloadRequirement::ExtensionPackId,
        transform: ResourcePayloadTransform::ExtensionPack,
        selected_action_for_resolution: Some("extension_pack_test_connection"),
    },
    ResourceActionContract {
        kind: "mcp_server",
        ops: &["create", "update", "install", "connect"],
        action_name: "mcp_server_manage",
        requirement: ResourcePayloadRequirement::McpServerConfig,
        transform: ResourcePayloadTransform::McpManage,
        selected_action_for_resolution: Some("mcp_server_manage"),
    },
    ResourceActionContract {
        kind: "mcp_server",
        ops: &["delete", "refresh"],
        action_name: "mcp_server_manage",
        requirement: ResourcePayloadRequirement::ResourceId,
        transform: ResourcePayloadTransform::McpManage,
        selected_action_for_resolution: Some("mcp_server_manage"),
    },
    ResourceActionContract {
        kind: "mcp_server",
        ops: &["list", "read", "status"],
        action_name: "mcp_server_manage",
        requirement: ResourcePayloadRequirement::None,
        transform: ResourcePayloadTransform::McpManage,
        selected_action_for_resolution: None,
    },
];
const LLM_NATIVE_IMAGE_ATTACHMENT_LIMIT: usize = 4;
const LLM_NATIVE_IMAGE_ATTACHMENT_MAX_BYTES: u64 = 8 * 1024 * 1024;
const SPINE_MODEL_STREAM_RETRY_ATTEMPTS_PER_CANDIDATE: usize = 2;
const TERMINAL_AUDIT_MAX_OUTPUT_TOKENS: u32 = 512;
const TERMINAL_AUDIT_SYSTEM_PROMPT: &str = "You are a bounded terminal audit judge. Return only the requested JSON object. Judge from structured evidence and semantic task completion, not wording.";

#[derive(Debug, Clone)]
pub struct SpineChatResponse {
    pub text: String,
    pub partial_text: Option<String>,
    pub tool_calls: Vec<SpineToolCall>,
    pub completion_tokens: usize,
    pub cache_read_tokens: usize,
    pub cache_creation_tokens: usize,
    /// Provider response latency in milliseconds — the time the model provider
    /// took to answer this request (request → final chunk), excluding spine
    /// prompt-assembly and tool execution. This is the "LLM latency" the user sees.
    pub provider_latency_ms: u64,
}

#[async_trait]
pub trait SpineLlmServer: Send + Sync {
    async fn chat_completion(
        &self,
        messages: Vec<SpineMessage>,
        tool_schemas: Vec<ActionDef>,
        streaming: bool,
        visual_attachments: Vec<ChatAttachmentHint>,
    ) -> Result<SpineChatResponse, SpineError>;

    async fn terminal_audit_completion(
        &self,
        prompt: String,
    ) -> Result<SpineChatResponse, SpineError> {
        self.chat_completion(
            vec![SpineMessage::User { content: prompt }],
            Vec::new(),
            false,
            Vec::new(),
        )
        .await
    }
}

fn spine_response_is_empty_terminal(text: &str, tool_call_count: usize) -> bool {
    text.trim().is_empty() && tool_call_count == 0
}

fn empty_terminal_spine_response_error() -> SpineError {
    SpineError::new(
        "empty_model_response",
        "The model returned an empty final response without tool calls.",
    )
}

#[derive(Clone)]
pub struct ToolRegistry {
    schemas: Arc<Vec<ActionDef>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            schemas: Arc::new(build_primitive_schemas()),
        }
    }

    pub fn schemas(&self) -> Vec<ActionDef> {
        self.schemas.as_ref().clone()
    }

    pub fn schemas_for_caller(&self, caller_kind: CallerKind) -> Vec<ActionDef> {
        let mut schemas = self.schemas();
        if memory_mutations_deferred_for_caller(caller_kind) {
            for schema in &mut schemas {
                if schema.name == "memory_rw" {
                    restrict_memory_rw_schema_to_read_only(schema);
                }
            }
        }
        schemas
    }

    async fn approval_envelope_for_call(
        &self,
        call: &SpineToolCall,
        cx: &SpineContext,
    ) -> Option<serde_json::Value> {
        if !cx.request.caller_kind.can_pause_for_approval() {
            return None;
        }
        let plan = self.plan_call(call, cx).await;
        let invocations = match plan {
            PrimitivePlan::Actions(invocations) => invocations,
            _ => return None,
        };
        let mut approval_calls = Vec::new();
        for invocation in invocations {
            let invocation = self
                .repair_invocation_for_request_context(&invocation, call, cx)
                .await;
            let action_def = cx
                .agent
                .runtime
                .action_definition(&invocation.action_name)
                .await;
            let decision = cx
                .agent
                .runtime
                .authorize_action_invocation(
                    &invocation.action_name,
                    action_def.as_ref(),
                    &invocation.arguments,
                    &cx.request.authorization,
                )
                .await
                .ok()?;
            if decision.requires_explicit_approval {
                approval_calls.push(DirectChatChainApprovalCall {
                    action_name: invocation.action_name,
                    arguments: invocation.arguments,
                });
            }
        }
        if approval_calls.is_empty() {
            return None;
        }
        match cx
            .agent
            .build_approval_required_envelope(
                cx.request.conversation_id.as_deref(),
                &cx.request.channel,
                &approval_calls,
                &cx.request.authorization,
                "spine_approval_policy",
                "The requested primitive maps to a protected action.",
            )
            .await
        {
            Ok(envelope) => Some(envelope),
            Err(error) => Some(tool_result_error(
                call.name.as_str(),
                "approval_persistence_failed",
                error.to_string(),
            )),
        }
    }

    async fn dispatch(&self, call: SpineToolCall, cx: SpineContext) -> ToolResult {
        let plan = self.plan_call(&call, &cx).await;
        match plan {
            PrimitivePlan::Actions(invocations) => {
                let mut outputs = Vec::new();
                for invocation in invocations {
                    match self.execute_invocation(&invocation, &call, &cx).await {
                        Ok(value) if action_invocation_value_reports_success(&value) => {
                            outputs.push(value)
                        }
                        Ok(value) => return ToolResult::from_value(false, value),
                        Err(value) => return ToolResult::from_value(false, value),
                    }
                }
                let value = if outputs.len() == 1 {
                    outputs
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    serde_json::json!({ "ok": true, "results": outputs })
                };
                ToolResult::from_value(true, value)
            }
            PrimitivePlan::Memory(op) => self.dispatch_memory(op, &call, &cx).await,
            PrimitivePlan::Conversation(op) => self.dispatch_conversation(op, &call, &cx).await,
            PrimitivePlan::Unsupported { reason, extra } => ToolResult::from_value(
                false,
                tool_result_error_with_extra(
                    call.name.as_str(),
                    "unsupported_primitive_request",
                    reason,
                    extra.unwrap_or_else(|| serde_json::json!({})),
                ),
            ),
        }
    }

    async fn execute_invocation(
        &self,
        invocation: &PrimitiveActionInvocation,
        call: &SpineToolCall,
        cx: &SpineContext,
    ) -> Result<serde_json::Value, serde_json::Value> {
        let invocation = self
            .repair_invocation_for_request_context(invocation, call, cx)
            .await;
        let action_def = cx
            .agent
            .runtime
            .action_definition(&invocation.action_name)
            .await;
        match cx
            .agent
            .runtime
            .authorize_action_invocation(
                &invocation.action_name,
                action_def.as_ref(),
                &invocation.arguments,
                &cx.request.authorization,
            )
            .await
        {
            Ok(decision) if decision.requires_explicit_approval => {
                return Err(tool_result_error_with_extra(
                    call.name.as_str(),
                    "approval_required",
                    decision.reason,
                    serde_json::json!({
                        "action_count": 1,
                        "remediation": {"type": "approve"}
                    }),
                ));
            }
            Ok(decision) if !decision.allowed => {
                return Err(tool_result_error(
                    call.name.as_str(),
                    "permission_denied",
                    decision.reason,
                ));
            }
            Err(error) => {
                return Err(tool_result_error(
                    call.name.as_str(),
                    "authorization_failed",
                    error.to_string(),
                ));
            }
            _ => {}
        }

        let content = cx
            .agent
            .execute_action_with_hooks(
                &invocation.action_name,
                &invocation.arguments,
                &cx.request.channel,
                None,
                Some(&cx.request.authorization),
                cx.request.conversation_id.as_deref(),
                cx.request.project_id.as_deref(),
                cx.stream_tx.as_ref(),
            )
            .await
            .map_err(|error| {
                if let Some(action_error) =
                    crate::actions::parse_structured_action_error_text(&error.to_string())
                {
                    action_error.to_envelope(&call.name)
                } else {
                    tool_result_error(call.name.as_str(), "execution_failed", error.to_string())
                }
            })?;
        let arkdistill_profile =
            crate::core::agent::ark_distill::load_arkdistill_profile(&cx.agent.storage).await;
        let output = spine_tool_result_output_for_model_with_profile(
            call.name.as_str(),
            &invocation.action_name,
            content,
            &arkdistill_profile,
        );
        log_arkdistill_tool_output(
            cx,
            call.name.as_str(),
            &invocation.action_name,
            &output.stats,
        )
        .await;
        let mut result = output.value;
        remember_pending_credential_prompt_from_tool_result(cx, &mut result).await;
        Ok(result)
    }

    async fn repair_invocation_for_request_context(
        &self,
        invocation: &PrimitiveActionInvocation,
        call: &SpineToolCall,
        cx: &SpineContext,
    ) -> PrimitiveActionInvocation {
        if call.name == "browse" && invocation.action_name.eq_ignore_ascii_case("browser_auto") {
            return browser_auto_invocation_with_request_profile(
                invocation,
                cx.request.browser_profile_context.as_ref(),
            );
        }
        invocation.clone()
    }

    async fn dispatch_memory(
        &self,
        op: MemoryPrimitiveOp,
        call: &SpineToolCall,
        cx: &SpineContext,
    ) -> ToolResult {
        match op {
            MemoryPrimitiveOp::Search { query, limit } => {
                let args = serde_json::json!({
                    "query": query,
                    "limit": limit.unwrap_or(5),
                    "include_semantic": true,
                    "include_structured": true,
                    "include_procedures": true,
                    "include_lessons": true
                });
                let invocation = PrimitiveActionInvocation {
                    action_name: "memory_lookup".to_string(),
                    arguments: args,
                };
                match self.execute_invocation(&invocation, call, cx).await {
                    Ok(value) => ToolResult::from_value(true, value),
                    Err(value) => ToolResult::from_value(false, value),
                }
            }
            MemoryPrimitiveOp::DeferredMutation { op, intent_summary } => ToolResult::from_value(
                true,
                serde_json::json!({
                    "ok": true,
                    "status": "deferred_to_background_memory_capture",
                    "operation": op,
                    "memory_mutation_applied": false,
                    "background_processor": "arkmemory",
                    "capture_source": "persisted_chat_turn",
                    "intent_summary": intent_summary,
                    "assistant_instruction": "Do not call memory_rw again for this chat memory mutation. Continue the user-facing response; ArkMemory will evaluate the persisted source message asynchronously."
                }),
            ),
            MemoryPrimitiveOp::Write {
                key,
                value,
                kind,
                scope,
                slot_cardinality,
                confidence,
                reason,
                intent_summary,
            } => match cx
                .agent
                .upsert_learned_user_memory(
                    &key,
                    &value,
                    kind.as_deref(),
                    None,
                    scope.as_deref(),
                    confidence.unwrap_or(0.85),
                    &cx.request.channel,
                    cx.request.conversation_id.as_deref(),
                    cx.request.project_id.as_deref(),
                    "memory_rw",
                    None,
                    None,
                    None,
                    reason.as_deref().or(intent_summary.as_deref()),
                    None,
                    None,
                    &[],
                    slot_cardinality.as_deref(),
                    None,
                )
                .await
            {
                Ok(id) => {
                    ToolResult::from_value(true, serde_json::json!({ "ok": true, "memory_id": id }))
                }
                Err(error) => ToolResult::from_value(
                    false,
                    tool_result_error(call.name.as_str(), "memory_write_failed", error.to_string()),
                ),
            },
            MemoryPrimitiveOp::Delete {
                key,
                kind,
                scope,
                reason,
                intent_summary,
            } => {
                let ids = cx
                    .agent
                    .retract_learned_user_memory(
                        &key,
                        kind.as_deref(),
                        scope.as_deref(),
                        &cx.request.channel,
                        cx.request.conversation_id.as_deref(),
                        cx.request.project_id.as_deref(),
                        reason.as_deref().or(intent_summary.as_deref()),
                    )
                    .await;
                ToolResult::from_value(
                    true,
                    serde_json::json!({
                        "ok": true,
                        "retracted_memory_ids": ids,
                    }),
                )
            }
        }
    }

    async fn dispatch_conversation(
        &self,
        op: ConversationPrimitiveOp,
        _call: &SpineToolCall,
        cx: &SpineContext,
    ) -> ToolResult {
        match op {
            ConversationPrimitiveOp::Read { limit } => {
                let Some(conversation_id) = cx.request.conversation_id.as_deref() else {
                    return ToolResult::from_value(
                        false,
                        tool_result_error(
                            "resource_rw",
                            "missing_conversation_id",
                            "No active conversation id is available.",
                        ),
                    );
                };
                match cx
                    .agent
                    .encrypted_storage
                    .get_recent_messages_decrypted(conversation_id, limit.unwrap_or(20) as u64)
                    .await
                {
                    Ok(messages) => ToolResult::from_value(
                        true,
                        serde_json::json!({
                            "ok": true,
                            "messages": messages.into_iter().map(|message| {
                                serde_json::json!({
                                    "id": message.id,
                                    "role": message.role,
                                    "content": message.content,
                                    "timestamp": message.timestamp,
                                    "tool_calls": message.tool_calls_json
                                        .as_deref()
                                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
                                    "tool_call_id": message.tool_call_id,
                                })
                            }).collect::<Vec<_>>()
                        }),
                    ),
                    Err(error) => ToolResult::from_value(
                        false,
                        tool_result_error(
                            "resource_rw",
                            "conversation_read_failed",
                            error.to_string(),
                        ),
                    ),
                }
            }
        }
    }

    async fn plan_call(&self, call: &SpineToolCall, cx: &SpineContext) -> PrimitivePlan {
        match call.name.as_str() {
            "search" => plan_search(&call.arguments),
            "fetch" => plan_fetch(&call.arguments),
            "browse" => plan_browse(&call.arguments),
            "code_exec" => plan_code_exec(&call.arguments),
            "pdf_generate" => plan_pdf_generate(&call.arguments),
            "app_deploy" => plan_direct_action("app_deploy", &call.arguments),
            "file_read" => plan_direct_action("file_read", &call.arguments),
            "file_search" => plan_direct_action("file_search", &call.arguments),
            "file_write" => plan_direct_action("file_write", &call.arguments),
            "file_patch" => plan_direct_action("file_patch", &call.arguments),
            "file_delete" => plan_direct_action("file_delete", &call.arguments),
            "skill_manage" => plan_skill_manage(&call.arguments),
            "resource_rw" => plan_resource_rw(&call.arguments),
            "memory_rw" => plan_memory_rw_for_caller(&call.arguments, cx.request.caller_kind),
            "action_call" => {
                let action_def = match json_text(&call.arguments, "action_name") {
                    Some(action_name) => cx.agent.runtime.action_definition(&action_name).await,
                    None => None,
                };
                plan_action_call(&call.arguments, action_def.as_ref())
            }
            "delegate" => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "delegate".to_string(),
                arguments: merge_content_metadata(&call.arguments),
            }]),
            other => unsupported(format!(
                "Unknown primitive `{}`. The primitive registry exposes only {:?}.",
                other, PRIMITIVE_NAMES
            )),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn memory_mutations_deferred_for_caller(caller_kind: CallerKind) -> bool {
    matches!(
        caller_kind,
        CallerKind::Chat | CallerKind::Gateway | CallerKind::Companion
    )
}

fn restrict_memory_rw_schema_to_read_only(schema: &mut ActionDef) {
    if let Some(op_schema) = schema
        .input_schema
        .get_mut("properties")
        .and_then(|value| value.get_mut("op"))
        .and_then(|value| value.as_object_mut())
    {
        op_schema.insert("enum".to_string(), serde_json::json!(["search", "read"]));
    }
    schema.description = "Foreground saved-memory read/search for answering the current request. Chat memory writes, updates, and deletions are handled by deferred ArkMemory capture after the chat turn is persisted.".to_string();
}

async fn remember_pending_credential_prompt_from_tool_result(
    cx: &SpineContext,
    result: &mut serde_json::Value,
) {
    let Some(conversation_id) = cx
        .request
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let request = find_credential_request_value(result)
        .or_else(|| infer_credential_request_from_result(result));
    let Some(request) = request else {
        return;
    };
    let Some(kind) = credential_value_text(&request, "kind") else {
        return;
    };
    let mut remembered = false;
    match kind.as_str() {
        "mcp_server_auth" => {
            if let Some(server_id) = credential_value_text(&request, "server_id") {
                let server_name = credential_value_text(&request, "server_name")
                    .or_else(|| credential_value_text(&request, "display_name"))
                    .unwrap_or_else(|| server_id.clone());
                let auth_type = credential_value_text(&request, "auth_type")
                    .unwrap_or_else(|| "bearer".to_string());
                cx.agent
                    .remember_mcp_server_auth_chat_prompt(
                        conversation_id,
                        &server_id,
                        &server_name,
                        &auth_type,
                        credential_value_text(&request, "auth_name").as_deref(),
                        credential_value_text(&request, "settings_path").as_deref(),
                    )
                    .await;
                remembered = true;
            }
        }
        "custom_api_auth" => {
            if let Some(api_id) = credential_value_text(&request, "api_id") {
                let api_name = credential_value_text(&request, "api_name")
                    .or_else(|| credential_value_text(&request, "display_name"))
                    .unwrap_or_else(|| api_id.clone());
                let auth_mode = credential_value_text(&request, "auth_mode")
                    .unwrap_or_else(|| "bearer".to_string());
                cx.agent
                    .remember_custom_api_auth_chat_prompt(
                        conversation_id,
                        &api_id,
                        &api_name,
                        &auth_mode,
                        credential_value_text(&request, "auth_name").as_deref(),
                        credential_value_text(&request, "settings_path").as_deref(),
                    )
                    .await;
                remembered = true;
            }
        }
        "integration_auth" => {
            if let Some(integration_id) = credential_value_text(&request, "integration_id") {
                cx.agent
                    .remember_integration_auth_chat_prompt(
                        conversation_id,
                        &integration_id,
                        None,
                        None,
                    )
                    .await;
                remembered = true;
            }
        }
        "extension_pack_connection" => {
            if let (Some(pack_id), Some(connection_id)) = (
                credential_value_text(&request, "pack_id"),
                credential_value_text(&request, "connection_id"),
            ) {
                let pack_name = credential_value_text(&request, "pack_name")
                    .or_else(|| credential_value_text(&request, "display_name"))
                    .unwrap_or_else(|| pack_id.clone());
                let required_keys = credential_value_string_array(&request, "required_keys")
                    .or_else(|| credential_value_string_array(&request, "required_secrets"))
                    .unwrap_or_default();
                cx.agent
                    .remember_extension_pack_chat_credential_prompt(
                        conversation_id,
                        &pack_id,
                        &pack_name,
                        &connection_id,
                        &required_keys,
                    )
                    .await;
                remembered = !required_keys.is_empty();
            }
        }
        "app_runtime_secret" => {
            if let Some(app_id) = credential_value_text(&request, "app_id") {
                let title = credential_value_text(&request, "display_name")
                    .or_else(|| credential_value_text(&request, "title"))
                    .unwrap_or_else(|| app_id.clone());
                let required_keys = credential_value_string_array(&request, "required_keys")
                    .or_else(|| credential_value_string_array(&request, "required_env"))
                    .or_else(|| credential_value_string_array(&request, "missing_env"))
                    .unwrap_or_default();
                if !required_keys.is_empty() {
                    cx.agent
                        .remember_pending_secret_followup(
                            conversation_id,
                            PendingSecretFollowupKind::RestartApp {
                                app_id,
                                title,
                                missing_env: required_keys,
                            },
                        )
                        .await;
                    remembered = true;
                }
            }
        }
        _ => {}
    }
    if remembered {
        mark_secure_credential_prompt_pending(result);
    }
}

fn mark_secure_credential_prompt_pending(value: &mut serde_json::Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "secure_credential_prompt_pending".to_string(),
            serde_json::Value::Bool(true),
        );
        object.insert(
            "credential_delivery".to_string(),
            serde_json::Value::String("secure_chat_prompt".to_string()),
        );
    }
}

fn find_credential_request_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(request) = object
                .get("credential_request")
                .filter(|request| request.is_object())
            {
                return Some(request.clone());
            }
            object.values().find_map(find_credential_request_value)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_credential_request_value),
        _ => None,
    }
}

fn infer_credential_request_from_result(value: &serde_json::Value) -> Option<serde_json::Value> {
    if !result_has_status(value, "needs_credentials") {
        return None;
    }
    if let Some(integration_id) = find_text_field(value, "integration_id") {
        return Some(serde_json::json!({
            "kind": "integration_auth",
            "integration_id": integration_id,
            "settings_path": find_text_field(value, "settings_path"),
            "secure_input_required": true
        }));
    }
    if let (Some(pack_id), Some(connection_id)) = (
        find_text_field(value, "pack_id"),
        find_text_field(value, "connection_id"),
    ) {
        let required_keys = find_string_array_field(value, "required_keys")
            .or_else(|| find_string_array_field(value, "required_secrets"))
            .unwrap_or_default();
        return Some(serde_json::json!({
            "kind": "extension_pack_connection",
            "pack_id": pack_id,
            "pack_name": find_text_field(value, "pack_name"),
            "connection_id": connection_id,
            "required_keys": required_keys,
            "settings_path": find_text_field(value, "settings_path"),
            "secure_input_required": true
        }));
    }
    None
}

fn credential_request_requires_secure_input(request: &serde_json::Value) -> bool {
    request
        .get("secure_input_required")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn result_has_status(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object
                .get("status")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|status| status.eq_ignore_ascii_case(expected))
                || object
                    .values()
                    .any(|value| result_has_status(value, expected))
        }
        serde_json::Value::Array(items) => {
            items.iter().any(|value| result_has_status(value, expected))
        }
        _ => false,
    }
}

fn find_text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => object
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_text_field(value, key))
            }),
        serde_json::Value::Array(items) => {
            items.iter().find_map(|value| find_text_field(value, key))
        }
        _ => None,
    }
}

fn find_string_array_field(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    match value {
        serde_json::Value::Object(object) => object
            .get(key)
            .and_then(credential_string_array)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_string_array_field(value, key))
            }),
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|value| find_string_array_field(value, key)),
        _ => None,
    }
}

fn credential_value_text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn credential_value_string_array(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    value.get(key).and_then(credential_string_array)
}

fn credential_string_array(value: &serde_json::Value) -> Option<Vec<String>> {
    let items = value
        .as_array()?
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    Some(items)
}

#[derive(Clone)]
pub struct SpineContext {
    pub agent: Agent,
    pub request: Arc<SpineRequest>,
    pub trace: Arc<SpineTraceRecorder>,
    pub stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    paused_approval: Arc<tokio::sync::Mutex<Option<serde_json::Value>>>,
}

impl SpineContext {
    pub fn new(
        agent: Agent,
        request: Arc<SpineRequest>,
        trace: Arc<SpineTraceRecorder>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Self {
        Self {
            agent,
            request,
            trace,
            stream_tx,
            paused_approval: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    async fn emit(&self, event: SpineTraceEvent) {
        self.trace.emit(event).await;
    }

    pub async fn paused_approval(&self) -> Option<serde_json::Value> {
        self.paused_approval.lock().await.clone()
    }

    async fn set_paused_approval(&self, envelope: serde_json::Value) {
        *self.paused_approval.lock().await = Some(envelope);
    }
}

#[derive(Debug, Clone)]
struct PrimitiveActionInvocation {
    action_name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
enum PrimitivePlan {
    Actions(Vec<PrimitiveActionInvocation>),
    Memory(MemoryPrimitiveOp),
    Conversation(ConversationPrimitiveOp),
    Unsupported {
        reason: String,
        extra: Option<serde_json::Value>,
    },
}

#[derive(Debug, Clone)]
enum MemoryPrimitiveOp {
    Search {
        query: String,
        limit: Option<usize>,
    },
    DeferredMutation {
        op: String,
        intent_summary: Option<String>,
    },
    Write {
        key: String,
        value: String,
        kind: Option<String>,
        scope: Option<String>,
        slot_cardinality: Option<String>,
        confidence: Option<f32>,
        reason: Option<String>,
        intent_summary: Option<String>,
    },
    Delete {
        key: String,
        kind: Option<String>,
        scope: Option<String>,
        reason: Option<String>,
        intent_summary: Option<String>,
    },
}

#[derive(Debug, Clone)]
enum ConversationPrimitiveOp {
    Read { limit: Option<usize> },
}

#[derive(Debug, Clone)]
struct ToolResult {
    ok: bool,
    value: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpineTerminalTextKind {
    Completed,
    Blocked,
    NeedsInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalTextProvenance {
    ModelAuthored,
    SystemAuthored,
    StructuredUserQuestion,
    ToolOrigin,
}

impl ToolResult {
    fn from_value(ok: bool, value: serde_json::Value) -> Self {
        Self { ok, value }
    }

    fn base_value(&self) -> serde_json::Value {
        if self.value.get("ok").is_some() || self.value.get("status").is_some() {
            let mut value = self.value.clone();
            if let Some(object) = value.as_object_mut() {
                object
                    .entry("ok".to_string())
                    .or_insert_with(|| serde_json::Value::Bool(self.ok));
            }
            value
        } else {
            serde_json::json!({
                "ok": self.ok,
                "result": self.value,
            })
        }
    }

    fn to_json(&self) -> String {
        self.base_value().to_string()
    }

    fn to_json_for_tool(&self, call: &SpineToolCall) -> String {
        normalize_tool_evidence_envelope(call, self).to_string()
    }

    fn summary(&self) -> String {
        let raw = self.to_json();
        safe_truncate(&raw, 240)
    }
}

fn action_invocation_value_reports_success(value: &serde_json::Value) -> bool {
    !matches!(
        super::tool_responses::structured_tool_value_outcome(value).map(|report| report.state),
        Some(
            super::tool_responses::StructuredToolOutcomeState::Failure
                | super::tool_responses::StructuredToolOutcomeState::NeedsInput
        )
    )
}

fn normalize_tool_evidence_envelope(
    call: &SpineToolCall,
    result: &ToolResult,
) -> serde_json::Value {
    let base = result.base_value();
    let mut object = base.as_object().cloned().unwrap_or_else(|| {
        let mut object = serde_json::Map::new();
        object.insert("ok".to_string(), serde_json::Value::Bool(result.ok));
        object.insert("result".to_string(), base.clone());
        object
    });

    let outcome = super::tool_responses::structured_tool_value_outcome(&serde_json::Value::Object(
        object.clone(),
    ));
    let status = outcome
        .as_ref()
        .map(|report| match report.state {
            super::tool_responses::StructuredToolOutcomeState::Success => "ok",
            super::tool_responses::StructuredToolOutcomeState::Failure => "error",
            super::tool_responses::StructuredToolOutcomeState::NeedsInput => "needs_input",
        })
        .unwrap_or(if result.ok { "ok" } else { "error" });

    object.insert(
        "operation".to_string(),
        serde_json::json!({
            "primitive": call.name.as_str(),
            "reported_tool": first_structured_string(&serde_json::Value::Object(object.clone()), &["tool", "primitive"]),
            "reported_operation": first_structured_string(&serde_json::Value::Object(object.clone()), &["operation", "action", "op", "kind"]),
        }),
    );
    object.insert(
        "status".to_string(),
        serde_json::Value::String(status.to_string()),
    );
    object.insert(
        "user_visible_outcome".to_string(),
        super::tool_responses::summarize_structured_tool_output_for_user(
            &serde_json::Value::Object(object.clone()).to_string(),
        )
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "next_repair_hint".to_string(),
        next_repair_hint_from_tool_value(&serde_json::Value::Object(object.clone())),
    );
    let (error_class, retryable) =
        classify_tool_evidence_failure(&serde_json::Value::Object(object.clone()), result.ok);
    object.insert("error_class".to_string(), error_class);
    object.insert("retryable".to_string(), serde_json::Value::Bool(retryable));
    object.insert(
        "artifacts".to_string(),
        tool_evidence_artifacts(call, &serde_json::Value::Object(object.clone())),
    );
    object.insert(
        "capability_tags".to_string(),
        tool_evidence_capability_tags(call, &serde_json::Value::Object(object.clone())),
    );
    object.insert(
        "diagnostics".to_string(),
        tool_evidence_diagnostics(&serde_json::Value::Object(object.clone())),
    );
    object.insert(
        "logs".to_string(),
        tool_evidence_logs(&serde_json::Value::Object(object.clone())),
    );

    serde_json::Value::Object(object)
}

fn first_structured_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(text) = object
            .get(*key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_string());
        }
    }
    if let Some(nested) = object.get("result").and_then(|value| value.as_object()) {
        for key in keys {
            if let Some(text) = nested
                .get(*key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn embedded_json_object_from_string(value: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(value.trim())
        .ok()
        .filter(|value| value.is_object())
}

fn next_repair_hint_from_tool_value(value: &serde_json::Value) -> serde_json::Value {
    let nested_data = value.get("data");
    let remediation = value.get("remediation").cloned().or_else(|| {
        // Durable validation failures carry the full argument contract; promote
        // it so the repair turn sees the complete expected shape, not just the
        // field that tripped.
        value
            .get("expected_contract")
            .or_else(|| nested_data.and_then(|data| data.get("expected_contract")))
            .cloned()
    });
    let hint = first_structured_string(value, &["hint", "next_repair_hint"])
        .or_else(|| {
            value
                .get("message")
                .and_then(|value| value.as_str())
                .and_then(embedded_json_object_from_string)
                .and_then(|embedded| {
                    first_structured_string(&embedded, &["hint", "next_repair_hint"])
                })
        })
        .or_else(|| {
            first_structured_string(value, &["assistant_instruction"]).or_else(|| {
                nested_data
                    .and_then(|data| first_structured_string(data, &["assistant_instruction"]))
            })
        });
    if remediation.is_none() && hint.is_none() {
        return serde_json::Value::Null;
    }
    serde_json::json!({
        "hint": hint,
        "remediation": remediation.unwrap_or(serde_json::Value::Null),
    })
}

fn classify_tool_evidence_failure(
    value: &serde_json::Value,
    ok: bool,
) -> (serde_json::Value, bool) {
    if ok {
        return (serde_json::Value::Null, false);
    }
    let domain = first_structured_string(value, &["domain"]);
    let reason = first_structured_string(value, &["reason", "error"]);
    let embedded_retryable = value
        .get("message")
        .and_then(|value| value.as_str())
        .and_then(embedded_json_object_from_string)
        .and_then(|embedded| embedded.get("retryable").and_then(|value| value.as_bool()));
    let retryable = embedded_retryable
        .or_else(|| value.get("retryable").and_then(|value| value.as_bool()))
        .unwrap_or({
            matches!(
                reason.as_deref(),
                Some("timeout" | "rate_limited" | "unavailable")
            )
        });
    let class = match (domain.as_deref(), reason.as_deref()) {
        (Some("auth"), _) => "auth",
        (_, Some("not_connected" | "permission_denied" | "approval_required")) => "access",
        (_, Some("missing_input" | "invalid_input" | "ambiguous")) => "input",
        (_, Some("timeout" | "rate_limited" | "unavailable")) => "transient",
        (Some("app"), _) => "runtime",
        (Some("search"), _) => "evidence",
        _ => "execution",
    };
    (serde_json::Value::String(class.to_string()), retryable)
}

fn push_tool_evidence_capability_tag(tags: &mut Vec<String>, raw: &str) {
    let mut normalized = String::new();
    let mut previous_separator = false;
    for ch in raw.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            previous_separator = false;
        } else if !previous_separator && !normalized.is_empty() {
            normalized.push('_');
            previous_separator = true;
        }
    }
    let normalized = normalized.trim_matches('_');
    if !normalized.is_empty() && !tags.iter().any(|tag| tag == normalized) {
        tags.push(normalized.to_string());
    }
}

fn collect_tool_evidence_capability_tags(value: &serde_json::Value, tags: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for key in ["capability_tags", "capabilities", "tags"] {
        match object.get(key) {
            Some(serde_json::Value::Array(values)) => {
                for value in values {
                    if let Some(tag) = value.as_str() {
                        push_tool_evidence_capability_tag(tags, tag);
                    }
                }
            }
            Some(serde_json::Value::String(value)) => {
                push_tool_evidence_capability_tag(tags, value);
            }
            _ => {}
        }
    }
    if let Some(result) = object.get("result") {
        collect_tool_evidence_capability_tags(result, tags);
    }
}

fn tool_evidence_capability_tags(
    call: &SpineToolCall,
    value: &serde_json::Value,
) -> serde_json::Value {
    let mut tags = Vec::new();
    push_tool_evidence_capability_tag(&mut tags, call.name.as_str());
    if let Some(kind) = call.arguments.get("kind").and_then(|value| value.as_str()) {
        push_tool_evidence_capability_tag(&mut tags, kind);
    }
    collect_tool_evidence_capability_tags(value, &mut tags);
    serde_json::Value::Array(tags.into_iter().map(serde_json::Value::String).collect())
}

fn tool_evidence_artifacts(call: &SpineToolCall, value: &serde_json::Value) -> serde_json::Value {
    let mut artifacts = Vec::new();
    if let Some(path) = first_structured_string(value, &["path", "file_path", "managed_file"]) {
        artifacts.push(serde_json::json!({
            "kind": "file",
            "path": path,
        }));
    }
    if call.name == "app_deploy" {
        let app_id = first_structured_string(value, &["app_id", "id"]);
        let url = first_structured_string(value, &["access_url", "url"]);
        if app_id.is_some() || url.is_some() {
            artifacts.push(serde_json::json!({
                "kind": "app_service",
                "id": app_id,
                "url": url,
            }));
        }
    }
    serde_json::Value::Array(artifacts)
}

fn tool_evidence_diagnostics(value: &serde_json::Value) -> serde_json::Value {
    let mut diagnostics = serde_json::Map::new();
    for key in [
        "message",
        "detail",
        "summary",
        "error",
        "exit_code",
        "phase",
        "command",
        "cwd",
    ] {
        if let Some(item) = value.get(key) {
            diagnostics.insert(key.to_string(), item.clone());
        }
    }
    if let Some(message) = value.get("message").and_then(|value| value.as_str()) {
        if let Some(embedded) = embedded_json_object_from_string(message) {
            if let Some(object) = embedded.as_object() {
                for key in ["phase", "command", "cwd", "exit_code", "log_tail", "hint"] {
                    if let Some(item) = object.get(key) {
                        diagnostics.insert(key.to_string(), item.clone());
                    }
                }
            }
        }
    }
    serde_json::Value::Object(diagnostics)
}

fn tool_evidence_logs(value: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "stdout": value.get("stdout").cloned().unwrap_or(serde_json::Value::Null),
        "stderr": value.get("stderr").cloned().unwrap_or(serde_json::Value::Null),
        "log_tail": value.get("log_tail").cloned().unwrap_or_else(|| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .and_then(embedded_json_object_from_string)
                .and_then(|embedded| embedded.get("log_tail").cloned())
                .unwrap_or(serde_json::Value::Null)
        }),
    })
}

const SPINE_TOOL_STREAM_MAX_STRING_CHARS: usize = 900;
const SPINE_TOOL_STREAM_MAX_ARRAY_ITEMS: usize = 12;
const SPINE_TOOL_STREAM_MAX_OBJECT_KEYS: usize = 24;
const SPINE_TOOL_STREAM_MAX_DEPTH: usize = 5;
const SPINE_TOOL_STREAM_PREVIEW_ITEMS: usize = 5;
const SPINE_TOOL_HISTORY_MAX_STRING_CHARS: usize = 240;
const SPINE_TOOL_HISTORY_MAX_ARRAY_ITEMS: usize = 8;
const SPINE_TOOL_HISTORY_MAX_OBJECT_KEYS: usize = 16;
const SPINE_TOOL_HISTORY_MAX_DEPTH: usize = 4;
const SPINE_TOOL_HISTORY_PREVIEW_ITEMS: usize = 8;

fn spine_tool_start_stream_payload(call: &SpineToolCall) -> serde_json::Value {
    let arguments = sanitize_spine_tool_stream_value(&call.arguments, 0);
    let activity_label = call
        .activity_label
        .as_deref()
        .and_then(clean_model_tool_activity_label);
    let intent_summary = activity_label
        .clone()
        .unwrap_or_else(|| spine_tool_start_intent_summary(&call.name, &arguments));
    let mut payload = serde_json::json!({
        "kind": "model_tool_call",
        "tool_call_id": call.id,
        "tool_name": call.name,
        "arguments": arguments,
        "intent_summary": intent_summary,
    });
    if let Some(activity_label) = activity_label {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "activity_label".to_string(),
                serde_json::json!(activity_label),
            );
            obj.insert(
                "display_label".to_string(),
                serde_json::json!(activity_label),
            );
        }
    }
    payload
}

fn clean_model_tool_activity_label(value: &str) -> Option<String> {
    let label = value
        .replace(['\r', '\n', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        None
    } else {
        Some(safe_truncate(&label, 80))
    }
}

fn spine_tool_result_stream_content(call: &SpineToolCall, result: &ToolResult) -> String {
    let result_preview = sanitize_spine_tool_stream_value(&result.value, 0);
    let summary =
        spine_tool_result_visible_summary(&result_preview).unwrap_or_else(|| result.summary());
    serde_json::json!({
        "kind": "model_tool_result",
        "tool_call_id": call.id,
        "tool_name": call.name,
        "ok": result.ok,
        "summary": summary,
        "result_preview": result_preview,
    })
    .to_string()
}

fn redact_spine_tool_stream_text(value: &str) -> String {
    let secret_redacted = crate::security::redact_secret_input(value).text;
    crate::security::redact_pii(&secret_redacted)
}

fn is_bulk_tool_argument_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "content"
            | "raw_content"
            | "file_content"
            | "body"
            | "html"
            | "markdown"
            | "code"
            | "patch"
            | "files"
            | "file_patches"
            | "file_payloads"
            | "content_base64"
            | "bytes_b64"
    )
}

fn omitted_tool_argument_summary(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::json!({
            "omitted": true,
            "chars": text.chars().count(),
        }),
        serde_json::Value::Array(items) => serde_json::json!({
            "omitted": true,
            "items": items.len(),
        }),
        serde_json::Value::Object(map) => serde_json::json!({
            "omitted": true,
            "fields": map.len(),
        }),
        _ => serde_json::json!({
            "omitted": true,
        }),
    }
}

fn sanitize_spine_tool_history_value(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= SPINE_TOOL_HISTORY_MAX_DEPTH {
        return match value {
            serde_json::Value::Null => serde_json::Value::Null,
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => value.clone(),
            serde_json::Value::String(text) => serde_json::Value::String(safe_truncate(
                redact_spine_tool_stream_text(text.trim()).trim(),
                120,
            )),
            serde_json::Value::Array(items) => serde_json::json!({
                "truncated": true,
                "items": items.len(),
            }),
            serde_json::Value::Object(map) => serde_json::json!({
                "truncated": true,
                "fields": map.len(),
            }),
        };
    }

    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.clone()
        }
        serde_json::Value::String(text) => serde_json::Value::String(safe_truncate(
            redact_spine_tool_stream_text(text.trim()).trim(),
            SPINE_TOOL_HISTORY_MAX_STRING_CHARS,
        )),
        serde_json::Value::Array(items) => {
            let mut sanitized = items
                .iter()
                .take(SPINE_TOOL_HISTORY_MAX_ARRAY_ITEMS)
                .map(|item| sanitize_spine_tool_history_value(item, depth + 1))
                .collect::<Vec<_>>();
            if items.len() > SPINE_TOOL_HISTORY_MAX_ARRAY_ITEMS {
                sanitized.push(serde_json::json!({
                    "truncated_items": items.len() - SPINE_TOOL_HISTORY_MAX_ARRAY_ITEMS,
                }));
            }
            serde_json::Value::Array(sanitized)
        }
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            let mut omitted = 0usize;
            for (key, inner) in map {
                if key.starts_with('_') {
                    continue;
                }
                if sanitized.len() >= SPINE_TOOL_HISTORY_MAX_OBJECT_KEYS {
                    omitted += 1;
                    continue;
                }
                if is_sensitive_tool_call_argument_key(key) {
                    sanitized.insert(
                        key.clone(),
                        serde_json::Value::String("[redacted]".to_string()),
                    );
                    continue;
                }
                if is_bulk_tool_argument_key(key) {
                    let keep_short_string = inner
                        .as_str()
                        .is_some_and(|text| text.chars().count() <= 80);
                    if !keep_short_string {
                        sanitized.insert(key.clone(), omitted_tool_argument_summary(inner));
                        continue;
                    }
                }
                sanitized.insert(
                    key.clone(),
                    sanitize_spine_tool_history_value(inner, depth + 1),
                );
            }
            if omitted > 0 {
                sanitized.insert("truncated_keys".to_string(), serde_json::json!(omitted));
            }
            serde_json::Value::Object(sanitized)
        }
    }
}

fn spine_tool_call_history_context(
    tool_calls: &[SpineToolCall],
    tool_call_aliases: &HashMap<String, String>,
) -> Option<String> {
    if tool_calls.is_empty() {
        return None;
    }
    let lines = tool_calls
        .iter()
        .map(|call| {
            let call_label = tool_call_aliases
                .get(&call.id)
                .map(String::as_str)
                .unwrap_or("tool_call");
            let arguments = sanitize_spine_tool_history_value(&call.arguments, 0);
            let preview = spine_tool_stream_preview(&arguments, SPINE_TOOL_HISTORY_PREVIEW_ITEMS);
            if preview.trim().is_empty() {
                format!("- `{}` called `{}`.", call_label, call.name)
            } else {
                format!(
                    "- `{}` called `{}` with {}.",
                    call_label, call.name, preview
                )
            }
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(
            crate::core::model::llm_context_sanitizer::wrap_internal_tool_context(&format!(
                "tool_call_context:\n{}",
                lines.join("\n")
            )),
        )
    }
}

fn sanitize_spine_tool_stream_value(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= SPINE_TOOL_STREAM_MAX_DEPTH {
        return match value {
            serde_json::Value::Null => serde_json::Value::Null,
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => value.clone(),
            serde_json::Value::String(text) => {
                let redacted = redact_spine_tool_stream_text(text.trim());
                serde_json::Value::String(safe_truncate(redacted.trim(), 160))
            }
            serde_json::Value::Array(items) => serde_json::json!({
                "truncated": true,
                "items": items.len(),
            }),
            serde_json::Value::Object(map) => serde_json::json!({
                "truncated": true,
                "fields": map.len(),
            }),
        };
    }

    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.clone()
        }
        serde_json::Value::String(text) => serde_json::Value::String(safe_truncate(
            redact_spine_tool_stream_text(text.trim()).trim(),
            SPINE_TOOL_STREAM_MAX_STRING_CHARS,
        )),
        serde_json::Value::Array(items) => {
            let mut sanitized = items
                .iter()
                .take(SPINE_TOOL_STREAM_MAX_ARRAY_ITEMS)
                .map(|item| sanitize_spine_tool_stream_value(item, depth + 1))
                .collect::<Vec<_>>();
            if items.len() > SPINE_TOOL_STREAM_MAX_ARRAY_ITEMS {
                sanitized.push(serde_json::json!({
                    "truncated_items": items.len() - SPINE_TOOL_STREAM_MAX_ARRAY_ITEMS,
                }));
            }
            serde_json::Value::Array(sanitized)
        }
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            let mut omitted = 0usize;
            for (key, inner) in map {
                if key.starts_with('_') {
                    continue;
                }
                if sanitized.len() >= SPINE_TOOL_STREAM_MAX_OBJECT_KEYS {
                    omitted += 1;
                    continue;
                }
                if is_sensitive_tool_call_argument_key(key) {
                    sanitized.insert(
                        key.clone(),
                        serde_json::Value::String("[redacted]".to_string()),
                    );
                    continue;
                }
                sanitized.insert(
                    key.clone(),
                    sanitize_spine_tool_stream_value(inner, depth + 1),
                );
            }
            if omitted > 0 {
                sanitized.insert("truncated_keys".to_string(), serde_json::json!(omitted));
            }
            serde_json::Value::Object(sanitized)
        }
    }
}

fn spine_tool_start_intent_summary(tool_name: &str, arguments: &serde_json::Value) -> String {
    let label = readable_spine_tool_name(tool_name);
    let preview = spine_tool_stream_preview(arguments, SPINE_TOOL_STREAM_PREVIEW_ITEMS);
    if preview.is_empty() {
        format!("Starting {label}.")
    } else {
        format!("Starting {label} with {preview}.")
    }
}

fn spine_tool_result_visible_summary(result_preview: &serde_json::Value) -> Option<String> {
    let preview = spine_tool_stream_preview(result_preview, SPINE_TOOL_STREAM_PREVIEW_ITEMS);
    if preview.is_empty() {
        None
    } else {
        Some(format!("Returned {preview}."))
    }
}

fn spine_tool_stream_preview(value: &serde_json::Value, max_items: usize) -> String {
    let mut parts = Vec::new();
    collect_spine_tool_stream_preview(None, value, max_items, &mut parts);
    parts.join("; ")
}

fn collect_spine_tool_stream_preview(
    label: Option<&str>,
    value: &serde_json::Value,
    max_items: usize,
    out: &mut Vec<String>,
) {
    if out.len() >= max_items {
        return;
    }
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::Bool(value) => {
            if let Some(label) = label {
                out.push(format!("{}: {}", readable_json_key(label), value));
            }
        }
        serde_json::Value::Number(value) => {
            if let Some(label) = label {
                out.push(format!("{}: {}", readable_json_key(label), value));
            }
        }
        serde_json::Value::String(text) => {
            let text = text.trim();
            if text.is_empty() || text == "[redacted]" {
                return;
            }
            let text = safe_truncate(text, 160);
            if let Some(label) = label {
                out.push(format!("{}: {}", readable_json_key(label), text));
            } else {
                out.push(text);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_spine_tool_stream_preview(label, item, max_items, out);
                if out.len() >= max_items {
                    break;
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                if key.starts_with('_') || is_sensitive_tool_call_argument_key(key) {
                    continue;
                }
                collect_spine_tool_stream_preview(Some(key), inner, max_items, out);
                if out.len() >= max_items {
                    break;
                }
            }
        }
    }
}

fn readable_spine_tool_name(name: &str) -> String {
    let readable = name
        .split(['_', '-'])
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if readable.is_empty() {
        "tool".to_string()
    } else {
        readable
    }
}

fn readable_json_key(key: &str) -> String {
    let readable = key
        .split(['_', '-'])
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if readable.is_empty() {
        "value".to_string()
    } else {
        readable
    }
}

pub async fn run_spine(
    request: SpineRequest,
    server: &dyn SpineLlmServer,
    tools: &ToolRegistry,
    cx: &SpineContext,
) -> SpineResult {
    let mut messages = request.messages.clone();
    // The CURRENT turn's request, captured from the pristine initial message
    // list before any synthetic reprompt/refresh User messages are appended.
    // The freshness and completion verifiers must judge against THIS — using
    // the conversation's first user message judged every follow-up turn
    // against a stale earlier goal ("install X" vs "now query X"), forcing
    // false incomplete verdicts and wasted reprompt turns.
    let current_user_request = current_user_request_text(&request.messages);
    let max_turns = completion_guarded_max_turns(request.max_turns);
    let mut completed_tool_signatures: HashMap<String, ToolProgressClass> = HashMap::new();
    let mut tool_repair_attempts: HashMap<String, usize> = HashMap::new();
    let mut completion_reprompts = 0usize;
    // Never reset by tool evidence; the run-wide backstop against a
    // permanently-failing tool refilling the per-evidence reprompt budget.
    let mut total_completion_reprompts = 0usize;
    let current_turn_evidence_start = messages.len();
    let mut freshness_refresh_attempted = false;
    let mut capability_readiness_generation_seen = request.capability_readiness_generation;
    let mut capability_readiness_reloops = 0usize;

    for turn in 0..max_turns {
        if request.cancel_token.is_cancelled() {
            cx.emit(SpineTraceEvent::TurnCompleted {
                turn,
                terminal_state: SpineTerminalState::Cancelled,
                final_text_present: false,
            })
            .await;
            return SpineResult::Cancelled {
                messages,
                turns_used: turn,
                reason: "request_cancelled".to_string(),
            };
        }

        cx.emit(SpineTraceEvent::TurnStarted {
            turn,
            prompt_token_estimate: estimate_prompt_tokens(&messages),
            tool_count: PRIMITIVE_NAMES.len(),
        })
        .await;

        let response = match server
            .chat_completion(
                messages.clone(),
                tools.schemas_for_caller(request.caller_kind),
                request.streaming,
                request.visual_attachments.clone(),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                cx.emit(SpineTraceEvent::TurnCompleted {
                    turn,
                    terminal_state: SpineTerminalState::PlatformFailed,
                    final_text_present: false,
                })
                .await;
                return SpineResult::PlatformFailed {
                    messages,
                    turns_used: turn,
                    error,
                };
            }
        };

        cx.emit(SpineTraceEvent::ModelCompleted {
            turn,
            completion_tokens: response.completion_tokens,
            tool_calls_count: response.tool_calls.len(),
            cache_read_tokens: response.cache_read_tokens,
            cache_creation_tokens: response.cache_creation_tokens,
            // True provider response latency (request → final chunk), measured
            // inside chat_completion around the actual provider call.
            latency_ms: response.provider_latency_ms,
        })
        .await;

        if spine_response_is_empty_terminal(&response.text, response.tool_calls.len()) {
            cx.emit(SpineTraceEvent::TurnCompleted {
                turn,
                terminal_state: SpineTerminalState::PlatformFailed,
                final_text_present: false,
            })
            .await;
            return SpineResult::PlatformFailed {
                messages,
                turns_used: turn + 1,
                error: empty_terminal_spine_response_error(),
            };
        }

        if response.tool_calls.is_empty() {
            let visible_response_text =
                crate::core::model::llm_context_sanitizer::strip_internal_tool_transcript(
                    &response.text,
                );
            let final_text = finalize_user_text(
                SpineTerminalTextKind::Completed,
                &normalize_final_response_artifact_links(&visible_response_text, &messages),
                TerminalTextProvenance::ModelAuthored,
            );
            if matches!(request.caller_kind, CallerKind::Chat)
                && capability_readiness_reloops == 0
                && turn + 1 < max_turns
            {
                let current_generation = cx.agent.capability_readiness_generation().await;
                let previous_generation = capability_readiness_generation_seen.unwrap_or(0);
                if current_generation > previous_generation {
                    if let Some(readiness_context) =
                        cx.agent.capability_readiness_context_message().await
                    {
                        capability_readiness_generation_seen = Some(current_generation);
                        capability_readiness_reloops += 1;
                        messages.push(SpineMessage::System {
                            content: readiness_context,
                        });
                        messages.push(SpineMessage::User {
                            content: "Capability readiness state changed during this turn. Re-evaluate the answer using the updated readiness context. Treat AgentArk-managed capability readiness from that context as authoritative; do not claim a capability is ready unless it is connected and enabled there. If a requested capability is not ready, explain the readiness blocker in user-facing terms.".to_string(),
                        });
                        tracing::info!(
                            turn,
                            previous_generation,
                            current_generation,
                            "Spine re-looping after capability readiness changed during turn"
                        );
                        continue;
                    }
                }
            }
            // Freshness is established structurally when this turn already
            // produced successful tool evidence, when a refresh already ran,
            // or when AgentArk-managed capability readiness was supplied via
            // the typed registry context. No user wording participates in
            // this decision.
            let terminal_gate_started = std::time::Instant::now();
            let audit_decision = terminal_audit_decision(
                SpineTerminalTextKind::Completed,
                TerminalTextProvenance::ModelAuthored,
                request.caller_kind,
                &messages,
                current_turn_evidence_start,
                completion_reprompts,
                capability_readiness_reloops,
            );
            let managed_capability_readiness_established =
                has_capability_readiness_context_evidence(&messages);
            let freshness_established = freshness_refresh_attempted
                || managed_capability_readiness_established
                || current_turn_has_successful_tool_evidence(
                    &messages,
                    current_turn_evidence_start,
                );
            let (freshness_verdict, completion_verdict) = if audit_decision
                == TerminalAuditDecision::Skip
            {
                tracing::info!(
                    turn,
                    "spine_terminal_audit_skipped: structural no-action terminal"
                );
                (FreshnessVerdict::Fresh, CompletionVerdict::Complete)
            } else if freshness_established {
                if managed_capability_readiness_established {
                    tracing::debug!(
                        turn,
                        "Spine skipped LLM freshness verdict using capability readiness registry evidence"
                    );
                }
                let completion = verification_verdict_for_terminal_candidate(
                    server,
                    cx,
                    &current_user_request,
                    &messages,
                    request.caller_kind,
                    SpineTerminalTextKind::Completed,
                    TerminalTextProvenance::ModelAuthored,
                    turn,
                    &final_text,
                )
                .await;
                (FreshnessVerdict::Fresh, completion)
            } else {
                combined_terminal_verdicts(
                    server,
                    cx,
                    &current_user_request,
                    &messages,
                    current_turn_evidence_start,
                    turn,
                    &final_text,
                )
                .await
            };
            if let Some(refresh_call) = freshness_refresh_tool_call(&freshness_verdict) {
                // The merged completion verdict (if any) is intentionally
                // discarded here: the refresh produces new evidence that
                // invalidates it, and the next iteration re-verifies.
                freshness_refresh_attempted = true;
                let refresh_started = std::time::Instant::now();
                let partial_text =
                    "Checking the current integration state before answering.".to_string();
                messages.push(SpineMessage::Assistant {
                    content: Some(partial_text),
                    tool_calls: vec![refresh_call.clone()],
                });
                let result = execute_prepared_tool_call(
                    tools.clone(),
                    cx.clone(),
                    refresh_call.clone(),
                    false,
                )
                .await;
                messages.push(SpineMessage::Tool {
                    tool_call_id: refresh_call.id.clone(),
                    content: result.to_json_for_tool(&refresh_call),
                });
                if tool_result_progresses_completion_reprompt_budget(&result) {
                    completion_reprompts =
                        completion_reprompts_after_tool_evidence(completion_reprompts);
                }
                tracing::info!(
                    turn,
                    elapsed_ms = refresh_started.elapsed().as_millis() as u64,
                    "Spine freshness refresh tool call executed before terminal accept"
                );
                continue;
            }
            match next_completion_step(
                completion_verdict,
                completion_reprompts,
                total_completion_reprompts,
                turn,
                max_turns,
            ) {
                CompletionStep::Accept => {}
                CompletionStep::Reprompt { prompt } => {
                    completion_reprompts += 1;
                    total_completion_reprompts += 1;
                    tracing::info!(
                        turn,
                        completion_reprompts,
                        total_completion_reprompts,
                        elapsed_ms = terminal_gate_started.elapsed().as_millis() as u64,
                        "Spine completion reprompt issued: terminal answer deferred for another turn"
                    );
                    messages.push(SpineMessage::User { content: prompt });
                    continue;
                }
                CompletionStep::AcceptWithCaveat { message } => {
                    let answer = incomplete_terminal_acceptance_text(
                        &message,
                        &final_text,
                        TerminalTextProvenance::ModelAuthored,
                    );
                    let answer = finalize_user_text(
                        SpineTerminalTextKind::Blocked,
                        &answer,
                        TerminalTextProvenance::SystemAuthored,
                    );
                    tracing::info!(
                        turn,
                        total_completion_reprompts,
                        completion_gap = %message,
                        "Spine blocking incomplete model-authored terminal answer after reprompt budget"
                    );
                    messages.push(SpineMessage::Assistant {
                        content: Some(answer.clone()),
                        tool_calls: Vec::new(),
                    });
                    cx.emit(SpineTraceEvent::TurnCompleted {
                        turn,
                        terminal_state: SpineTerminalState::Blocked,
                        final_text_present: true,
                    })
                    .await;
                    return SpineResult::Blocked {
                        messages,
                        final_text: answer,
                        turns_used: turn + 1,
                    };
                }
            }
            messages.push(SpineMessage::Assistant {
                content: Some(final_text.clone()),
                tool_calls: Vec::new(),
            });
            cx.emit(SpineTraceEvent::TurnCompleted {
                turn,
                terminal_state: SpineTerminalState::Completed,
                final_text_present: !final_text.trim().is_empty(),
            })
            .await;
            return SpineResult::Completed {
                messages,
                final_text,
                turns_used: turn + 1,
            };
        }

        let partial_text = spine_visible_partial_text(response.partial_text.as_deref());
        if let (Some(stream_tx), Some(text)) = (cx.stream_tx.as_ref(), partial_text.as_ref()) {
            queue_stream_event(stream_tx, spine_model_prose_stream_event(turn, text));
        }
        messages.push(SpineMessage::Assistant {
            content: partial_text.clone(),
            tool_calls: response.tool_calls.clone(),
        });

        for tool_call in &response.tool_calls {
            if let Some(envelope) = tools.approval_envelope_for_call(tool_call, cx).await {
                cx.set_paused_approval(envelope).await;
                cx.emit(SpineTraceEvent::TurnCompleted {
                    turn,
                    terminal_state: SpineTerminalState::PausedForApproval,
                    final_text_present: false,
                })
                .await;
                return SpineResult::PausedForApproval {
                    messages,
                    turns_used: turn + 1,
                    pending_call: tool_call.clone(),
                };
            }
        }

        let prepared_calls = response
            .tool_calls
            .iter()
            .cloned()
            .map(|tool_call| {
                let signature = tool_call_progress_signature(&tool_call);
                let blocked = signature.as_ref().is_some_and(|signature| {
                    completed_tool_signatures.contains_key(&signature.key)
                });
                (tool_call, signature, blocked)
            })
            .collect::<Vec<_>>();

        let batches = dependency_batches_for_tool_calls(
            &prepared_calls,
            cx.request.conversation_id.as_deref(),
        );
        let mut results_by_index = vec![None; prepared_calls.len()];
        for batch in batches {
            let futures = batch
                .iter()
                .map(|idx| {
                    let (tool_call, _signature, blocked) = &prepared_calls[*idx];
                    execute_prepared_tool_call(
                        tools.clone(),
                        cx.clone(),
                        tool_call.clone(),
                        *blocked,
                    )
                })
                .collect::<Vec<_>>();
            let batch_results = join_all(futures).await;
            for (idx, result) in batch.into_iter().zip(batch_results) {
                results_by_index[idx] = Some(result);
            }
        }
        let results = results_by_index
            .into_iter()
            .map(|result| {
                result.unwrap_or_else(|| {
                    ToolResult::from_value(
                        false,
                        tool_result_error(
                            "spine",
                            "dependency_scheduler_failed",
                            "A scheduled tool call did not produce a result.",
                        ),
                    )
                })
            })
            .collect::<Vec<_>>();

        for ((tool_call, signature, blocked), result) in prepared_calls.iter().zip(results.iter()) {
            if result.ok && !blocked {
                if let Some(signature) = signature {
                    if signature.class == ToolProgressClass::Mutation {
                        completed_tool_signatures
                            .retain(|_, class| *class != ToolProgressClass::ReadOnly);
                    }
                    completed_tool_signatures.insert(signature.key.clone(), signature.class);
                }
            }
            messages.push(SpineMessage::Tool {
                tool_call_id: tool_call.id.clone(),
                content: result.to_json_for_tool(tool_call),
            });
        }
        if prepared_calls
            .iter()
            .zip(results.iter())
            .any(|((_, _, blocked), result)| {
                !*blocked && tool_result_progresses_completion_reprompt_budget(result)
            })
        {
            completion_reprompts = completion_reprompts_after_tool_evidence(completion_reprompts);
        }
        for ((tool_call, _, blocked), result) in prepared_calls.iter().zip(results.iter()) {
            if *blocked || !tool_result_requires_model_repair(result) {
                continue;
            }
            if let Some(content) =
                note_tool_repair_attempt(&mut tool_repair_attempts, &tool_call.name)
            {
                messages.push(SpineMessage::User { content });
            }
        }
        if let Some(final_text) = failed_search_message_from_tool_results(&results) {
            let mut final_text = finalize_user_text(
                SpineTerminalTextKind::Blocked,
                &final_text,
                TerminalTextProvenance::SystemAuthored,
            );
            let terminal_gate_started = std::time::Instant::now();
            let audit_decision = terminal_audit_decision(
                SpineTerminalTextKind::Blocked,
                TerminalTextProvenance::SystemAuthored,
                request.caller_kind,
                &messages,
                current_turn_evidence_start,
                completion_reprompts,
                capability_readiness_reloops,
            );
            let completion_verdict = if audit_decision == TerminalAuditDecision::Skip {
                CompletionVerdict::Complete
            } else {
                verification_verdict_for_terminal_candidate(
                    server,
                    cx,
                    &current_user_request,
                    &messages,
                    request.caller_kind,
                    SpineTerminalTextKind::Blocked,
                    TerminalTextProvenance::SystemAuthored,
                    turn,
                    &final_text,
                )
                .await
            };
            match next_completion_step(
                completion_verdict,
                completion_reprompts,
                total_completion_reprompts,
                turn,
                max_turns,
            ) {
                CompletionStep::Accept => {}
                CompletionStep::Reprompt { prompt } => {
                    completion_reprompts += 1;
                    total_completion_reprompts += 1;
                    tracing::info!(
                        turn,
                        completion_reprompts,
                        total_completion_reprompts,
                        elapsed_ms = terminal_gate_started.elapsed().as_millis() as u64,
                        "Spine completion reprompt issued: blocked terminal answer deferred for another turn"
                    );
                    messages.push(SpineMessage::User { content: prompt });
                    continue;
                }
                CompletionStep::AcceptWithCaveat { message } => {
                    let answer = incomplete_terminal_acceptance_text(
                        &message,
                        &final_text,
                        TerminalTextProvenance::SystemAuthored,
                    );
                    tracing::info!(
                        turn,
                        total_completion_reprompts,
                        completion_gap = %message,
                        "Spine returning incomplete blocked terminal answer after reprompt budget"
                    );
                    final_text = finalize_user_text(
                        SpineTerminalTextKind::Blocked,
                        &answer,
                        TerminalTextProvenance::SystemAuthored,
                    );
                }
            }
            messages.push(SpineMessage::Assistant {
                content: Some(final_text.clone()),
                tool_calls: Vec::new(),
            });
            cx.emit(SpineTraceEvent::TurnCompleted {
                turn,
                terminal_state: SpineTerminalState::Blocked,
                final_text_present: true,
            })
            .await;
            return SpineResult::Blocked {
                messages,
                final_text,
                turns_used: turn + 1,
            };
        }
        if let Some(final_text) = needs_input_message_from_tool_results(&results) {
            let final_text = finalize_user_text(
                SpineTerminalTextKind::NeedsInput,
                &final_text,
                TerminalTextProvenance::StructuredUserQuestion,
            );
            messages.push(SpineMessage::Assistant {
                content: Some(final_text.clone()),
                tool_calls: Vec::new(),
            });
            cx.emit(SpineTraceEvent::TurnCompleted {
                turn,
                terminal_state: SpineTerminalState::NeedsInput,
                final_text_present: true,
            })
            .await;
            return SpineResult::NeedsInput {
                messages,
                final_text,
                turns_used: turn + 1,
            };
        }
    }

    cx.emit(SpineTraceEvent::TurnCompleted {
        turn: max_turns,
        terminal_state: SpineTerminalState::MaxTurnsExceeded,
        final_text_present: false,
    })
    .await;
    SpineResult::MaxTurnsExceeded {
        messages,
        turns_used: max_turns,
    }
}

fn spine_visible_partial_text(raw_text: Option<&str>) -> Option<String> {
    raw_text
        .map(crate::core::model::llm_context_sanitizer::strip_internal_tool_transcript)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn spine_model_prose_stream_event(turn: usize, text: &str) -> StreamEvent {
    StreamEvent::ToolProgress {
        name: "agent_model".to_string(),
        content: text.to_string(),
        payload: Some(serde_json::json!({
            "kind": "model_prose",
            "content": text,
            "content_snapshot": text,
            "stream_key": format!("model-prose:{turn}"),
            "done": true,
        })),
    }
}

async fn execute_prepared_tool_call(
    tools: ToolRegistry,
    cx: SpineContext,
    tool_call: SpineToolCall,
    blocked: bool,
) -> ToolResult {
    let start_payload = spine_tool_start_stream_payload(&tool_call);
    if let Some(stream_tx) = cx.stream_tx.as_ref() {
        queue_stream_event(
            stream_tx,
            StreamEvent::ToolStart {
                name: tool_call.name.clone(),
                payload: Some(start_payload.clone()),
            },
        );
    }
    cx.emit(SpineTraceEvent::ToolStarted {
        tool_call_id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        arguments: start_payload.get("arguments").cloned(),
        activity_label: start_payload
            .get("activity_label")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        display_label: start_payload
            .get("display_label")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        intent_summary: start_payload
            .get("intent_summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
    .await;
    let result = if blocked {
        ToolResult::from_value(
            false,
            tool_result_error_with_extra(
                &tool_call.name,
                "repeated_no_progress_tool_call",
                "This exact successful tool request already completed in this run. Use the previous result to continue, or call a different primitive or different arguments for remaining work.",
                serde_json::json!({
                    "hint": "Do not repeat identical completed reads, status checks, or creates inside one run."
                }),
            ),
        )
    } else {
        tools.dispatch(tool_call.clone(), cx.clone()).await
    };
    if let Some(stream_tx) = cx.stream_tx.as_ref() {
        queue_stream_event(
            stream_tx,
            StreamEvent::ToolResult {
                name: tool_call.name.clone(),
                content: spine_tool_result_stream_content(&tool_call, &result),
            },
        );
    }
    cx.emit(SpineTraceEvent::ToolCompleted {
        tool_call_id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        ok: result.ok,
        summary: result.summary(),
    })
    .await;
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolProgressClass {
    ReadOnly,
    Mutation,
}

#[derive(Debug, Clone)]
struct ToolProgressSignature {
    key: String,
    class: ToolProgressClass,
}

#[derive(Debug, Clone, Default)]
struct ToolDependencyProfile {
    reads: HashSet<String>,
    writes: HashSet<String>,
    read_domains: HashSet<String>,
    write_domains: HashSet<String>,
    barrier: bool,
}

impl ToolDependencyProfile {
    fn read(mut self, key: impl Into<String>) -> Self {
        self.reads.insert(key.into());
        self
    }

    fn write(mut self, key: impl Into<String>) -> Self {
        self.writes.insert(key.into());
        self
    }

    fn read_domain(mut self, domain: impl Into<String>) -> Self {
        self.read_domains.insert(domain.into());
        self
    }

    fn write_domain(mut self, domain: impl Into<String>) -> Self {
        self.write_domains.insert(domain.into());
        self
    }

    fn barrier() -> Self {
        Self {
            barrier: true,
            ..Self::default()
        }
    }
}

fn dependency_batches_for_tool_calls(
    prepared_calls: &[(SpineToolCall, Option<ToolProgressSignature>, bool)],
    conversation_id: Option<&str>,
) -> Vec<Vec<usize>> {
    let profiles = prepared_calls
        .iter()
        .map(|(call, _, _)| tool_dependency_profile(call, conversation_id))
        .collect::<Vec<_>>();
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut assigned_batch = vec![0usize; prepared_calls.len()];

    for idx in 0..prepared_calls.len() {
        let mut min_batch = 0usize;
        for prev in 0..idx {
            if tool_dependencies_conflict(&profiles[prev], &profiles[idx]) {
                min_batch = min_batch.max(assigned_batch[prev].saturating_add(1));
            }
        }
        while batches.len() <= min_batch {
            batches.push(Vec::new());
        }
        assigned_batch[idx] = min_batch;
        batches[min_batch].push(idx);
    }

    batches
}

fn tool_dependencies_conflict(left: &ToolDependencyProfile, right: &ToolDependencyProfile) -> bool {
    if left.barrier || right.barrier {
        return true;
    }
    left.writes
        .iter()
        .any(|key| right.reads.contains(key) || right.writes.contains(key))
        || right
            .writes
            .iter()
            .any(|key| left.reads.contains(key) || left.writes.contains(key))
        || left.writes.iter().any(|key| {
            resource_key_domain(key).is_some_and(|domain| right.read_domains.contains(domain))
        })
        || right.writes.iter().any(|key| {
            resource_key_domain(key).is_some_and(|domain| left.read_domains.contains(domain))
        })
        || left.write_domains.iter().any(|domain| {
            right.read_domains.contains(domain)
                || right.write_domains.contains(domain)
                || right
                    .reads
                    .iter()
                    .chain(right.writes.iter())
                    .any(|key| resource_key_domain(key) == Some(domain.as_str()))
        })
        || right.write_domains.iter().any(|domain| {
            left.read_domains.contains(domain)
                || left.write_domains.contains(domain)
                || left
                    .reads
                    .iter()
                    .chain(left.writes.iter())
                    .any(|key| resource_key_domain(key) == Some(domain.as_str()))
        })
        || left
            .read_domains
            .iter()
            .any(|domain| right.write_domains.contains(domain))
        || right
            .read_domains
            .iter()
            .any(|domain| left.write_domains.contains(domain))
}

fn resource_key_domain(key: &str) -> Option<&str> {
    key.split_once(':')
        .map(|(domain, _)| domain)
        .filter(|domain| !domain.is_empty())
}

fn normalized_resource_part(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.trim().is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn file_resource_key(path: &str) -> Option<String> {
    normalized_resource_part(path).map(|path| format!("file:{}", path))
}

fn source_file_resource_key(source_dir: &str, source_path: &str) -> Option<String> {
    let dir = normalized_resource_part(source_dir)?;
    let path = normalized_resource_part(source_path)?;
    Some(format!("file:{}/{}", dir, path))
}

fn resource_target_key(kind: &str, arguments: &serde_json::Value) -> Option<String> {
    json_text(arguments, "id")
        .or_else(|| json_text(arguments, "app_id"))
        .or_else(|| json_text(arguments, "service_id"))
        .or_else(|| json_text(arguments, "task_id"))
        .or_else(|| json_text(arguments, "watcher_id"))
        .or_else(|| json_text(arguments, "background_session_id"))
        .or_else(|| json_text(arguments, "name"))
        .or_else(|| json_text_path(arguments, &["content", "id"]))
        .or_else(|| json_text_path(arguments, &["content", "app_id"]))
        .or_else(|| json_text_path(arguments, &["content", "task_id"]))
        .or_else(|| json_text_path(arguments, &["content", "watcher_id"]))
        .or_else(|| json_text_path(arguments, &["content", "background_session_id"]))
        .or_else(|| json_text_path(arguments, &["content", "name"]))
        .or_else(|| json_text(arguments, "query"))
        .and_then(|value| normalized_resource_part(&value))
        .map(|target| format!("{}:{}", kind, target))
}

fn tool_dependency_profile(
    call: &SpineToolCall,
    conversation_id: Option<&str>,
) -> ToolDependencyProfile {
    match call.name.as_str() {
        "file_read" => json_text(&call.arguments, "path")
            .and_then(|path| file_resource_key(&path))
            .map(|key| ToolDependencyProfile::default().read(key))
            .unwrap_or_else(|| ToolDependencyProfile::default().read_domain("file")),
        "file_search" => ToolDependencyProfile::default().read_domain("file"),
        "file_write" | "file_delete" => json_text(&call.arguments, "path")
            .and_then(|path| file_resource_key(&path))
            .map(|key| ToolDependencyProfile::default().write(key))
            .unwrap_or_else(|| ToolDependencyProfile::default().write_domain("file")),
        "file_patch" => file_patch_dependency_profile(&call.arguments),
        "app_deploy" => app_deploy_dependency_profile(&call.arguments, conversation_id),
        "skill_manage" => skill_manage_dependency_profile(&call.arguments),
        "resource_rw" => resource_rw_dependency_profile(&call.arguments),
        "memory_rw" => memory_rw_dependency_profile(&call.arguments),
        "pdf_generate" => ToolDependencyProfile::default().write_domain("document"),
        "code_exec" | "browse" | "delegate" => ToolDependencyProfile::barrier(),
        "fetch" => fetch_dependency_profile(&call.arguments),
        "search" => ToolDependencyProfile::default(),
        _ => ToolDependencyProfile::barrier(),
    }
}

fn file_patch_dependency_profile(arguments: &serde_json::Value) -> ToolDependencyProfile {
    let mut profile = ToolDependencyProfile::default();
    if let Some(path) = json_text(arguments, "path").and_then(|path| file_resource_key(&path)) {
        profile = profile.read(path.clone()).write(path);
    }
    if let Some(patches) = arguments.get("patches").and_then(|value| value.as_array()) {
        for patch in patches {
            if let Some(path) = json_text(patch, "path").and_then(|path| file_resource_key(&path)) {
                profile = profile.read(path.clone()).write(path);
            }
        }
    }
    if profile.reads.is_empty() && profile.writes.is_empty() {
        profile.write_domain("file")
    } else {
        profile
    }
}

fn app_deploy_dependency_profile(
    arguments: &serde_json::Value,
    conversation_id: Option<&str>,
) -> ToolDependencyProfile {
    let mut profile = ToolDependencyProfile::default();
    if let Some(source_dir) = json_text(arguments, "source_dir") {
        if let Some(paths) = arguments
            .get("source_paths")
            .and_then(|value| value.as_array())
        {
            if paths.is_empty() {
                profile = profile.read_domain("file");
            } else {
                for path in paths.iter().filter_map(|value| value.as_str()) {
                    if let Some(key) = source_file_resource_key(&source_dir, path) {
                        profile = profile.read(key);
                    }
                }
            }
        } else {
            profile = profile.read_domain("file");
        }
    }

    let app_key = json_text(arguments, "app_id")
        .and_then(|app_id| normalized_resource_part(&app_id))
        .map(|app_id| format!("app:{}", app_id))
        .or_else(|| {
            conversation_id
                .and_then(normalized_resource_part)
                .map(|id| format!("app:conversation:{}", id))
        });
    if let Some(app_key) = app_key {
        profile.write(app_key)
    } else {
        profile.write_domain("app")
    }
}

fn skill_manage_dependency_profile(arguments: &serde_json::Value) -> ToolDependencyProfile {
    let operation = json_text(arguments, "operation")
        .unwrap_or_else(|| "list".to_string())
        .to_ascii_lowercase();
    let target = resource_target_key("skill", arguments);
    let read_only = matches!(operation.as_str(), "list" | "read" | "status");
    match (read_only, target) {
        (true, Some(key)) => ToolDependencyProfile::default().read(key),
        (true, None) => ToolDependencyProfile::default().read_domain("skill"),
        (false, Some(key)) => ToolDependencyProfile::default().write(key),
        (false, None) => ToolDependencyProfile::default().write_domain("skill"),
    }
}

fn resource_rw_dependency_profile(arguments: &serde_json::Value) -> ToolDependencyProfile {
    let kind = json_text(arguments, "kind")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "resource".to_string());
    let op = json_text(arguments, "op")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "read".to_string());
    if kind == "file" {
        let key = json_text_path(arguments, &["content", "path"])
            .or_else(|| json_text(arguments, "id"))
            .or_else(|| json_text(arguments, "query"))
            .and_then(|path| file_resource_key(&path));
        return if matches!(op.as_str(), "read" | "status") {
            key.map(|key| ToolDependencyProfile::default().read(key))
                .unwrap_or_else(|| ToolDependencyProfile::default().read_domain("file"))
        } else if op == "list" {
            ToolDependencyProfile::default().read_domain("file")
        } else {
            key.map(|key| ToolDependencyProfile::default().write(key))
                .unwrap_or_else(|| ToolDependencyProfile::default().write_domain("file"))
        };
    }

    let domain = if matches!(kind.as_str(), "app_service" | "dashboard") {
        "app".to_string()
    } else {
        format!("resource_{}", kind)
    };
    let target = resource_target_key(&domain, arguments);
    if matches!(op.as_str(), "read" | "status" | "list") {
        target
            .map(|key| ToolDependencyProfile::default().read(key))
            .unwrap_or_else(|| ToolDependencyProfile::default().read_domain(domain))
    } else {
        target
            .map(|key| ToolDependencyProfile::default().write(key))
            .unwrap_or_else(|| ToolDependencyProfile::default().write_domain(domain))
    }
}

fn memory_rw_dependency_profile(arguments: &serde_json::Value) -> ToolDependencyProfile {
    let op = json_text(arguments, "op")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "search".to_string());
    let key = json_text(arguments, "id")
        .or_else(|| json_text_path(arguments, &["content", "key"]))
        .and_then(|value| normalized_resource_part(&value))
        .map(|value| format!("memory:{}", value));
    if matches!(op.as_str(), "search" | "read") {
        key.map(|key| ToolDependencyProfile::default().read(key))
            .unwrap_or_else(|| ToolDependencyProfile::default().read_domain("memory"))
    } else {
        key.map(|key| ToolDependencyProfile::default().write(key))
            .unwrap_or_else(|| ToolDependencyProfile::default().write_domain("memory"))
    }
}

fn fetch_dependency_profile(arguments: &serde_json::Value) -> ToolDependencyProfile {
    if json_text(arguments, "url").is_some_and(|url| url.contains("/apps/")) {
        ToolDependencyProfile::default().read_domain("app")
    } else {
        ToolDependencyProfile::default()
    }
}

#[derive(Debug, Clone)]
struct FinalAppLink {
    id: String,
    title: String,
    url: String,
}

fn collect_final_app_links(messages: &[SpineMessage]) -> Vec<FinalAppLink> {
    let mut links = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for message in messages {
        let SpineMessage::Tool { content, .. } = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
            continue;
        };
        let app = value.get("app").unwrap_or(&value);
        let id = json_value_text(app, "id").or_else(|| json_value_text(app, "app_id"));
        let url = json_value_text(app, "url")
            .or_else(|| json_value_text(app, "access_url"))
            .or_else(|| id.as_ref().map(|id| format!("/apps/{}/", id)));
        let Some(url) = url.filter(|value| value.trim_start().starts_with("/apps/")) else {
            continue;
        };
        let id = id.or_else(|| {
            url.trim_start_matches("/apps/")
                .split('/')
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        });
        let Some(id) = id else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        links.push(FinalAppLink {
            title: json_value_text(app, "title").unwrap_or_else(|| "Open app".to_string()),
            id,
            url,
        });
    }
    links
}

fn markdown_link_for_app(link: &FinalAppLink) -> String {
    let label = link
        .title
        .replace('[', "(")
        .replace(']', ")")
        .trim()
        .to_string();
    let label = if label.is_empty() {
        "Open app".to_string()
    } else {
        label
    };
    format!("[{}]({})", label, link.url)
}

fn response_has_markdown_app_link(text: &str, app_id: &str) -> bool {
    let pattern = format!(r"\]\(/apps/{}(?:/|[?#])?[^)]*\)", regex::escape(app_id));
    regex::Regex::new(&pattern)
        .ok()
        .is_some_and(|regex| regex.is_match(text))
}

fn normalize_final_response_artifact_links(text: &str, messages: &[SpineMessage]) -> String {
    let app_links = collect_final_app_links(messages);
    if app_links.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    for link in &app_links {
        let markdown_link = markdown_link_for_app(link);
        let escaped_id = regex::escape(&link.id);
        if let Ok(markdown_href_re) = regex::Regex::new(&format!(
            r"\[([^\]]+)\]\(https?://[^\s)]+/apps/{}(?:/[^)\s]*)?\)",
            escaped_id
        )) {
            out = markdown_href_re
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    let label = caps.get(1).map(|m| m.as_str()).unwrap_or("Open app");
                    format!("[{}]({})", label, link.url)
                })
                .into_owned();
        }
        if let Ok(bare_absolute_re) = regex::Regex::new(&format!(
            r"https?://[^\s<>()]+/apps/{}(?:/[^\s<>()]*)?",
            escaped_id
        )) {
            out = bare_absolute_re
                .replace_all(&out, markdown_link.as_str())
                .into_owned();
        }
    }
    let missing_links = app_links
        .iter()
        .filter(|link| !response_has_markdown_app_link(&out, &link.id))
        .map(markdown_link_for_app)
        .collect::<Vec<_>>();
    if !missing_links.is_empty() {
        let suffix = missing_links
            .into_iter()
            .map(|link| format!("App: {}", link))
            .collect::<Vec<_>>()
            .join("\n");
        if out.trim().is_empty() {
            out = suffix;
        } else {
            out.push_str("\n\n");
            out.push_str(&suffix);
        }
    }
    out
}

fn tool_call_progress_signature(call: &SpineToolCall) -> Option<ToolProgressSignature> {
    let class = match call.name.as_str() {
        "resource_rw" => match json_text(&call.arguments, "op")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "read" | "status" | "list" => ToolProgressClass::ReadOnly,
            "create" | "update" | "delete" | "pause" | "resume" | "stop" | "cancel"
            | "update_delivery" => ToolProgressClass::Mutation,
            _ => return None,
        },
        "memory_rw" => match json_text(&call.arguments, "op")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "search" | "read" => ToolProgressClass::ReadOnly,
            "write" | "update" | "delete" => ToolProgressClass::Mutation,
            _ => return None,
        },
        "search" | "fetch" => ToolProgressClass::ReadOnly,
        "pdf_generate" => ToolProgressClass::Mutation,
        "file_read" | "file_search" => ToolProgressClass::ReadOnly,
        "file_write" | "file_patch" | "file_delete" | "app_deploy" => ToolProgressClass::Mutation,
        "skill_manage" => match json_text(&call.arguments, "operation")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "list" | "read" | "status" => ToolProgressClass::ReadOnly,
            _ => ToolProgressClass::Mutation,
        },
        _ => return None,
    };
    let signature_args =
        tool_call_progress_identity(call).unwrap_or_else(|| call.arguments.clone());
    Some(ToolProgressSignature {
        key: format!("{}:{}", call.name, canonical_json_string(&signature_args)),
        class,
    })
}

/// Stable, bounded fingerprint of a JSON value for progress-dedup identity.
/// Lets file_write/file_patch keys reflect CONTENT without bloating the key with
/// the full payload, so a changed rewrite is distinct from an identical repeat.
fn progress_value_fingerprint(value: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_json_string(value).as_bytes());
    hex::encode(hasher.finalize())
}

fn spine_candidate_error_is_retryable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::core::model::llm::LlmStreamFailure>()
        .is_some_and(|failure| failure.retryable_model_stream_failure())
}

fn file_write_progress_material(arguments: &serde_json::Value) -> serde_json::Value {
    let mut material = serde_json::Map::new();
    for key in [
        "content",
        "content_base64",
        "source_resource",
        "source_path",
        "content_type",
        "document_visible",
        "index_document",
        "duplicate_policy",
        "allow_duplicate",
        "metadata",
    ] {
        if let Some(value) = arguments.get(key) {
            material.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(material)
}

fn file_patch_progress_material(arguments: &serde_json::Value) -> serde_json::Value {
    let mut material = serde_json::Map::new();
    for key in ["patch", "patches", "dry_run"] {
        if let Some(value) = arguments.get(key) {
            material.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(material)
}

fn tool_call_progress_identity(call: &SpineToolCall) -> Option<serde_json::Value> {
    if call.name == "pdf_generate" {
        return Some(serde_json::json!({
            "title": json_text(&call.arguments, "title"),
            "filename": json_text(&call.arguments, "filename"),
        }));
    }
    if call.name == "app_deploy" {
        return Some(serde_json::json!({
            "app_id": json_text(&call.arguments, "app_id"),
            "title": json_text(&call.arguments, "title"),
            "repo_url": json_text(&call.arguments, "repo_url"),
            "repo_ref": json_text(&call.arguments, "repo_ref"),
            "repo_subdir": json_text(&call.arguments, "repo_subdir"),
            "source_dir": json_text(&call.arguments, "source_dir"),
            "source_paths": call.arguments.get("source_paths").cloned(),
            "artifact_identity": call.arguments.get("artifact_identity")
                .or_else(|| call.arguments.pointer("/metadata/artifact_identity"))
                .cloned(),
        }));
    }
    if matches!(call.name.as_str(), "file_read" | "file_delete") {
        // Re-reading or re-deleting the same path is genuinely no new progress.
        return Some(serde_json::json!({
            "path": json_text(&call.arguments, "path"),
        }));
    }
    if call.name == "file_write" {
        // Content-aware identity: a CORRECTED rewrite of the same path is real
        // progress and must NOT be deduped as a repeated no-progress call — only
        // an identical rewrite (same path AND same body/source options) is.
        return Some(serde_json::json!({
            "path": json_text(&call.arguments, "path"),
            "material_fp": progress_value_fingerprint(&file_write_progress_material(&call.arguments)),
        }));
    }
    if call.name == "file_patch" {
        // Same principle: a different patch to the same path is real progress, so
        // fingerprint the patch payload rather than keying on path alone.
        return Some(serde_json::json!({
            "path": json_text(&call.arguments, "path"),
            "material_fp": progress_value_fingerprint(&file_patch_progress_material(&call.arguments)),
        }));
    }
    if call.name == "file_search" {
        return Some(serde_json::json!({
            "query": json_text(&call.arguments, "query"),
            "filename_query": json_text(&call.arguments, "filename_query"),
            "content_query": json_text(&call.arguments, "content_query"),
            "root": json_text(&call.arguments, "root"),
            "globs": call.arguments.get("globs").cloned(),
        }));
    }
    if call.name == "skill_manage" {
        return Some(serde_json::json!({
            "operation": json_text(&call.arguments, "operation"),
            "name": json_text(&call.arguments, "name"),
            "id": json_text(&call.arguments, "id"),
            "url": json_text(&call.arguments, "url"),
        }));
    }
    if call.name != "resource_rw" {
        return None;
    }
    let kind = json_text(&call.arguments, "kind")?.to_ascii_lowercase();
    let op = json_text(&call.arguments, "op")?.to_ascii_lowercase();
    let payload = call
        .arguments
        .get("content")
        .or_else(|| call.arguments.get("metadata"))
        .cloned();
    let id = json_text(&call.arguments, "id")
        .or_else(|| json_text_path(&call.arguments, &["content", "path"]))
        .or_else(|| json_text_path(&call.arguments, &["content", "name"]))
        .or_else(|| json_text_path(&call.arguments, &["content", "title"]))
        .or_else(|| json_text_path(&call.arguments, &["metadata", "title"]));
    let query = json_text(&call.arguments, "query");
    if id.is_none() && query.is_none() {
        return None;
    }
    Some(serde_json::json!({
        "kind": kind,
        "op": op,
        "id": id,
        "query": query,
        "payload": if matches!(kind.as_str(), "custom_api" | "integration") && matches!(op.as_str(), "create" | "update" | "install" | "connect") {
            payload
        } else {
            None
        }
    }))
}

pub struct AgentSpineLlmServer {
    agent: Agent,
    channel: String,
    stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    trace: Arc<SpineTraceRecorder>,
    caller_kind: CallerKind,
    long_running: bool,
    conversation_id: Option<String>,
    ordered_primary_candidates: tokio::sync::Mutex<Option<Vec<LlmAttemptCandidate>>>,
    prompt_profiles: tokio::sync::Mutex<Option<AgentSpinePromptProfiles>>,
}

#[derive(Clone)]
struct AgentSpinePromptProfiles {
    primary_response: crate::core::self_evolve::PromptBundleProfile,
    fragments: crate::core::model::prompt_fragments::PromptFragmentBundleProfile,
}

impl AgentSpineLlmServer {
    pub fn new(
        agent: Agent,
        channel: impl Into<String>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        trace: Arc<SpineTraceRecorder>,
        caller_kind: CallerKind,
        long_running: bool,
        conversation_id: Option<String>,
    ) -> Self {
        Self {
            agent,
            channel: channel.into(),
            stream_tx,
            trace,
            caller_kind,
            long_running,
            conversation_id,
            ordered_primary_candidates: tokio::sync::Mutex::new(None),
            prompt_profiles: tokio::sync::Mutex::new(None),
        }
    }

    async fn ordered_primary_candidates(&self) -> Vec<LlmAttemptCandidate> {
        if let Some(cached) = self.ordered_primary_candidates.lock().await.clone() {
            return cached;
        }

        let mut candidates = self.agent.llm_candidates_for_role(&ModelRole::Primary);
        if candidates.is_empty() {
            candidates.push(self.agent.primary_llm_candidate());
        }
        let ordered = self
            .agent
            .reorder_candidates_with_failover(candidates, None)
            .await;

        let mut guard = self.ordered_primary_candidates.lock().await;
        if guard.is_none() {
            *guard = Some(ordered);
        }
        guard.clone().unwrap_or_default()
    }

    async fn clear_ordered_primary_candidates(&self) {
        *self.ordered_primary_candidates.lock().await = None;
    }

    async fn active_prompt_profiles(&self, seed_user_message: &str) -> AgentSpinePromptProfiles {
        if let Some(cached) = self.prompt_profiles.lock().await.clone() {
            return cached;
        }

        let primary_response = self
            .agent
            .active_prompt_bundle_for_conversation_message(
                self.conversation_id.as_deref(),
                seed_user_message,
            )
            .await;
        let fragments = self
            .agent
            .active_prompt_fragment_bundle_for_conversation_message(
                self.conversation_id.as_deref(),
                seed_user_message,
            )
            .await;
        let profiles = AgentSpinePromptProfiles {
            primary_response,
            fragments,
        };

        let mut guard = self.prompt_profiles.lock().await;
        if guard.is_none() {
            *guard = Some(profiles);
        }
        guard.clone().unwrap_or_else(|| AgentSpinePromptProfiles {
            primary_response: crate::core::self_evolve::PromptBundleProfile::default(),
            fragments: crate::core::model::prompt_fragments::default_prompt_fragment_bundle(),
        })
    }

    async fn load_llm_image_attachments(
        &self,
        hints: &[ChatAttachmentHint],
    ) -> Vec<crate::core::model::llm::LlmImageAttachment> {
        let mut attachments = Vec::new();
        for hint in hints {
            if attachments.len() >= LLM_NATIVE_IMAGE_ATTACHMENT_LIMIT {
                break;
            }
            if !chat_attachment_hint_is_visual_image(hint) {
                continue;
            }
            let upload_id = hint.upload_id.trim();
            if upload_id.is_empty() {
                continue;
            }
            let manifest = match self.agent.storage.load_upload_manifest(upload_id).await {
                Ok(Some(manifest)) => manifest,
                Ok(None) => {
                    tracing::debug!(upload_id = %upload_id, "No upload manifest for native image attachment");
                    continue;
                }
                Err(err) => {
                    tracing::debug!(upload_id = %upload_id, error = %err, "Failed to load upload manifest for native image attachment");
                    continue;
                }
            };
            let Some(mime_type) = hint
                .content_type
                .as_deref()
                .or(manifest.content_type.as_deref())
                .filter(|mime| is_supported_native_image_mime(mime))
                .map(str::to_string)
            else {
                continue;
            };
            if manifest.size_bytes > LLM_NATIVE_IMAGE_ATTACHMENT_MAX_BYTES {
                tracing::debug!(
                    upload_id = %upload_id,
                    bytes = manifest.size_bytes,
                    "Skipping oversized native image attachment"
                );
                continue;
            }
            let Some(bytes) =
                read_managed_upload_bytes(&self.agent.data_dir, &manifest.stored_name).await
            else {
                tracing::debug!(upload_id = %upload_id, "Could not read native image attachment bytes");
                continue;
            };
            if bytes.len() as u64 > LLM_NATIVE_IMAGE_ATTACHMENT_MAX_BYTES {
                continue;
            }
            let data_base64 =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
            attachments.push(crate::core::model::llm::LlmImageAttachment {
                mime_type,
                data_base64,
                label: Some(manifest.original_name),
            });
        }

        if !attachments.is_empty() {
            tracing::debug!(
                count = attachments.len(),
                "Loaded native image attachments for current model request"
            );
        }
        attachments
    }

    async fn request_candidate_completion(
        &self,
        candidate: &LlmAttemptCandidate,
        system_prompt: &str,
        prepared: &PreparedSpineMessages,
        tool_schemas: &[ActionDef],
        streaming: bool,
        image_attachments: &[crate::core::model::llm::LlmImageAttachment],
    ) -> anyhow::Result<crate::core::model::llm::LlmResponse> {
        if streaming {
            if let Some(tx) = self.stream_tx.clone() {
                if self.long_running {
                    candidate
                        .client
                        .chat_with_history_stream_for_long_running_tool_with_images(
                            system_prompt,
                            &prepared.user_message,
                            &prepared.history,
                            &[],
                            tool_schemas,
                            tx,
                            image_attachments,
                            &self.agent.config.model_privacy,
                            false,
                        )
                        .await
                } else {
                    candidate
                        .client
                        .chat_with_history_stream_for_helper_with_images(
                            system_prompt,
                            &prepared.user_message,
                            &prepared.history,
                            &[],
                            tool_schemas,
                            tx,
                            image_attachments,
                            &self.agent.config.model_privacy,
                            false,
                        )
                        .await
                }
            } else if self.long_running {
                candidate
                    .client
                    .chat_with_history_for_long_running_tool_with_images(
                        system_prompt,
                        &prepared.user_message,
                        &prepared.history,
                        &[],
                        tool_schemas,
                        image_attachments,
                        &self.agent.config.model_privacy,
                        false,
                    )
                    .await
            } else {
                candidate
                    .client
                    .chat_with_history_for_helper_with_images(
                        system_prompt,
                        &prepared.user_message,
                        &prepared.history,
                        &[],
                        tool_schemas,
                        image_attachments,
                        &self.agent.config.model_privacy,
                        false,
                    )
                    .await
            }
        } else if self.long_running {
            candidate
                .client
                .chat_with_history_for_long_running_tool_with_images(
                    system_prompt,
                    &prepared.user_message,
                    &prepared.history,
                    &[],
                    tool_schemas,
                    image_attachments,
                    &self.agent.config.model_privacy,
                    false,
                )
                .await
        } else {
            candidate
                .client
                .chat_with_history_for_helper_with_images(
                    system_prompt,
                    &prepared.user_message,
                    &prepared.history,
                    &[],
                    tool_schemas,
                    image_attachments,
                    &self.agent.config.model_privacy,
                    false,
                )
                .await
        }
    }
}

fn chat_attachment_hint_is_visual_image(hint: &ChatAttachmentHint) -> bool {
    hint.content_type
        .as_deref()
        .is_some_and(is_supported_native_image_mime)
        || matches!(
            hint.kind.trim().to_ascii_lowercase().as_str(),
            "image" | "visual"
        )
}

fn is_supported_native_image_mime(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "image/png" | "image/jpeg" | "image/jpg" | "image/webp" | "image/gif"
    )
}

fn spine_path_has_source_checkout_markers(path: &std::path::Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("src").is_dir()
}

fn spine_data_dir_looks_like_source_checkout(data_dir: &std::path::Path) -> bool {
    if spine_path_has_source_checkout_markers(data_dir) {
        return true;
    }

    let Ok(current_dir) = std::env::current_dir() else {
        return false;
    };
    if !spine_path_has_source_checkout_markers(&current_dir) {
        return false;
    }

    let canonical_data = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let canonical_current =
        std::fs::canonicalize(&current_dir).unwrap_or_else(|_| current_dir.clone());
    canonical_data == canonical_current
}

fn spine_managed_uploads_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    if !spine_data_dir_looks_like_source_checkout(data_dir) {
        return data_dir.join("uploads");
    }

    if let Some(dirs) = crate::branding::project_dirs() {
        let fallback_data_dir = dirs.data_dir().to_path_buf();
        if !spine_data_dir_looks_like_source_checkout(&fallback_data_dir) {
            return fallback_data_dir.join("uploads");
        }
    }

    std::env::temp_dir().join("agentark").join("uploads")
}

async fn read_managed_upload_bytes(
    data_dir: &std::path::Path,
    stored_name: &str,
) -> Option<Vec<u8>> {
    let uploads_dir = spine_managed_uploads_dir(data_dir);
    let uploads_root = tokio::fs::canonicalize(&uploads_dir).await.ok()?;
    let resolved = tokio::fs::canonicalize(uploads_root.join(stored_name))
        .await
        .ok()?;
    if !resolved.starts_with(&uploads_root) {
        return None;
    }
    tokio::fs::read(resolved).await.ok()
}

#[async_trait]
impl SpineLlmServer for AgentSpineLlmServer {
    async fn chat_completion(
        &self,
        messages: Vec<SpineMessage>,
        tool_schemas: Vec<ActionDef>,
        streaming: bool,
        visual_attachments: Vec<ChatAttachmentHint>,
    ) -> Result<SpineChatResponse, SpineError> {
        let prepared = prepare_spine_messages_for_llm(&messages);
        let image_attachments = if messages
            .iter()
            .any(|message| matches!(message, SpineMessage::Tool { .. }))
        {
            Vec::new()
        } else {
            self.load_llm_image_attachments(&visual_attachments).await
        };
        let candidates = self.ordered_primary_candidates().await;
        let active_prompt_profiles = self.active_prompt_profiles(&prepared.user_message).await;
        let active_prompt_bundle = active_prompt_profiles.primary_response;
        let active_prompt_fragment_bundle = active_prompt_profiles.fragments;
        let system_prompt = build_spine_system_prompt(
            &prepared.system_prompt,
            Some(&active_prompt_bundle),
            Some(&active_prompt_fragment_bundle),
        );
        self.trace
            .emit(SpineTraceEvent::PromptTelemetry {
                data: build_spine_prompt_telemetry(
                    self.caller_kind,
                    &prepared,
                    &active_prompt_bundle,
                    &active_prompt_fragment_bundle,
                    &tool_schemas,
                ),
            })
            .await;

        let mut last_error: Option<String> = None;
        for (idx, candidate) in candidates.iter().take(3).enumerate() {
            let max_attempts = if streaming {
                SPINE_MODEL_STREAM_RETRY_ATTEMPTS_PER_CANDIDATE
            } else {
                1
            };
            for attempt_idx in 0..max_attempts {
                let started = std::time::Instant::now();
                let result = self
                    .request_candidate_completion(
                        candidate,
                        &system_prompt,
                        &prepared,
                        &tool_schemas,
                        streaming,
                        &image_attachments,
                    )
                    .await;
                let result = if result.is_err() && !image_attachments.is_empty() {
                    tracing::debug!(
                        slot_id = %candidate.slot_id,
                        attempt = attempt_idx + 1,
                        "Retrying model turn without native image attachments so tool fallback can handle visuals"
                    );
                    self.request_candidate_completion(
                        candidate,
                        &system_prompt,
                        &prepared,
                        &tool_schemas,
                        streaming,
                        &[],
                    )
                    .await
                } else {
                    result
                };

                match result {
                    Ok(resp) => {
                        let provider_latency_ms = started.elapsed().as_millis() as u64;
                        self.agent
                            .record_llm_usage(&self.channel, "spine_model_turn", &resp)
                            .await;
                        if spine_response_is_empty_terminal(&resp.content, resp.tool_calls.len()) {
                            let error = empty_terminal_spine_response_error();
                            let error_text = error.message.clone();
                            last_error = Some(error_text.clone());
                            let mut attempted = Vec::new();
                            let mut attempt_records = Vec::new();
                            self.agent
                                .record_model_attempt(
                                    &mut attempted,
                                    &mut attempt_records,
                                    candidate,
                                    false,
                                    Some(&error_text),
                                    idx > 0 || attempt_idx > 0,
                                    provider_latency_ms,
                                    None,
                                )
                                .await;
                            self.clear_ordered_primary_candidates().await;
                            if attempt_idx + 1 < max_attempts {
                                tracing::warn!(
                                    slot_id = %candidate.slot_id,
                                    slot_label = %candidate.slot_label,
                                    model = %candidate.client.model_name(),
                                    provider = %candidate.client.provider_name(),
                                    attempt = attempt_idx + 1,
                                    max_attempts,
                                    "Retrying spine model candidate after empty response without tool calls"
                                );
                                continue;
                            }
                            break;
                        }
                        let mut attempted = Vec::new();
                        let mut attempt_records = Vec::new();
                        self.agent
                            .record_model_attempt(
                                &mut attempted,
                                &mut attempt_records,
                                candidate,
                                true,
                                None,
                                idx > 0 || attempt_idx > 0,
                                provider_latency_ms,
                                None,
                            )
                            .await;
                        let usage = resp.usage.clone();
                        return Ok(SpineChatResponse {
                            provider_latency_ms,
                            text: resp.content.clone(),
                            partial_text: if resp.content.trim().is_empty() {
                                None
                            } else {
                                Some(resp.content.clone())
                            },
                            tool_calls: resp
                                .tool_calls
                                .into_iter()
                                .map(|call| SpineToolCall {
                                    id: call.id,
                                    name: call.name,
                                    arguments: call.arguments,
                                    activity_label: call.activity_label,
                                })
                                .collect(),
                            completion_tokens: usage
                                .as_ref()
                                .map(|usage| usage.completion_tokens as usize)
                                .unwrap_or_default(),
                            cache_read_tokens: usage
                                .as_ref()
                                .map(|usage| usage.cached_prompt_tokens as usize)
                                .unwrap_or_default(),
                            cache_creation_tokens: usage
                                .as_ref()
                                .map(|usage| usage.cache_creation_prompt_tokens as usize)
                                .unwrap_or_default(),
                        });
                    }
                    Err(error) => {
                        let retryable_stream_error =
                            streaming && spine_candidate_error_is_retryable(&error);
                        let error_text = error.to_string();
                        last_error = Some(error_text.clone());
                        let mut attempted = Vec::new();
                        let mut attempt_records = Vec::new();
                        self.agent
                            .record_model_attempt(
                                &mut attempted,
                                &mut attempt_records,
                                candidate,
                                false,
                                Some(&error_text),
                                idx > 0 || attempt_idx > 0,
                                started.elapsed().as_millis() as u64,
                                None,
                            )
                            .await;
                        self.clear_ordered_primary_candidates().await;
                        if retryable_stream_error && attempt_idx + 1 < max_attempts {
                            tracing::warn!(
                                slot_id = %candidate.slot_id,
                                slot_label = %candidate.slot_label,
                                model = %candidate.client.model_name(),
                                provider = %candidate.client.provider_name(),
                                attempt = attempt_idx + 1,
                                max_attempts,
                                error = %safe_truncate(&error_text, 320),
                                "Retrying spine model candidate after retryable stream failure"
                            );
                            continue;
                        }
                        break;
                    }
                }
            }
        }
        Err(SpineError::new(
            "provider_exhausted",
            last_error.unwrap_or_else(|| {
                "No configured model could complete the spine turn.".to_string()
            }),
        ))
    }

    async fn terminal_audit_completion(
        &self,
        prompt: String,
    ) -> Result<SpineChatResponse, SpineError> {
        let candidates = self.ordered_primary_candidates().await;

        let mut last_error: Option<String> = None;
        for candidate in candidates.iter().take(3) {
            let started = std::time::Instant::now();
            match candidate
                .client
                .chat_terminal_audit_bounded(
                    TERMINAL_AUDIT_SYSTEM_PROMPT,
                    &prompt,
                    TERMINAL_AUDIT_MAX_OUTPUT_TOKENS,
                )
                .await
            {
                Ok(resp) => {
                    let provider_latency_ms = started.elapsed().as_millis() as u64;
                    let usage = resp.usage.clone();
                    let usage_resp = resp.clone();
                    let usage_agent = self.agent.clone();
                    let usage_channel = self.channel.clone();
                    tokio::spawn(async move {
                        usage_agent
                            .record_llm_usage(&usage_channel, "spine_terminal_audit", &usage_resp)
                            .await;
                    });
                    tracing::info!(
                        provider_latency_ms,
                        model = %candidate.client.model_name(),
                        provider = %candidate.client.provider_name(),
                        "Spine terminal audit model call completed"
                    );
                    return Ok(SpineChatResponse {
                        provider_latency_ms,
                        text: resp.content.clone(),
                        partial_text: if resp.content.trim().is_empty() {
                            None
                        } else {
                            Some(resp.content.clone())
                        },
                        tool_calls: Vec::new(),
                        completion_tokens: usage
                            .as_ref()
                            .map(|usage| usage.completion_tokens as usize)
                            .unwrap_or_default(),
                        cache_read_tokens: usage
                            .as_ref()
                            .map(|usage| usage.cached_prompt_tokens as usize)
                            .unwrap_or_default(),
                        cache_creation_tokens: usage
                            .as_ref()
                            .map(|usage| usage.cache_creation_prompt_tokens as usize)
                            .unwrap_or_default(),
                    });
                }
                Err(error) => {
                    let error_text = error.to_string();
                    tracing::debug!(
                        model = %candidate.client.model_name(),
                        provider = %candidate.client.provider_name(),
                        error = %safe_truncate(&error_text, 320),
                        "Spine terminal audit candidate failed"
                    );
                    last_error = Some(error_text);
                }
            }
        }

        Err(SpineError::new(
            "terminal_audit_failed",
            last_error.unwrap_or_else(|| {
                "No configured model could complete the terminal audit.".to_string()
            }),
        ))
    }
}

struct PreparedSpineMessages {
    system_prompt: String,
    history: Vec<ConversationMessage>,
    user_message: String,
}

fn prepare_spine_messages_for_llm(messages: &[SpineMessage]) -> PreparedSpineMessages {
    let mut system_parts = Vec::new();
    let mut conversational = Vec::new();
    let mut tool_call_aliases: HashMap<String, String> = HashMap::new();
    for message in messages {
        match message {
            SpineMessage::System { content } => system_parts.push(content.clone()),
            SpineMessage::User { content } => conversational.push(ConversationMessage {
                role: "user".to_string(),
                content: content.clone(),
                _timestamp: chrono::Utc::now(),
            }),
            SpineMessage::Assistant {
                content,
                tool_calls,
            } => {
                let text = content.clone().unwrap_or_default();
                if !text.trim().is_empty() || tool_calls.is_empty() {
                    conversational.push(ConversationMessage {
                        role: "assistant".to_string(),
                        content: text,
                        _timestamp: chrono::Utc::now(),
                    });
                }
                for call in tool_calls {
                    if !tool_call_aliases.contains_key(&call.id) {
                        let alias = format!("tool_call_{}", tool_call_aliases.len() + 1);
                        tool_call_aliases.insert(call.id.clone(), alias);
                    }
                }
                if let Some(tool_context) =
                    spine_tool_call_history_context(tool_calls, &tool_call_aliases)
                {
                    conversational.push(ConversationMessage {
                        role: "assistant".to_string(),
                        content: tool_context,
                        _timestamp: chrono::Utc::now(),
                    });
                }
            }
            SpineMessage::Tool {
                tool_call_id,
                content,
            } => {
                let call_label = tool_call_aliases
                    .get(tool_call_id)
                    .map(String::as_str)
                    .unwrap_or("tool_call");
                let tool_result_context =
                    crate::core::model::llm_context_sanitizer::wrap_internal_tool_context(
                        &format!("tool_result:\nlabel: {}\ncontent:\n{}", call_label, content),
                    );
                conversational.push(ConversationMessage {
                    role: "user".to_string(),
                    content: tool_result_context,
                    _timestamp: chrono::Utc::now(),
                });
            }
        }
    }

    let user_message = conversational
        .pop()
        .map(|message| {
            if message.role == "user" {
                message.content
            } else {
                conversational.push(message);
                "Continue from the structured tool results and either call the next needed primitive or return the final answer.".to_string()
            }
        })
        .unwrap_or_else(|| "Continue.".to_string());

    PreparedSpineMessages {
        system_prompt: system_parts.join("\n\n"),
        history: conversational,
        user_message,
    }
}

fn bounded_json_for_spine_context(
    value: &serde_json::Value,
    max_chars: usize,
) -> serde_json::Value {
    let raw = serde_json::to_string(value).unwrap_or_default();
    if raw.chars().count() <= max_chars {
        return value.clone();
    }
    serde_json::json!({
        "truncated": true,
        "original_chars": raw.chars().count(),
        "preview": raw.chars().take(max_chars).collect::<String>(),
    })
}

#[derive(Debug, Clone, Copy)]
struct SpineRequestContextBudget {
    max_chars: usize,
}

fn json_char_count(value: &serde_json::Value) -> usize {
    serde_json::to_string(value)
        .unwrap_or_default()
        .chars()
        .count()
}

fn render_spine_request_context(context: serde_json::Map<String, serde_json::Value>) -> String {
    format!(
        "Structured chat request context:\n{}",
        serde_json::to_string(&serde_json::Value::Object(context))
            .unwrap_or_else(|_| "{}".to_string())
    )
}

fn shrink_spine_request_context_to_budget(
    mut context: serde_json::Map<String, serde_json::Value>,
    budget: SpineRequestContextBudget,
) -> serde_json::Map<String, serde_json::Value> {
    let originals = context.clone();
    let mut preview_budgets = originals
        .iter()
        .map(|(key, value)| (key.clone(), json_char_count(value)))
        .collect::<HashMap<_, _>>();

    loop {
        let current = render_spine_request_context(context.clone())
            .chars()
            .count();
        if current <= budget.max_chars {
            return context;
        }

        let Some((key, original_value, current_preview_budget)) = originals
            .iter()
            .filter_map(|(key, value)| {
                let current_preview_budget = *preview_budgets.get(key)?;
                (current_preview_budget > SPINE_REQUEST_CONTEXT_MIN_PREVIEW_CHARS).then_some((
                    key.clone(),
                    value.clone(),
                    current_preview_budget,
                ))
            })
            .max_by_key(|(_, _, current_preview_budget)| *current_preview_budget)
        else {
            return context;
        };

        let overflow = current.saturating_sub(budget.max_chars);
        let next_preview_budget = current_preview_budget
            .saturating_sub(overflow.saturating_add(256))
            .min(current_preview_budget.saturating_mul(2) / 3)
            .max(SPINE_REQUEST_CONTEXT_MIN_PREVIEW_CHARS);
        if next_preview_budget >= current_preview_budget {
            return context;
        }
        preview_budgets.insert(key.clone(), next_preview_budget);
        context.insert(
            key,
            bounded_json_for_spine_context(&original_value, next_preview_budget),
        );
    }
}

fn structured_chat_request_context_system_message(
    request_hints: &RequestExecutionHints,
    budget: Option<SpineRequestContextBudget>,
) -> Option<String> {
    let mut context = serde_json::Map::new();
    if request_hints.attachments_present || !request_hints.attachments.is_empty() {
        context.insert(
            "attachments_present".to_string(),
            serde_json::json!(
                request_hints.attachments_present || !request_hints.attachments.is_empty()
            ),
        );
        let attachments = serde_json::to_value(&request_hints.attachments)
            .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
        context.insert("attachments".to_string(), attachments);
    }
    if let Some(execution_profile) = request_hints.execution_profile.as_ref() {
        context.insert("execution_profile".to_string(), execution_profile.clone());
        let depth_hint = execution_profile
            .get("depth_hint")
            .or_else(|| execution_profile.get("depth"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if execution_profile_requests_long_running(Some(execution_profile))
            || depth_hint.is_some_and(|value| value.eq_ignore_ascii_case("deep"))
        {
            context.insert(
                "quality_contract".to_string(),
                serde_json::json!({
                    "research": "For research-heavy work, gather enough source-backed evidence before synthesis. Use the search primitive with depth=\"deep\" when the visible answer requires broad, comparative, current, primary-source, or decision-grade support. Create PDF/report artifacts through explicit primitives when the user-visible deliverable requires them."
                }),
            );
        }
    }
    if let Some(arkorbit_context) = request_hints.arkorbit_context.as_ref() {
        context.insert("arkorbit_context".to_string(), arkorbit_context.clone());
    }
    if let Some(browser_profile_context) = request_hints.browser_profile_context.as_ref() {
        context.insert(
            "browser_profile_context".to_string(),
            browser_profile_context.clone(),
        );
    }
    if request_hints.client_timezone.is_some()
        || request_hints.client_timezone_offset_minutes.is_some()
    {
        context.insert(
            "client_temporal_context".to_string(),
            serde_json::json!({
                "timezone": request_hints.client_timezone.clone(),
                "timezone_offset_minutes": request_hints.client_timezone_offset_minutes,
                "contract": "Use this request timezone for user wall-clock dates and times when the saved profile timezone is absent. Use cron for recurring scheduled_task cadences; use at/scheduled_for for fully known one-time timestamps; use local_time plus timezone for wall-clock times that must be resolved from runtime temporal context. Do not infer that local_time replaces cron for recurrence."
            }),
        );
    }
    if !request_hints.recent_actionable_artifacts.is_empty() {
        context.insert(
            "recent_actionable_artifacts".to_string(),
            serde_json::Value::Array(request_hints.recent_actionable_artifacts.clone()),
        );
    }
    if context.is_empty() {
        return None;
    }

    if let Some(budget) = budget.filter(|budget| budget.max_chars > 0) {
        let compact = render_spine_request_context(context.clone());
        if compact.chars().count() <= budget.max_chars {
            return Some(compact);
        }
        context = shrink_spine_request_context_to_budget(context, budget);
    }

    Some(render_spine_request_context(context))
}

fn execution_profile_requests_long_running(profile: Option<&serde_json::Value>) -> bool {
    profile
        .and_then(|value| value.get("long_running"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn spine_request_context_budget_for_llm(
    llm: &crate::core::model::llm::LlmClient,
) -> SpineRequestContextBudget {
    let context_tokens = crate::core::model::context_budget::context_window_tokens_for_llm(
        llm,
        crate::core::model::context_budget::HistoryBudgetConfig {
            scope_env: "SPINE",
            default_context_window_tokens: SPINE_REQUEST_CONTEXT_DEFAULT_CONTEXT_WINDOW_TOKENS,
            default_budget_ratio_percent: SPINE_REQUEST_CONTEXT_BUDGET_RATIO_PERCENT,
            min_history_token_budget: 1_024,
            max_summary_tokens: 8_000,
        },
    );
    let reserved_output_tokens =
        crate::core::orchestration::execution::supervised_request_output_budget(
            "model_routed_spine_v1",
        )
        .unwrap_or_default() as usize;
    let ratio_percent = crate::core::model::context_budget::read_usize_env(
        "AGENTARK_SPINE_REQUEST_CONTEXT_BUDGET_RATIO_PERCENT",
    )
    .unwrap_or(SPINE_REQUEST_CONTEXT_BUDGET_RATIO_PERCENT)
    .clamp(3, 40);
    let prompt_budget_tokens = context_tokens.saturating_sub(reserved_output_tokens);
    SpineRequestContextBudget {
        max_chars: prompt_budget_tokens
            .saturating_mul(ratio_percent)
            .saturating_mul(4)
            / 100,
    }
}

async fn spine_request_context_compaction_disabled(storage: &crate::storage::Storage) -> bool {
    storage
        .get(crate::core::self_evolve::SPINE_REQUEST_CONTEXT_COMPACTION_DISABLED_KEY)
        .await
        .ok()
        .flatten()
        .is_some()
}

fn action_directory_source_rank(source: &crate::actions::ActionSource) -> usize {
    match source {
        crate::actions::ActionSource::Custom => 4,
        crate::actions::ActionSource::Bundled => 3,
        crate::actions::ActionSource::System => 1,
    }
}

fn action_directory_access_rank(action: &crate::actions::ActionDef) -> usize {
    let access = &action.authorization.access;
    let mut score = 0usize;
    if !access.integration_ids.is_empty() {
        score += 3;
    }
    if !access.extension_pack_ids.is_empty() {
        score += 3;
    }
    if !access.integration_features.is_empty() {
        score += 2;
    }
    if !access.channel_targets.is_empty() {
        score += 2;
    }
    if action.authorization.requires_auth {
        score += 1;
    }
    if action.authorization.outbound.outbound_write || action.authorization.outbound.read_only {
        score += 1;
    }
    score
}

fn action_directory_capability_rank(action: &crate::actions::ActionDef) -> usize {
    let mut score = 0usize;
    for capability in &action.capabilities {
        let capability = capability.trim();
        if capability.is_empty() {
            continue;
        }
        score += 1;
        if matches!(
            capability,
            "custom_api"
                | "integration"
                | "integration_builder"
                | "external_write"
                | "network"
                | "webhook"
                | "messaging"
        ) {
            score += 2;
        }
    }
    score
}

fn action_directory_rank(action: &crate::actions::ActionDef) -> (usize, usize, usize, String) {
    (
        action_directory_source_rank(&action.source),
        action_directory_access_rank(action),
        action_directory_capability_rank(action),
        action.name.clone(),
    )
}

fn action_directory_candidate(action: &crate::actions::ActionDef) -> bool {
    let name = action.name.trim();
    if name.is_empty() || PRIMITIVE_NAMES.contains(&name) {
        return false;
    }
    true
}

fn build_action_directory_entry(action: &crate::actions::ActionDef) -> serde_json::Value {
    let schema_fields =
        crate::core::orchestration::action_catalog::action_schema_field_descriptions(
            &action.input_schema,
            ACTION_DIRECTORY_SCHEMA_FIELD_LIMIT,
        )
        .into_iter()
        .map(|field| safe_truncate(&field, ACTION_DIRECTORY_FIELD_MAX_CHARS))
        .collect::<Vec<_>>();
    let required_shapes =
        crate::core::orchestration::action_catalog::action_schema_required_shape_descriptions(
            &action.input_schema,
            4,
        )
        .into_iter()
        .map(|shape| safe_truncate(&shape, ACTION_DIRECTORY_FIELD_MAX_CHARS))
        .collect::<Vec<_>>();
    serde_json::json!({
        "action_name": action.name,
        "source": match action.source {
            crate::actions::ActionSource::System => "system",
            crate::actions::ActionSource::Bundled => "bundled",
            crate::actions::ActionSource::Custom => "custom",
        },
        "description": safe_truncate(action.description.trim(), ACTION_DIRECTORY_FIELD_MAX_CHARS),
        "capabilities": action.capabilities,
        "requires_auth": action.authorization.requires_auth,
        "outbound": {
            "read_only": action.authorization.outbound.read_only,
            "outbound_write": action.authorization.outbound.outbound_write,
            "public_publish": action.authorization.outbound.public_publish,
        },
        "schema_fields": schema_fields,
        "required_shapes": required_shapes,
    })
}

fn build_action_directory_context_message(
    actions: &[crate::actions::ActionDef],
    _user_request: &str,
    max_entries: usize,
) -> Option<String> {
    if max_entries == 0 {
        return None;
    }
    let mut actions = actions
        .iter()
        .filter(|action| action_directory_candidate(action))
        .collect::<Vec<_>>();
    actions.sort_by(|left, right| {
        action_directory_rank(right)
            .cmp(&action_directory_rank(left))
            .then_with(|| left.name.cmp(&right.name))
    });
    let compact_action_names = actions
        .iter()
        .take(ACTION_DIRECTORY_CONTEXT_MAX_NAMES)
        .map(|action| action.name.clone())
        .collect::<Vec<_>>();
    let action_count = actions.len();
    let entries = actions
        .into_iter()
        .take(max_entries)
        .map(build_action_directory_entry)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    let directory = serde_json::json!({
        "available_action_count": action_count,
        "compact_action_names": compact_action_names,
        "action_cards": entries,
    });
    Some(format!(
        "Runtime action directory:\n{}\nUse action_call only with an exact action_name from this directory or prior tool evidence, and pass the selected action's declared arguments object. Prefer higher-level spine primitives when they cover the requested capability.",
        serde_json::to_string_pretty(&directory).unwrap_or_else(|_| "{}".to_string())
    ))
}

async fn runtime_action_directory_context_system_message(
    agent: &Agent,
    user_request: &str,
) -> Option<String> {
    let actions = match agent.load_action_catalog_actions().await {
        Ok(actions) => actions,
        Err(error) => {
            tracing::debug!(error = %error, "failed to load runtime action directory for spine context");
            return None;
        }
    };
    build_action_directory_context_message(
        &actions,
        user_request,
        ACTION_DIRECTORY_CONTEXT_MAX_ENTRIES,
    )
}

async fn browser_profiles_context_system_message(
    storage: &crate::storage::Storage,
) -> Option<String> {
    let response = crate::core::BrowserProfileControlPlane::list(storage)
        .await
        .ok()?;
    let profiles = response
        .profiles
        .into_iter()
        .filter(|profile| profile.enabled)
        .take(12)
        .map(|profile| {
            serde_json::json!({
                "id": profile.id,
                "name": profile.name,
                "description": profile.description,
                "tags": profile.tags,
                "target_kind": profile.target_kind,
                "target_endpoint": profile.target_endpoint,
                "target_workspace": profile.target_workspace,
                "login_state": profile.login_state,
                "last_used_at": profile.last_used_at,
                "recent_session_count": profile.recent_sessions.len(),
            })
        })
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        return None;
    }
    Some(format!(
        "Saved browser login profiles available for browser tasks. When a browser task should reuse a saved login or browser identity, set the browse.profile field to the best matching id, name, target, or semantic selector from this data. Do not infer a profile when the task does not need one.\n{}",
        serde_json::to_string_pretty(&serde_json::Value::Array(profiles))
            .unwrap_or_else(|_| "[]".to_string())
    ))
}

fn browser_session_status_is_live_context(status: &str) -> bool {
    matches!(
        status,
        "active" | "waiting_for_operator" | "operator_claimed" | "ready"
    )
}

async fn live_browser_sessions_context_system_message(
    browser_sessions: &crate::core::connectivity::browser_session::BrowserSessionManager,
    conversation_id: Option<&str>,
) -> Option<String> {
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let listed = browser_sessions.list_session_views().await;
    let mut sessions = Vec::new();
    for session in listed
        .into_iter()
        .filter(|session| {
            session.conversation_id.as_deref() == Some(conversation_id)
                && browser_session_status_is_live_context(session.status.as_str())
        })
        .take(3)
    {
        let view = browser_sessions
            .describe_session(&session.id)
            .await
            .unwrap_or(session);
        sessions.push(serde_json::json!({
            "id": view.id,
            "status": view.status,
            "task_description": view.task_description,
            "question": view.question,
            "summary": view.summary,
            "page_url": view.page_url,
            "page_title": view.page_title,
            "can_claim": view.can_claim,
            "can_release": view.can_release,
            "can_complete": view.can_complete,
        }));
    }
    if sessions.is_empty() {
        return None;
    }
    Some(format!(
        "Live browser session context for this chat. If the user refers to the current page, screen, browser, handoff, checkpoint, or a pending browser question, use the browse primitive action that matches the meaning: action=snapshot to read the current page, action=resume_handoff to pass the user's reply or handoff note back to the waiting browser loop, and action=start_session only for new or continued browser automation. Do not state current page content without snapshot evidence.\n{}",
        serde_json::to_string_pretty(&serde_json::Value::Array(sessions))
            .unwrap_or_else(|_| "[]".to_string())
    ))
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_model_routed_spine_for_chat(
        &self,
        channel: &str,
        message: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        request_hints: &RequestExecutionHints,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> anyhow::Result<ProcessedMessage> {
        let mut spine_messages = self
            .load_spine_history_for_chat(
                conversation_id,
                request_hints.recorded_user_message_id.as_deref(),
            )
            .await;
        if let Some(saved_user_memory) = self
            .build_saved_user_facts_context(project_id, conversation_id, message)
            .await
            .map(|context| context.trim().to_string())
            .filter(|context| !context.is_empty())
        {
            spine_messages.insert(
                0,
                SpineMessage::System {
                    content: saved_user_memory,
                },
            );
        }
        if let Some(browser_profile_context) =
            browser_profiles_context_system_message(&self.storage).await
        {
            spine_messages.push(SpineMessage::System {
                content: browser_profile_context,
            });
        }
        if let Some(live_browser_context) =
            live_browser_sessions_context_system_message(&self.browser_sessions, conversation_id)
                .await
        {
            spine_messages.push(SpineMessage::System {
                content: live_browser_context,
            });
        }
        let request_context_budget =
            if spine_request_context_compaction_disabled(&self.storage).await {
                None
            } else {
                Some(
                    self.model_pool
                        .get(&self.primary_model_id)
                        .map(|(_, llm)| spine_request_context_budget_for_llm(llm))
                        .unwrap_or_else(|| spine_request_context_budget_for_llm(&self.llm)),
                )
            };
        if let Some(request_context) =
            structured_chat_request_context_system_message(request_hints, request_context_budget)
        {
            spine_messages.push(SpineMessage::System {
                content: request_context,
            });
        }
        if let Some(action_directory_context) =
            runtime_action_directory_context_system_message(self, message).await
        {
            spine_messages.push(SpineMessage::System {
                content: action_directory_context,
            });
        }
        let capability_readiness_generation = self.capability_readiness_generation().await;
        if let Some(readiness_context) = self.capability_readiness_context_message().await {
            spine_messages.push(SpineMessage::System {
                content: readiness_context,
            });
        }
        spine_messages.push(SpineMessage::User {
            content: message.to_string(),
        });

        let mut request = SpineRequest::new(CallerKind::Chat, spine_messages, channel);
        request.conversation_id = conversation_id.map(str::to_string);
        request.project_id = project_id.map(str::to_string);
        request.execution_profile = request_hints.execution_profile.clone();
        request.long_running =
            execution_profile_requests_long_running(request.execution_profile.as_ref());
        request.browser_profile_context = request_hints.browser_profile_context.clone();
        request.capability_readiness_generation = Some(capability_readiness_generation);
        request.visual_attachments = request_hints
            .attachments
            .iter()
            .filter(|hint| chat_attachment_hint_is_visual_image(hint))
            .cloned()
            .collect();
        request.streaming = stream_tx.is_some();
        request.authorization = ActionAuthorizationContext {
            principal: request_hints.caller_principal.clone(),
            surface: request_hints.execution_surface.clone(),
            direct_user_intent: request_hints.direct_user_intent,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: Some(
                conversation_id
                    .map(str::to_string)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ),
            request_timezone: request_hints.client_timezone.clone(),
            request_timezone_offset_minutes: request_hints.client_timezone_offset_minutes,
        };
        if matches!(
            request.authorization.surface,
            ActionExecutionSurface::Internal
        ) {
            request.authorization.surface = ActionExecutionSurface::Chat;
        }

        let trace = Arc::new(SpineTraceRecorder::default());
        let request = Arc::new(request);
        let cx = SpineContext::new(
            self.clone(),
            request.clone(),
            trace.clone(),
            stream_tx.clone(),
        );
        let server = AgentSpineLlmServer::new(
            self.clone(),
            channel,
            stream_tx,
            trace.clone(),
            request.caller_kind,
            request.long_running,
            request.conversation_id.clone(),
        );
        let tools = ToolRegistry::new();
        let result = run_spine((*request).clone(), &server, &tools, &cx).await;
        let trace_events = trace.snapshot().await;
        let trace_steps = spine_trace_steps(&trace_events);
        let turn_records = spine_turn_records(&result);
        let (cached_prompt_tokens, cache_creation_prompt_tokens) =
            spine_cache_usage_from_events(&trace_events);

        match result {
            SpineResult::Completed {
                final_text,
                messages,
                turns_used: _,
            } => Ok(ProcessedMessage {
                response: spine_narrated_final_text(request.messages.len(), &messages, &final_text),
                conversation_id: conversation_id.map(str::to_string),
                conversation_title: None,
                run_id: None,
                run_status: Some(
                    crate::core::ExecutionRunStatus::Completed
                        .as_str()
                        .to_string(),
                ),
                trace_id: None,
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
                choices: Vec::new(),
                degradation: Vec::new(),
                attempted_models: Vec::new(),
                user_outcome: None,
                trace_steps,
                turn_records,
                turn_plan: None,
            }),
            SpineResult::Blocked {
                final_text,
                ref messages,
                ..
            } => Ok(spine_blocked_processed_message(
                conversation_id,
                &spine_narrated_final_text(request.messages.len(), messages, &final_text),
                crate::core::ExecutionRunStatus::Blocked.as_str(),
                trace_steps,
                turn_records,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
            )),
            SpineResult::NeedsInput {
                final_text,
                ref messages,
                ..
            } => Ok(spine_blocked_processed_message(
                conversation_id,
                &spine_narrated_final_text(request.messages.len(), messages, &final_text),
                crate::core::ExecutionRunStatus::NeedsInput.as_str(),
                trace_steps,
                turn_records,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
            )),
            SpineResult::PausedForApproval { .. } => {
                let approval = cx.paused_approval().await.unwrap_or_else(|| {
                    tool_result_error(
                        "spine",
                        "approval_required",
                        "The requested action requires approval.",
                    )
                });
                let choices = choices_from_spine_approval(&approval);
                Ok(ProcessedMessage {
                    // Approval reasons are policy-authored, but route them through the
                    // same terminal funnel as every other emitter so the invariant
                    // holds uniformly.
                    response: finalize_user_text(
                        SpineTerminalTextKind::Blocked,
                        &approval_required_response(&approval),
                        TerminalTextProvenance::SystemAuthored,
                    ),
                    conversation_id: conversation_id.map(str::to_string),
                    conversation_title: None,
                    run_id: None,
                    run_status: Some(
                        crate::core::ExecutionRunStatus::Blocked
                            .as_str()
                            .to_string(),
                    ),
                    trace_id: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    cached_prompt_tokens,
                    cache_creation_prompt_tokens,
                    choices,
                    degradation: Vec::new(),
                    attempted_models: Vec::new(),
                    user_outcome: None,
                    trace_steps,
                    turn_records,
                    turn_plan: None,
                })
            }
            SpineResult::MaxTurnsExceeded { .. } => Ok(spine_blocked_processed_message(
                conversation_id,
                "The spine reached its turn budget before producing a final answer.",
                crate::core::ExecutionRunStatus::PlatformFailed.as_str(),
                trace_steps,
                turn_records,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
            )),
            SpineResult::Cancelled { reason, .. } => Ok(spine_blocked_processed_message(
                conversation_id,
                &format!("The request was cancelled: {}.", reason),
                crate::core::ExecutionRunStatus::Cancelled.as_str(),
                trace_steps,
                turn_records,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
            )),
            SpineResult::PlatformFailed { error, .. } => Ok(spine_blocked_processed_message(
                conversation_id,
                &user_visible_platform_failure_message(&error.message),
                crate::core::ExecutionRunStatus::PlatformFailed.as_str(),
                trace_steps,
                turn_records,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
            )),
        }
    }

    async fn load_spine_history_for_chat(
        &self,
        conversation_id: Option<&str>,
        excluded_message_id: Option<&str>,
    ) -> Vec<SpineMessage> {
        let Some(conversation_id) = conversation_id.filter(|value| !value.trim().is_empty()) else {
            return Vec::new();
        };
        match self
            .encrypted_storage
            .get_recent_messages_decrypted(conversation_id, 12)
            .await
        {
            Ok(messages) => messages
                .into_iter()
                .filter(|message| excluded_message_id != Some(message.id.as_str()))
                .map(storage_message_to_spine_message)
                .collect(),
            Err(error) => {
                tracing::warn!(
                    conversation_id,
                    error = %error,
                    "failed to load spine chat history"
                );
                Vec::new()
            }
        }
    }

    pub(super) async fn run_model_routed_spine_for_task(
        &self,
        task: &super::task::Task,
    ) -> anyhow::Result<String> {
        let channel = task
            .arguments
            .get("channel")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("task");
        let conversation_id = task
            .arguments
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let project_id = task
            .arguments
            .get("project_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let mut spine_messages = vec![SpineMessage::User {
            content: serde_json::json!({
                "caller": "task",
                "task": {
                    "id": task.id.to_string(),
                    "description": task.description.clone(),
                    "action": task.action.clone(),
                    "arguments": task.arguments.clone(),
                    "capabilities": task.capabilities.clone(),
                    "scheduled_for": task.scheduled_for.as_ref().map(|value| value.to_rfc3339()),
                    "cron": task.cron.clone(),
                    "priority": task.priority,
                    "urgency": task.urgency,
                    "importance": task.importance,
                    "eisenhower_quadrant": task.eisenhower_quadrant,
                },
                "instruction": "Complete this automation task through the model-routed spine. Use primitive tools when execution is needed; return a concise final result when complete."
            })
            .to_string(),
        }];
        if let Some(conversation_id) = conversation_id {
            let mut history = self
                .load_spine_history_for_chat(Some(conversation_id), None)
                .await;
            history.append(&mut spine_messages);
            spine_messages = history;
        }
        if let Some(action_directory_context) =
            runtime_action_directory_context_system_message(self, &task.description).await
        {
            spine_messages.insert(
                0,
                SpineMessage::System {
                    content: action_directory_context,
                },
            );
        }

        let mut request = SpineRequest::new(CallerKind::Task, spine_messages, channel);
        request.conversation_id = conversation_id.map(str::to_string);
        request.project_id = project_id.map(str::to_string);
        request.authorization = automation_runtime_authorization_context(
            &task.arguments,
            ActionExecutionSurface::Automation,
        );
        request.long_running = task
            .arguments
            .get("long_running")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let trace = Arc::new(SpineTraceRecorder::default());
        let request = Arc::new(request);
        let cx = SpineContext::new(self.clone(), request.clone(), trace.clone(), None);
        let server = AgentSpineLlmServer::new(
            self.clone(),
            channel,
            None,
            trace.clone(),
            request.caller_kind,
            request.long_running,
            request.conversation_id.clone(),
        );
        let tools = ToolRegistry::new();
        match run_spine((*request).clone(), &server, &tools, &cx).await {
            SpineResult::Completed { final_text, .. } => Ok(final_text),
            SpineResult::Blocked {
                final_text,
                turns_used,
                ..
            } => Ok(serde_json::json!({
                "status": "blocked",
                "turns_used": turns_used,
                "message": final_text,
            })
            .to_string()),
            SpineResult::NeedsInput {
                final_text,
                turns_used,
                ..
            } => Ok(serde_json::json!({
                "status": "needs_input",
                "turns_used": turns_used,
                "message": final_text,
            })
            .to_string()),
            SpineResult::PausedForApproval { pending_call, .. } => {
                let pending = serde_json::json!({
                    "status": "paused_for_approval",
                    "primitive": pending_call.name,
                    "tool_call_id": pending_call.id,
                    "approval": cx.paused_approval().await,
                });
                Ok(pending.to_string())
            }
            SpineResult::MaxTurnsExceeded { turns_used, .. } => Ok(serde_json::json!({
                "status": "max_turns_exceeded",
                "turns_used": turns_used,
            })
            .to_string()),
            SpineResult::Cancelled { reason, .. } => Ok(serde_json::json!({
                "status": "cancelled",
                "reason": reason,
            })
            .to_string()),
            SpineResult::PlatformFailed { error, .. } => {
                Err(anyhow::anyhow!("spine platform failure: {}", error.message))
            }
        }
    }

    pub(crate) async fn run_model_routed_spine_for_watcher_poll(
        &self,
        watcher: &super::watcher::Watcher,
    ) -> anyhow::Result<String> {
        let origin = automation_origin_from_arguments(&watcher.poll_arguments);
        let channel = origin
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("watcher");
        let durable_observation_contract =
            crate::core::automation::durable_observation_result_contract();
        let spine_messages = vec![SpineMessage::User {
            content: serde_json::json!({
                "caller": "watcher",
                "watcher": {
                    "id": watcher.id.to_string(),
                    "description": watcher.description.clone(),
                    "poll_action": watcher.poll_action.clone(),
                    "poll_arguments": watcher.poll_arguments.clone(),
                    "condition": watcher.condition.clone(),
                    "on_trigger": watcher.on_trigger.clone(),
                    "notify_channel": watcher.notify_channel.clone(),
                    "poll_count": watcher.poll_count,
                    "last_result": watcher.last_result.clone(),
                },
                "instruction": format!(
                    "Run exactly one watcher poll through the model-routed spine. Return the poll result as final text; do not evaluate the watcher condition in the final answer. If poll_arguments.semantic_watcher_poll is true, treat poll_arguments.intent and poll_arguments.source_payload as the polling contract: choose suitable read-only tools from the semantic target and return final text as one JSON object using this durable-observation contract: {}. Set durable_observation_result=true and ready_for_change_detection=true only when every target has a stable source, comparable=true, non-empty observed_values, and empty blocking_gaps. If comparable observations are missing, return the same JSON shape with ready_for_change_detection=false and blocking_gaps explaining what prevented a commit-ready observation. Do not notify the user or perform the trigger action during the poll.",
                    durable_observation_contract
                )
            })
            .to_string(),
        }];
        let mut request = SpineRequest::new(CallerKind::Watcher, spine_messages, channel);
        request.conversation_id = origin.conversation_id;
        request.project_id = origin.project_id;
        request.authorization = automation_runtime_authorization_context(
            &watcher.poll_arguments,
            ActionExecutionSurface::Background,
        );
        request.long_running = watcher
            .poll_arguments
            .get("long_running")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let trace = Arc::new(SpineTraceRecorder::default());
        let request = Arc::new(request);
        let cx = SpineContext::new(self.clone(), request.clone(), trace.clone(), None);
        let server = AgentSpineLlmServer::new(
            self.clone(),
            channel,
            None,
            trace.clone(),
            request.caller_kind,
            request.long_running,
            request.conversation_id.clone(),
        );
        let tools = ToolRegistry::new();
        match run_spine((*request).clone(), &server, &tools, &cx).await {
            SpineResult::Completed { final_text, .. } => Ok(final_text),
            SpineResult::Blocked { final_text, .. } => Err(anyhow::anyhow!(
                "watcher spine poll blocked: {}",
                final_text
            )),
            SpineResult::NeedsInput { final_text, .. } => Err(anyhow::anyhow!(
                "watcher spine poll needs input: {}",
                final_text
            )),
            SpineResult::MaxTurnsExceeded { turns_used, .. } => Err(anyhow::anyhow!(
                "watcher spine poll exceeded turn budget after {} turns",
                turns_used
            )),
            SpineResult::Cancelled { reason, .. } => {
                Err(anyhow::anyhow!("watcher spine poll cancelled: {}", reason))
            }
            SpineResult::PausedForApproval { .. } => {
                Err(anyhow::anyhow!("watcher spine poll requires approval"))
            }
            SpineResult::PlatformFailed { error, .. } => Err(anyhow::anyhow!(
                "watcher spine poll platform failure: {}",
                error.message
            )),
        }
    }

    pub(super) async fn invalidate_spine_tool_directory(&self, reason: &'static str) {
        tracing::debug!(reason, "spine tool directory invalidated");
    }

    pub(crate) fn forced_swarm_specs(
        &self,
        task: &str,
        actions: &[ActionDef],
    ) -> Vec<crate::core::automation::task_router::SubAgentSpec> {
        let objective = safe_truncate(task.trim(), 1_200);
        let available_action_summary = actions
            .iter()
            .filter(|action| !action.name.trim().is_empty())
            .take(24)
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let context = if available_action_summary.is_empty() {
            objective.clone()
        } else {
            format!(
                "{}\n\nAvailable substrate actions include: {}",
                objective, available_action_summary
            )
        };
        vec![
            crate::core::automation::task_router::SubAgentSpec {
                agent_type: "researcher".to_string(),
                task: format!(
                    "Gather the evidence and constraints needed to complete this delegated objective:\n{}",
                    context
                ),
                preferred_model_role: Some("research".to_string()),
                depends_on: Vec::new(),
                plan_step_id: None,
            },
            crate::core::automation::task_router::SubAgentSpec {
                agent_type: "coder".to_string(),
                task: format!(
                    "Implement or execute the concrete work needed for this delegated objective:\n{}",
                    context
                ),
                preferred_model_role: Some("code".to_string()),
                depends_on: Vec::new(),
                plan_step_id: None,
            },
            crate::core::automation::task_router::SubAgentSpec {
                agent_type: "validator".to_string(),
                task: format!(
                    "Review the delegated work for correctness, gaps, and final user-facing outcome:\n{}",
                    objective
                ),
                preferred_model_role: Some("analysis".to_string()),
                depends_on: vec![0, 1],
                plan_step_id: None,
            },
        ]
    }
}

fn storage_message_to_spine_message(
    message: crate::storage::entities::message::Model,
) -> SpineMessage {
    match message.role.as_str() {
        "system" => SpineMessage::System {
            content: message.content,
        },
        "assistant" => SpineMessage::Assistant {
            content: {
                let visible_content =
                    crate::core::model::llm_context_sanitizer::strip_internal_tool_transcript(
                        &message.content,
                    );
                (!visible_content.trim().is_empty()).then_some(visible_content)
            },
            tool_calls: message
                .tool_calls_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Vec<SpineToolCall>>(raw).ok())
                .unwrap_or_default(),
        },
        "tool" => SpineMessage::Tool {
            tool_call_id: message
                .tool_call_id
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(message.id),
            content: message.content,
        },
        _ => SpineMessage::User {
            content: message.content,
        },
    }
}

/// Compose the persisted reply from the terminal answer only.
///
/// Progress narration is streamed live while the run is active. Persisting it
/// into the final assistant message makes an incomplete run look like a valid
/// answer and causes the user to see all intermediate "what I am doing next"
/// prose dumped at the end. The saved chat message is therefore the terminal
/// answer/status, not the live progress transcript.
fn spine_narrated_final_text(
    _prior_message_count: usize,
    _messages: &[SpineMessage],
    final_text: &str,
) -> String {
    final_text.to_string()
}

fn spine_blocked_processed_message(
    conversation_id: Option<&str>,
    response: &str,
    status: &str,
    trace_steps: Vec<ExecutionStep>,
    turn_records: Vec<AgentTurnRecord>,
    cached_prompt_tokens: i64,
    cache_creation_prompt_tokens: i64,
) -> ProcessedMessage {
    let kind = match status {
        "completed" => SpineTerminalTextKind::Completed,
        "needs_input" => SpineTerminalTextKind::NeedsInput,
        _ => SpineTerminalTextKind::Blocked,
    };
    ProcessedMessage {
        response: finalize_user_text(kind, response, TerminalTextProvenance::SystemAuthored),
        conversation_id: conversation_id.map(str::to_string),
        conversation_title: None,
        run_id: None,
        run_status: Some(status.to_string()),
        trace_id: None,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cached_prompt_tokens,
        cache_creation_prompt_tokens,
        choices: Vec::new(),
        degradation: Vec::new(),
        attempted_models: Vec::new(),
        user_outcome: None,
        trace_steps,
        turn_records,
        turn_plan: None,
    }
}

fn choices_from_spine_approval(value: &serde_json::Value) -> Vec<ClarificationChoice> {
    value
        .get("inline_choices")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|choice| serde_json::from_value::<ClarificationChoice>(choice.clone()).ok())
        .collect()
}

fn approval_required_response(value: &serde_json::Value) -> String {
    let reason = value
        .get("reason")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("This action requires approval before it can run.");
    format!("Approval required: {}", reason)
}

fn spine_caller_mode_label(caller_kind: CallerKind) -> &'static str {
    match caller_kind {
        CallerKind::Chat => "chat",
        CallerKind::Task => "task",
        CallerKind::Watcher => "watcher",
        CallerKind::Cron => "cron",
        CallerKind::Gateway => "gateway",
        CallerKind::Companion => "companion",
    }
}

fn build_spine_prompt_telemetry(
    caller_kind: CallerKind,
    prepared: &PreparedSpineMessages,
    prompt_bundle: &crate::core::self_evolve::PromptBundleProfile,
    prompt_fragment_bundle: &crate::core::model::prompt_fragments::PromptFragmentBundleProfile,
    tool_schemas: &[ActionDef],
) -> serde_json::Value {
    let spine_prompt_bundle = spine_prompt_bundle::build_spine_prompt_bundle(
        &prepared.system_prompt,
        Some(prompt_bundle),
        Some(prompt_fragment_bundle),
        &PRIMITIVE_NAMES,
    );
    let visible_system_prompt = spine_prompt_bundle.render_visible();
    let base_spine_prompt_bundle =
        spine_prompt_bundle::build_spine_prompt_bundle("", None, None, &PRIMITIVE_NAMES);
    let tool_schema_json = serde_json::to_string(tool_schemas).unwrap_or_default();
    let tool_schema_chars = tool_schema_json.chars().count();
    let history_chars = prepared
        .history
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    let user_message_chars = prepared.user_message.chars().count();
    let extra_system_chars = prepared.system_prompt.chars().count();
    let base_prompt_chars = base_spine_prompt_bundle.render_visible().chars().count();
    let primary_response_prompt =
        crate::core::self_evolve::prompt_evolution::render_primary_response_system_prompt(
            prompt_bundle,
        );
    let prompt_bundle_chars = primary_response_prompt.chars().count();
    let final_system_prompt_chars = visible_system_prompt.chars().count();
    let estimated_total_request_chars = final_system_prompt_chars
        .saturating_add(history_chars)
        .saturating_add(user_message_chars)
        .saturating_add(tool_schema_chars);
    let system_prompt_tokens = estimate_prompt_tokens(&[SpineMessage::System {
        content: visible_system_prompt,
    }]);
    let history_messages = prepared
        .history
        .iter()
        .map(|message| {
            if message.role == "assistant" {
                SpineMessage::Assistant {
                    content: Some(message.content.clone()),
                    tool_calls: Vec::new(),
                }
            } else {
                SpineMessage::User {
                    content: message.content.clone(),
                }
            }
        })
        .collect::<Vec<_>>();
    let history_prompt_tokens = estimate_prompt_tokens(&history_messages);
    let user_prompt_tokens = estimate_prompt_tokens(&[SpineMessage::User {
        content: prepared.user_message.clone(),
    }]);
    let tool_schema_tokens = tool_schema_chars.div_ceil(4);
    let estimated_total_request_tokens = system_prompt_tokens
        .saturating_add(history_prompt_tokens)
        .saturating_add(user_prompt_tokens)
        .saturating_add(tool_schema_tokens);
    let prompt_version =
        crate::core::self_evolve::prompt_evolution::compose_prompt_version(&prompt_bundle.version);
    let prompt_fragment_version =
        crate::core::model::prompt_fragments::compose_prompt_fragment_version(
            &prompt_fragment_bundle.version,
        );
    let mut sections = serde_json::Map::new();
    for (section, chars) in spine_prompt_bundle.section_char_counts() {
        sections.insert(section, serde_json::json!(chars));
    }
    sections.insert(
        "runtime_access_summary".to_string(),
        serde_json::json!(base_prompt_chars),
    );
    if extra_system_chars > 0 {
        sections.insert(
            "request_context".to_string(),
            serde_json::json!(extra_system_chars),
        );
    }
    if prompt_bundle_chars > 0 {
        sections.insert(
            "prompt_bundle_primary_response".to_string(),
            serde_json::json!(prompt_bundle_chars),
        );
    }
    if tool_schema_chars > 0 {
        sections.insert(
            "action_catalog".to_string(),
            serde_json::json!(tool_schema_chars),
        );
    }

    serde_json::json!({
        "trace_kind": "prompt_telemetry",
        "request_mode": spine_caller_mode_label(caller_kind),
        "system_prompt_version": "model_routed_spine_v1",
        "spine_prompt_bundle_version": SPINE_PROMPT_BUNDLE_VERSION,
        "prompt_version": prompt_version,
        "prompt_fragment_version": prompt_fragment_version,
        "final_system_prompt_chars": final_system_prompt_chars,
        "tool_schema_chars": tool_schema_chars,
        "estimated_total_request_chars": estimated_total_request_chars,
        "final_system_prompt_tokens": system_prompt_tokens,
        "history_prompt_tokens": history_prompt_tokens,
        "user_prompt_tokens": user_prompt_tokens,
        "tool_schema_tokens": tool_schema_tokens,
        "estimated_total_request_tokens": estimated_total_request_tokens,
        "tool_count": tool_schemas.len(),
        "allowed_evolvable_spine_fragment_ids": ALLOWED_EVOLVABLE_SPINE_FRAGMENT_IDS,
        "spine_prompt_fragments": spine_prompt_bundle.fragment_metadata_json(),
        "sections": serde_json::Value::Object(sections),
    })
}

fn spine_trace_steps(events: &[SpineTraceEvent]) -> Vec<ExecutionStep> {
    events
        .iter()
        .map(|event| {
            let (icon, title, step_type, data) = match event {
                SpineTraceEvent::PromptTelemetry { data } => (
                    "[prompt]",
                    "Prompt Telemetry",
                    "info",
                    serde_json::to_string(data).ok(),
                ),
                SpineTraceEvent::ArkDistillTelemetry { data } => (
                    "[distill]",
                    "ArkDistill Context Savings",
                    "info",
                    serde_json::to_string(data).ok(),
                ),
                SpineTraceEvent::TurnStarted { .. } => (
                    "[spine]",
                    "Spine Turn Started",
                    "info",
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::ModelCompleted { .. } => (
                    "[model]",
                    "Model Completed",
                    "info",
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::CompletionVerificationStarted { .. } => (
                    "[check]",
                    "Checking Final Answer",
                    "info",
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::CompletionVerificationCompleted { complete, .. } => (
                    "[check]",
                    if *complete {
                        "Final Answer Accepted"
                    } else {
                        "Final Answer Needs More Work"
                    },
                    if *complete { "success" } else { "warning" },
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::ToolStarted { name, .. } => (
                    "[tool]",
                    name.as_str(),
                    "tool_start",
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::ToolCompleted { name, .. } => (
                    "[tool]",
                    name.as_str(),
                    "tool_result",
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::TurnCompleted { terminal_state, .. } => (
                    "[spine]",
                    match terminal_state {
                        SpineTerminalState::Completed => "Spine Completed",
                        SpineTerminalState::NeedsInput => "Spine Needs Input",
                        SpineTerminalState::Blocked => "Spine Blocked",
                        SpineTerminalState::MaxTurnsExceeded => "Spine Max Turns",
                        SpineTerminalState::Cancelled => "Spine Cancelled",
                        SpineTerminalState::PausedForApproval => "Spine Paused",
                        SpineTerminalState::PlatformFailed => "Spine Failed",
                    },
                    if matches!(terminal_state, SpineTerminalState::Completed) {
                        "success"
                    } else {
                        "warning"
                    },
                    serde_json::to_string(event).ok(),
                ),
            };
            // Model-call steps carry their true provider latency; every other
            // step has no measured duration (kept at 0).
            let duration_ms = if let SpineTraceEvent::ModelCompleted { latency_ms, .. } = event {
                Some(*latency_ms)
            } else {
                Some(0)
            };
            ExecutionStep {
                icon: icon.to_string(),
                title: title.to_string(),
                detail: data.clone().unwrap_or_default(),
                step_type: step_type.to_string(),
                data,
                timestamp: chrono::Utc::now(),
                duration_ms,
            }
        })
        .collect()
}

fn spine_cache_usage_from_events(events: &[SpineTraceEvent]) -> (i64, i64) {
    let mut cache_read_tokens = 0usize;
    let mut cache_creation_tokens = 0usize;
    for event in events {
        if let SpineTraceEvent::ModelCompleted {
            cache_read_tokens: read,
            cache_creation_tokens: created,
            ..
        } = event
        {
            cache_read_tokens = cache_read_tokens.saturating_add(*read);
            cache_creation_tokens = cache_creation_tokens.saturating_add(*created);
        }
    }
    (
        cache_read_tokens.min(i64::MAX as usize) as i64,
        cache_creation_tokens.min(i64::MAX as usize) as i64,
    )
}

fn spine_turn_records(result: &SpineResult) -> Vec<AgentTurnRecord> {
    let messages = match result {
        SpineResult::Completed { messages, .. }
        | SpineResult::NeedsInput { messages, .. }
        | SpineResult::Blocked { messages, .. }
        | SpineResult::MaxTurnsExceeded { messages, .. }
        | SpineResult::Cancelled { messages, .. }
        | SpineResult::PausedForApproval { messages, .. }
        | SpineResult::PlatformFailed { messages, .. } => messages,
    };
    let mut tool_names_by_id = HashMap::new();
    for message in messages {
        if let SpineMessage::Assistant { tool_calls, .. } = message {
            for call in tool_calls {
                tool_names_by_id.insert(call.id.clone(), call.name.clone());
            }
        }
    }
    messages
        .iter()
        .filter_map(|message| match message {
            SpineMessage::Tool {
                tool_call_id,
                content,
            } => Some(AgentTurnRecord {
                goal_id: tool_call_id.clone(),
                outcome: structured_turn_record_outcome(content),
                action_name: tool_names_by_id
                    .get(tool_call_id)
                    .cloned()
                    .or_else(|| Some("spine_tool_result".to_string())),
                side_effect: None,
                resolved_object_ref: None,
                tool_output: serde_json::from_str(content).ok(),
                reason: None,
                clarification_question: None,
            }),
            _ => None,
        })
        .collect()
}

fn structured_turn_record_outcome(content: &str) -> AgentTurnOutcomeKind {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) else {
        return AgentTurnOutcomeKind::Abandoned;
    };
    match super::tool_responses::structured_tool_value_outcome(&value).map(|report| report.state) {
        Some(super::tool_responses::StructuredToolOutcomeState::Success) => {
            AgentTurnOutcomeKind::Succeeded
        }
        Some(super::tool_responses::StructuredToolOutcomeState::NeedsInput) => {
            AgentTurnOutcomeKind::NeedsClarification
        }
        Some(super::tool_responses::StructuredToolOutcomeState::Failure) | None => {
            AgentTurnOutcomeKind::Abandoned
        }
    }
}

fn build_spine_system_prompt(
    extra_system: &str,
    prompt_bundle: Option<&crate::core::self_evolve::PromptBundleProfile>,
    prompt_fragment_bundle: Option<
        &crate::core::model::prompt_fragments::PromptFragmentBundleProfile,
    >,
) -> String {
    spine_prompt_bundle::build_spine_prompt_bundle(
        extra_system,
        prompt_bundle,
        prompt_fragment_bundle,
        &PRIMITIVE_NAMES,
    )
    .render()
}

fn schema_property(schema_type: &str, description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": schema_type,
        "description": description,
    })
}

fn described_schema_property(description: &str) -> serde_json::Value {
    serde_json::json!({
        "description": description,
    })
}

fn array_schema_property(description: &str, items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "description": description,
        "items": items,
    })
}

fn insert_schema_property(
    properties: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    schema: serde_json::Value,
) {
    properties.insert(key.to_string(), schema);
}

fn resource_rw_condition_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "Structured watcher trigger condition. Required when creating a watcher from a semantic target.",
        "properties": {
            "description": {
                "type": "string",
                "description": "Human-readable summary of what counts as a match. For change-detection watchers, state the material difference to compare against the previous successful poll."
            },
            "evaluation_mode": {
                "type": "string",
                "enum": ["current_state", "change"],
                "description": "Use current_state when the present poll result alone should trigger. Use change when the watcher should establish a baseline and only trigger when a later successful poll materially differs."
            },
            "type": {
                "type": "string",
                "enum": ["not_empty", "text_contains", "regex", "json_predicate", "json_logic", "llm"],
                "description": "Matcher type. Use llm only when the trigger cannot be expressed safely as a deterministic contract."
            },
            "text": {
                "type": "string",
                "description": "Required by text_contains."
            },
            "case_sensitive": {
                "type": "boolean",
                "description": "Optional text_contains case-sensitivity flag."
            },
            "pattern": {
                "type": "string",
                "description": "Required by regex."
            },
            "path": {
                "type": "string",
                "description": "Dot-path into structured poll output for json_predicate."
            },
            "operator": {
                "type": "string",
                "enum": ["exists", "not_exists", "eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "non_empty", "empty", "true", "false", "regex"],
                "description": "Operator for json_predicate."
            },
            "value": {
                "description": "Comparison value for operators that require one."
            },
            "logic": {
                "type": "string",
                "enum": ["all", "any"],
                "description": "Used by json_logic to combine rules."
            },
            "rules": {
                "type": "array",
                "description": "Rules for json_logic.",
                "items": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "operator": {
                            "type": "string",
                            "enum": ["exists", "not_exists", "eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "non_empty", "empty", "true", "false", "regex"]
                        },
                        "value": {}
                    },
                    "required": ["path", "operator"]
                }
            }
        },
        "required": ["description", "evaluation_mode", "type"]
    })
}

fn resource_rw_content_properties() -> serde_json::Map<String, serde_json::Value> {
    let mut properties = serde_json::Map::new();
    for (key, description) in [
        (
            "task",
            "Scheduled-task body: what should run or be delivered when the time or recurrence fires.",
        ),
        (
            "id",
            "Resource id inside the selected adapter payload when the id is supplied under content.",
        ),
        (
            "name",
            "Human-readable resource or integration name for adapters that create or update named resources.",
        ),
        (
            "message",
            "Notification body for direct notifications or scheduled notification tasks.",
        ),
        (
            "title",
            "Optional user-facing title for notifications, goals, resources, or artifacts.",
        ),
        (
            "delivery_channel",
            "Notification delivery route for direct notification or durable-work delivery updates.",
        ),
        (
            "channel",
            "Alias for a notification delivery route when creating notification resources.",
        ),
        (
            "recipient",
            "Optional notification recipient or channel-specific destination.",
        ),
        (
            "path",
            "Resource path, API path, operation path, or managed file path depending on the selected resource kind.",
        ),
        ("task_id", "Existing scheduled task id for updates."),
        (
            "cron",
            "Five-field recurring schedule at minute granularity.",
        ),
        ("at", "ISO 8601 timestamp for one-time scheduled work."),
        (
            "scheduled_for",
            "Alias for an ISO 8601 one-time scheduled timestamp.",
        ),
        (
            "local_time",
            "Wall-clock time for scheduled work when local date or timezone context should resolve it.",
        ),
        ("local_date", "Optional local date paired with local_time."),
        ("timezone", "IANA timezone for local_time scheduling."),
        (
            "report_to",
            "Scheduled-task delivery route; use in_app for web-only notifications.",
        ),
        (
            "action",
            "Scheduled-task action to execute when the schedule fires. Omit only for a simple notification reminder.",
        ),
        (
            "base_url",
            "Base URL for a custom API or HTTP-backed integration.",
        ),
        (
            "openapi_url",
            "URL to an OpenAPI document for custom API acquisition.",
        ),
        (
            "openapi_text",
            "Inline OpenAPI document text for custom API acquisition.",
        ),
        ("docs_url", "Documentation URL for custom API acquisition."),
        (
            "docs_text",
            "Inline documentation text for custom API acquisition.",
        ),
        (
            "method",
            "HTTP method for custom API operation definitions.",
        ),
        (
            "response_notes",
            "Notes about response shape or extraction for custom API operations.",
        ),
        (
            "auth_type",
            "Authentication type for custom API or MCP setup.",
        ),
        ("auth_mode", "Authentication mode for custom API setup."),
        (
            "auth_header",
            "Authentication header value placeholder or metadata for custom API setup.",
        ),
        (
            "auth_header_name",
            "Header name used for custom API authentication.",
        ),
        ("auth_name", "Named credential or auth parameter label."),
        (
            "auth_username",
            "Username metadata for auth modes that require one.",
        ),
        (
            "auth_profile_id",
            "Reusable auth profile id for OAuth or advanced auth handled outside direct secret fields.",
        ),
        (
            "pack_id",
            "Extension-pack id for install, connect, enable, disable, test, or delete operations.",
        ),
        ("source_url", "Source URL for extension-pack installation."),
        (
            "source_path",
            "Local or managed source path for extension-pack installation.",
        ),
        ("manifest_text", "Inline extension-pack manifest text."),
        (
            "url",
            "HTTP URL for MCP server setup, custom API operation setup, or other URL-backed resource adapters.",
        ),
        (
            "command",
            "Command for stdio MCP server setup or command-backed resource configuration.",
        ),
        (
            "goal_id",
            "Existing goal id for goal resource updates or deletes.",
        ),
        ("goal", "Goal text or semantic goal reference."),
        ("new_goal", "Updated goal text for goal resource updates."),
        ("due_date", "Optional goal due date."),
        (
            "profile_id",
            "Exact saved browser profile id for browser-profile resources.",
        ),
        (
            "profile",
            "Browser profile selector by id, name, target, tag, or semantic description.",
        ),
        ("profile_name", "Human-readable browser profile name."),
        (
            "script",
            "Optional script for scheduled execution or watcher polling when a low-level implementation is intentionally supplied.",
        ),
        ("script_language", "Language for script execution."),
        (
            "description",
            "Watcher target or durable resource description. For watchers, this is the semantic monitored target.",
        ),
        ("watcher_id", "Existing watcher id for updates."),
        (
            "poll_action",
            "Optional advanced watcher poll action override. Omit for semantic watcher polling.",
        ),
        (
            "on_trigger",
            "Watcher follow-up instructions executed only when the condition is met.",
        ),
    ] {
        insert_schema_property(&mut properties, key, schema_property("string", description));
    }
    for (key, description) in [
        ("action_arguments", "Arguments for the scheduled action."),
        (
            "default_headers",
            "Default non-secret headers for custom API operation definitions.",
        ),
        (
            "headers",
            "Headers for HTTP/custom API operation definitions; secrets should use secure credential fields or profiles.",
        ),
        (
            "default_query",
            "Default query parameters for custom API operation definitions.",
        ),
        (
            "send",
            "Outbound send specification for creating or updating a custom messaging channel.",
        ),
        ("manifest", "Parsed extension-pack manifest object."),
        (
            "transport",
            "Structured transport configuration for MCP server setup.",
        ),
        (
            "poll_arguments",
            "Arguments for an optional low-level watcher poll action.",
        ),
        (
            "validation",
            "Optional validation policy for durable automation results.",
        ),
        (
            "automation_policy",
            "Advanced durable automation execution policy.",
        ),
    ] {
        insert_schema_property(&mut properties, key, schema_property("object", description));
    }
    for (key, description) in [
        (
            "operations",
            "Operation definitions for custom API acquisition.",
        ),
        (
            "required_inputs",
            "Required request argument descriptors for custom API operations.",
        ),
        (
            "parameters",
            "Parameter descriptors for custom API operation definitions.",
        ),
        (
            "items",
            "Batch of independent scheduled-task or watcher outcomes. Items inherit top-level fields unless overridden.",
        ),
    ] {
        insert_schema_property(
            &mut properties,
            key,
            array_schema_property(description, serde_json::json!({ "type": "object" })),
        );
    }
    for (key, description) in [
        ("read_only", "Whether a custom API operation is read-only."),
        (
            "body_required",
            "Whether a custom API operation requires a request body.",
        ),
        (
            "clear_secret",
            "Whether to clear a stored secret for an integration that supports credential removal.",
        ),
        (
            "until_stopped",
            "Keep a watcher active until explicitly stopped.",
        ),
        (
            "repeat_on_match",
            "Keep a watcher active after a match for ongoing monitoring.",
        ),
        (
            "allow_duplicate",
            "Create a duplicate resource when supported.",
        ),
    ] {
        insert_schema_property(
            &mut properties,
            key,
            schema_property("boolean", description),
        );
    }
    for (key, description) in [
        ("interval_secs", "Watcher polling cadence in seconds."),
        ("timeout_secs", "Max seconds before a watcher times out."),
        ("timeout_hours", "Convenience watcher timeout in hours."),
        ("timeout_days", "Convenience watcher timeout in days."),
        (
            "max_attempts",
            "Maximum supervised retry attempts for durable automation.",
        ),
        (
            "stall_timeout_secs",
            "Maximum seconds before durable automation is treated as stalled.",
        ),
        (
            "retry_backoff_secs",
            "Base retry backoff for durable automation failures.",
        ),
    ] {
        insert_schema_property(
            &mut properties,
            key,
            schema_property("integer", description),
        );
    }
    for (key, description) in [
        (
            "operation",
            "Operation id, operation object, or lifecycle operation for resource adapters that need one.",
        ),
        (
            "body",
            "Request body template or default body for custom API operation definitions.",
        ),
        (
            "body_template",
            "Request body template for custom API operation definitions.",
        ),
        (
            "auth",
            "Authentication metadata for custom API or MCP setup. Secrets must use secure credential handling.",
        ),
    ] {
        insert_schema_property(&mut properties, key, described_schema_property(description));
    }
    insert_schema_property(&mut properties, "condition", resource_rw_condition_schema());
    properties
}

fn resource_rw_content_schema() -> serde_json::Value {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".to_string()),
    );
    schema.insert(
        "description".to_string(),
        serde_json::Value::String(format!(
            "Payload for the selected resource adapter. For durable kinds the full argument contract is declared here; satisfy it in the first call. kind=scheduled_task create/update contract: {}. kind=watcher create/update contract: {}. For other kinds, mandatory fields are enforced by registered adapter/action contracts at runtime; incomplete setup payloads route to capability resolution or inspection before mutation. Generated file content, generated app source, and generated SKILL.md content belong in file_write/file_patch, app_deploy, or skill_manage.",
            super::task_runtime::schedule_task_expected_contract(),
            super::task_runtime::watch_expected_contract(),
        )),
    );
    schema.insert(
        "properties".to_string(),
        serde_json::Value::Object(resource_rw_content_properties()),
    );
    schema.insert(
        "additionalProperties".to_string(),
        serde_json::Value::Bool(true),
    );
    serde_json::Value::Object(schema)
}

fn build_primitive_schemas() -> Vec<ActionDef> {
    vec![
        primitive_schema(
            "search",
            "Discover public information and research evidence. Provide a semantic query and optional depth/freshness constraints.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Semantic search or research query." },
                    "depth": { "type": "string", "enum": ["quick", "standard", "deep"], "default": "standard" },
                    "freshness": { "type": "string", "description": "Optional temporal requirement such as current, recent, historical, or a date range." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "fetch",
            "Read HTTP resources or connected integration data without browser interaction.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "method": { "type": "string", "enum": ["GET", "POST"], "default": "GET" },
                    "headers": { "type": "object", "description": "Optional HTTP headers for direct URL fetches. Values may use secure placeholders accepted by the underlying HTTP primitive." },
                    "body": { "description": "Optional JSON request body for POST-style direct HTTP calls or saved custom API operations." },
                    "arguments": { "type": "object", "description": "Explicit saved integration/custom API operation arguments. Use when the target operation has a typed argument contract." },
                    "integration": { "type": "string", "enum": ["gmail", "calendar", "google_drive", "connector", "custom_api"], "description": "Canonical integration surface for connected reads. Use custom_api with id plus operation/body to call a saved read-only custom API action." },
                    "id": { "type": "string", "description": "Saved integration/custom API id when integration=custom_api." },
                    "operation": { "type": "string", "description": "Saved custom API operation id, action name, or operation label when integration=custom_api." },
                    "op": { "type": "string", "description": "Read operation such as read, list, query, today, free_busy." },
                    "query": { "type": "string" },
                    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 300 },
                    "save_to": { "type": "string", "description": "Optional workspace/data file path for raw response bytes." },
                    "as_resource": { "type": "boolean", "description": "Return the exact response bytes as a ResourceRef when later steps must reuse the fetched artifact as a file instead of clipped readable text." },
                    "suggested_name": { "type": "string", "description": "Optional safe filename hint for an as_resource response. The runtime still chooses the managed storage location." },
                    "persist_response": { "type": "array", "items": {}, "description": "Optional response-field persistence contract forwarded to the HTTP primitive." },
                    "content": {
                        "type": "object",
                        "description": "Memory mutation payload. For write/update, provide key plus value or text when the user explicitly asked to manage memory.",
                        "properties": {
                            "key": {
                                "type": "string",
                                "description": "Memory key to write, update, or delete."
                            },
                            "value": {
                                "type": "string",
                                "description": "Memory value for write/update."
                            },
                            "text": {
                                "type": "string",
                                "description": "Alias for memory value when the stored content is text."
                            },
                            "kind": {
                                "type": "string",
                                "description": "Optional memory kind or category metadata."
                            },
                            "scope": {
                                "type": "string",
                                "description": "Optional memory scope metadata."
                            },
                            "slot_cardinality": {
                                "type": "string",
                                "enum": ["singleton", "collection"],
                                "description": "Use singleton when later values should replace one active row; use collection when multiple concrete values should coexist under this generic key."
                            }
                        },
                        "additionalProperties": true
                    },
                    "metadata": { "type": "object" }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "browse",
            "Use browser automation for pages requiring interaction, screenshots, login handoff, dynamic rendering, or visual inspection.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["start_session", "snapshot", "resume_handoff"], "default": "start_session", "description": "Use start_session for new or continued browser automation, snapshot to read the current live browser page without changing it, and resume_handoff to pass the user's chat reply or handoff note back to a waiting browser loop." },
                    "url": { "type": "string" },
                    "task": { "type": "string", "description": "Browser task to perform." },
                    "session_id": { "type": "string", "description": "Optional live browser session id from current browser session context." },
                    "note": { "type": "string", "description": "User reply, choice, or handoff note to return to a waiting browser loop when action=resume_handoff." },
                    "resume_in_chat": { "type": "boolean", "description": "When action=resume_handoff, return control to chat instead of continuing the browser loop." },
                    "profile": { "type": "string", "description": "Optional saved browser profile selector by id, name, target, tag, or semantic description when the task should reuse a saved login or browser identity." },
                    "profile_id": { "type": "string", "description": "Optional exact saved browser profile id when the task should reuse a saved login or browser identity." },
                    "expectation": { "type": "string", "description": "User-facing browser completion expectation, checkpoint, stop condition, requested question, or follow-up choice that must be preserved for the browser loop." },
                    "metadata": { "type": "object" }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "code_exec",
            "Run sandboxed computation, shell commands, tests, builds, parsers, or local analysis. Use ordered command probes or small scripts for diagnostics such as version checks, installed-package checks, logs, builds, and tests before deciding what to patch or restart.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["script", "command", "test", "build", "analysis"], "default": "script" },
                    "language": { "type": "string", "description": "Required when using inline code. Use the actual language/runtime for the code body." },
                    "code": { "type": "string" },
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "files": { "type": "array", "items": {} },
                    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 3600 },
                    "metadata": { "type": "object" }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "pdf_generate",
            "Create a managed PDF document artifact from complete supplied Markdown/text content. Use this for PDF deliverables so AgentArk returns a Documents-visible managed file directly instead of requiring code execution or filesystem handoff. Include fenced agentark-chart JSON blocks for charts when the evidence contains concrete numeric values; AgentArk renders those blocks as vector charts in the PDF. When chart fences are absent, numeric Markdown tables are automatically summarized as PDF charts.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Human-readable document title." },
                    "filename": { "type": "string", "description": "Suggested PDF filename. AgentArk normalizes it safely." },
                    "style": { "type": "string", "enum": ["plain", "report", "letter", "invoice"], "default": "report" },
                    "content": { "type": "string", "description": "Complete final Markdown/text body to render into the PDF. Fenced agentark-chart JSON blocks are rendered as PDF charts; numeric Markdown tables are auto-charted when chart fences are absent." },
                    "metadata": { "type": "object", "description": "Optional provenance or artifact identity metadata." }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "app_deploy",
            "Deploy or update a managed browser-runnable app, dashboard, page, game, tool, repo, or local service. Use this directly for generated UI/source artifacts; do not route app source through resource_rw. Include request_context and acceptance_criteria so deploy review can validate the app against the user's semantic requirements and explicit preferences, independent of exact phrasing. Prefer app_id plus mode=patch and file_patches for edits to an existing app. For multi-file or large generated apps, stage files with file_write under one workspace directory, then use source_dir; include source_paths only to deploy a deliberate subset. Use files for atomic small bundles, and repo_url for repository deploys. Duplicate deployments are not created unless allow_duplicate=true or duplicate_policy=create_new.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": { "type": "string", "description": "Existing deployed app id to update in place." },
                    "mode": { "type": "string", "enum": ["replace", "patch"], "default": "replace" },
                    "title": { "type": "string" },
                    "request_context": { "type": "string", "description": "Semantic summary of the user-requested app outcome, including explicit implementation preferences or constraints. Used for deploy acceptance review; do not use this as a trigger phrase." },
                    "acceptance_criteria": { "type": "array", "items": { "type": "string" }, "description": "Concrete requested capabilities, workflows, data/persistence requirements, runtime/integration constraints, and user preferences that the deployed app must satisfy." },
                    "files": { "type": "object", "description": "App-relative file path to complete file body map." },
                    "file_patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "patch": { "type": "string" }
                            },
                            "required": ["path", "patch"],
                            "additionalProperties": false
                        }
                    },
                    "delete_paths": { "type": "array", "items": { "type": "string" } },
                    "source_dir": { "type": "string", "description": "Workspace/data directory containing staged app files. If source_paths is omitted, deployable files under this directory are discovered automatically." },
                    "source_paths": { "type": "array", "items": { "type": "string" }, "description": "Optional app-relative staged files to deploy when only a subset of source_dir should be published." },
                    "repo_url": { "type": "string" },
                    "repo_ref": { "type": "string" },
                    "repo_subdir": { "type": "string" },
                    "service_mode": { "type": "string", "enum": ["auto", "frontend", "backend", "fullstack"] },
                    "deploy_target": { "type": "string", "enum": ["local", "vercel_direct", "vercel_git"], "default": "local" },
                    "entry_command": { "type": "string" },
                    "start_command": { "type": "string" },
                    "install_command": { "type": "string" },
                    "stop_command": { "type": "string" },
                    "commands": { "type": "object" },
                    "runtime_required": { "type": "boolean" },
                    "runtime_reason": { "type": "string" },
                    "runtime_image": { "type": "string" },
                    "runtime_preference": { "type": "string", "enum": ["local", "container"] },
                    "required_inputs": { "type": "array", "items": {} },
                    "required_secrets": { "type": "array", "items": { "type": "string" } },
                    "required_config": { "type": "array", "items": { "type": "string" } },
                    "required_env": { "type": "array", "items": { "type": "string" } },
                    "config": { "type": "object" },
                    "runtime_actions": { "type": "array", "items": {} },
                    "expose_public": { "type": "boolean" },
                    "access_guard": { "type": "boolean" },
                    "access_password": { "type": "string" },
                    "replace_existing": { "type": "boolean" },
                    "duplicate_policy": { "type": "string", "enum": ["reuse_existing", "create_new"], "default": "reuse_existing" },
                    "allow_duplicate": { "type": "boolean" },
                    "metadata": { "type": "object" },
                    "artifact_identity": { "type": "object" }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "file_read",
            "Read one managed workspace/data file. Use for source inspection before small edits.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "file_search",
            "Search workspace/data files by path and content without shell access. Use to locate source files before reading or patching.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "filename_query": { "type": "string" },
                    "content_query": { "type": "string" },
                    "mode": { "type": "string", "enum": ["auto", "filename", "content", "both"], "default": "auto" },
                    "root": { "type": "string" },
                    "globs": { "type": "array", "items": { "type": "string" } },
                    "exclude_globs": { "type": "array", "items": { "type": "string" } },
                    "context_lines": { "type": "integer", "minimum": 0, "maximum": 8 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "case_sensitive": { "type": "boolean" },
                    "max_file_bytes": { "type": "integer", "minimum": 4096, "maximum": 2000000 }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "file_write",
            "Write one workspace/data file from complete content, base64 bytes, source_path, or source_resource. Use this for files/documents/source assets and for staging generated app source before app_deploy.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "content_base64": { "type": "string" },
                    "source_resource": {},
                    "source_path": { "type": "string" },
                    "content_type": { "type": "string" },
                    "document_visible": { "type": "boolean" },
                    "index_document": { "type": "boolean" },
                    "duplicate_policy": { "type": "string", "enum": ["reuse_existing", "create_new"], "default": "reuse_existing" },
                    "allow_duplicate": { "type": "boolean" },
                    "metadata": { "type": "object" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "file_patch",
            "Apply targeted unified diffs to existing workspace/data files. Use this for small edits instead of rewriting whole files.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "patch": { "type": "string" },
                    "patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "patch": { "type": "string" }
                            },
                            "required": ["path", "patch"],
                            "additionalProperties": false
                        }
                    },
                    "dry_run": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "file_delete",
            "Delete one managed workspace/data file. Use only when the user intends file removal.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "skill_manage",
            "Create, update, import, install, inspect, test, enable, disable, archive, restore, or delete AgentArk skills directly. Use this for generated SKILL.md content or skill URLs; resource_rw remains for lifecycle/resource inspection only.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string", "enum": ["create", "update", "import", "install", "delete", "list", "read", "status", "enable", "disable", "refresh", "test", "pin", "unpin", "archive", "restore"] },
                    "name": { "type": "string" },
                    "id": { "type": "string" },
                    "url": { "type": "string" },
                    "content": { "type": "string", "description": "Complete SKILL.md content." },
                    "markdown": { "type": "string", "description": "Complete SKILL.md content." },
                    "enabled": { "type": "boolean" },
                    "security_confirmed": { "type": "boolean" },
                    "arguments": { "type": "object" },
                    "allow_duplicate": { "type": "boolean" },
                    "metadata": { "type": "object" }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "resource_rw",
            "Create, read, update, delete, list, pause, resume, connect, install, refresh, test, or inspect backed durable AgentArk resources, notifications, local activity evidence, and external integration surfaces. Use notification for user-facing notification delivery; include schedule fields when it should happen later. Use activity for recent work, recent conversations, Reflect/Sentinel signals, work patterns, attention, avoidance, recurring themes, and retrospective personal activity questions. Use watcher for autonomous condition/change monitoring and notify-only-when outcomes; use scheduled_task for pure time-based reminders or recurring work whose action/script is explicitly supplied. Use app_service or dashboard only for app resource lifecycle/status/control, not generated source or app files. Use integration/custom_api/custom_messaging_channel/extension_pack/mcp_server when setting up or inspecting external capabilities for AgentArk itself. Prefer native/bundled integrations, extension packs, or custom_api for official HTTP/REST/GraphQL provider APIs; choose mcp_server only when the requested substrate is explicitly MCP. Use direct app_deploy, file_* and skill_manage primitives for generated source/content. If a requested kind/op is unsupported, the tool result is terminal evidence; do not loop.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": resource_valid_ops() },
                    "kind": {
                        "type": "string",
                        "enum": resource_valid_kinds(),
                        "description": "Resource substrate registered by the backend resource adapter table. Choose the adapter by the requested resource semantics, then use the adapter's supported operations. Generated source/content belongs in direct authoring primitives such as app_deploy, file_write/file_patch, or skill_manage rather than resource_rw."
                    },
                    "id": {
                        "type": "string",
                        "description": "Identifier for an existing resource. For file read/delete, use a managed workspace/data-relative path or resource id."
                    },
                    "query": { "type": "string", "description": "Semantic lookup, listing, or matching text for the target resource." },
                    "content": resource_rw_content_schema(),
                    "metadata": {
                        "type": "object",
                        "description": "Optional provenance, title, source URLs, refresh notes, workflow steps, or non-sensitive operational metadata. Do not use metadata to request scheduling unless the user intent requires later autonomous execution."
                    },
                    "duplicate_policy": {
                        "type": "string",
                        "enum": ["reuse_existing", "create_new"],
                        "default": "reuse_existing",
                        "description": "For resource kinds that support duplicate detection, reuse/skip identical existing artifacts by default. Use create_new only when the user explicitly wants another duplicate copy."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Compatibility boolean for duplicate_policy=create_new. Default false."
                    }
                },
                "required": ["op", "kind"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "memory_rw",
            "Explicit foreground memory read, write, update, or deletion. Mutations are only for active user intent to manage memory; incidental facts are handled by background capture.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["search", "read", "write", "update", "delete"] },
                    "query": { "type": "string" },
                    "id": { "type": "string" },
                    "content": {
                        "type": "object",
                        "description": "Memory mutation payload. For write/update, provide key plus value or text when the user explicitly asked to manage memory.",
                        "properties": {
                            "key": {
                                "type": "string",
                                "description": "Memory key to write, update, or delete."
                            },
                            "value": {
                                "type": "string",
                                "description": "Memory value for write/update."
                            },
                            "text": {
                                "type": "string",
                                "description": "Alias for memory value when the stored content is text."
                            },
                            "kind": {
                                "type": "string",
                                "description": "Optional memory kind or category metadata."
                            },
                            "scope": {
                                "type": "string",
                                "description": "Optional memory scope metadata."
                            }
                        },
                        "additionalProperties": true
                    },
                    "metadata": { "type": "object" },
                    "explicit_user_request": { "type": "boolean", "description": "Required true for write, update, and delete. True only when the user is asking to manage saved memory now, not merely sharing information." },
                    "intent_summary": { "type": "string", "description": "Brief semantic reason the foreground memory operation is needed." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                },
                "required": ["op"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "action_call",
            "Invoke a specific installed runtime action by exact action_name and declared arguments. Use only when the runtime action directory or prior tool evidence supplies the exact action id and another higher-level primitive does not cover the capability. This works for built-in, custom, plugin, MCP, custom API, extension-pack, custom messaging, and dynamic integration actions that the runtime exposes. Do not invent action names.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action_name": {
                        "type": "string",
                        "description": "Exact installed runtime action id from the runtime action directory or a prior tool result."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "JSON object matching the selected action's declared input schema."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Brief semantic reason this exact action is the right runtime capability. This is trace metadata, not a selector."
                    }
                },
                "required": ["action_name", "arguments"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "delegate",
            "Delegate bounded work to a sub-agent or external delegated executor.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string" },
                    "role": { "type": "string" },
                    "context": { "type": "object" },
                    "metadata": { "type": "object" }
                },
                "required": ["task"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn primitive_schema(name: &str, description: &str, input_schema: serde_json::Value) -> ActionDef {
    ActionDef {
        name: name.to_string(),
        description: description.to_string(),
        version: "1.0.0".to_string(),
        input_schema,
        capabilities: vec!["spine_primitive".to_string(), name.to_string()],
        sandbox_mode: None,
        source: crate::actions::ActionSource::System,
        file_path: None,
        authorization: ActionAuthorization::default(),
    }
}

fn plan_search(arguments: &serde_json::Value) -> PrimitivePlan {
    let Some(query) = json_text(arguments, "query") else {
        return unsupported("search requires `query`.");
    };
    let depth = json_text(arguments, "depth").unwrap_or_else(|| "standard".to_string());
    if depth.eq_ignore_ascii_case("deep") {
        let mut research_args = serde_json::json!({
            "query": query,
            "depth": "deep",
        });
        if let Some(limit) = json_usize(arguments, "limit") {
            research_args["max_sources"] = serde_json::json!(limit);
        }
        PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "research".to_string(),
            arguments: research_args,
        }])
    } else {
        PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": query,
                "limit": json_usize(arguments, "limit").unwrap_or(5),
            }),
        }])
    }
}

fn plan_fetch(arguments: &serde_json::Value) -> PrimitivePlan {
    if let Some(url) = json_text(arguments, "url") {
        let method = json_text(arguments, "method").unwrap_or_else(|| "GET".to_string());
        let payload = merge_content_metadata(arguments);
        let needs_direct_http = payload.get("headers").is_some()
            || payload.get("body").is_some()
            || payload.get("save_to").is_some()
            || payload
                .get("as_resource")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            || payload.get("suggested_name").is_some()
            || payload.get("persist_response").is_some()
            || payload.get("timeout_secs").is_some()
            || payload.get("query").is_some_and(|value| value.is_object());
        if method.eq_ignore_ascii_case("GET") && !needs_direct_http {
            return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "page_fetch".to_string(),
                arguments: serde_json::json!({ "url": url }),
            }]);
        }
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "http_request".to_string(),
            arguments: merge_objects(serde_json::json!({ "url": url, "method": method }), payload),
        }]);
    }

    let integration = json_text(arguments, "integration")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let op = json_text(arguments, "op")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "read".to_string());
    let query = json_text(arguments, "query");
    match integration.as_str() {
        "gmail" => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "gmail_scan".to_string(),
            arguments: serde_json::json!({ "query": query, "mode": "search" }),
        }]),
        "calendar" => {
            let action_name = if op == "today" {
                "calendar_today"
            } else {
                "calendar_list"
            };
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: action_name.to_string(),
                arguments: merge_content_metadata(arguments),
            }])
        }
        "google_drive" => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "google_drive_search".to_string(),
            arguments: serde_json::json!({ "query": query.unwrap_or_default() }),
        }]),
        "connector" => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "connector_request".to_string(),
            arguments: merge_content_metadata(arguments),
        }]),
        "custom_api" => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                object.remove("integration");
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "custom_api_request".to_string(),
                arguments: payload,
            }])
        }
        _ => unsupported("fetch requires either `url` or a supported `integration` read target."),
    }
}

fn plan_browse(arguments: &serde_json::Value) -> PrimitivePlan {
    let action = json_text(arguments, "action").unwrap_or_else(|| "start_session".to_string());
    let task = json_text(arguments, "task");
    if action == "start_session" && task.is_none() {
        return unsupported("browse requires `task`.");
    }
    let mut payload = serde_json::Map::new();
    payload.insert("action".to_string(), serde_json::Value::String(action));
    if let Some(task) = task {
        payload.insert("task".to_string(), serde_json::Value::String(task));
    }
    if let Some(url) = json_text(arguments, "url") {
        payload.insert("url".to_string(), serde_json::Value::String(url));
    }
    if let Some(session_id) = json_text(arguments, "session_id") {
        payload.insert(
            "session_id".to_string(),
            serde_json::Value::String(session_id),
        );
    }
    if let Some(note) = json_text(arguments, "note") {
        payload.insert("note".to_string(), serde_json::Value::String(note));
    }
    if let Some(resume_in_chat) = arguments
        .get("resume_in_chat")
        .and_then(|value| value.as_bool())
    {
        payload.insert(
            "resume_in_chat".to_string(),
            serde_json::Value::Bool(resume_in_chat),
        );
    }
    if let Some(profile) = json_text(arguments, "profile") {
        payload.insert("profile".to_string(), serde_json::Value::String(profile));
    }
    if let Some(profile_id) = json_text(arguments, "profile_id") {
        payload.insert(
            "profile_id".to_string(),
            serde_json::Value::String(profile_id),
        );
    }
    if let Some(expectation) = json_text(arguments, "expectation") {
        payload.insert(
            "expectation".to_string(),
            serde_json::Value::String(expectation),
        );
    }
    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: "browser_auto".to_string(),
        arguments: serde_json::Value::Object(payload),
    }])
}

fn browser_profile_selector_from_request_context(
    context: Option<&serde_json::Value>,
) -> Option<(String, &'static str)> {
    let object = context?.as_object()?;
    for (key, field) in [("profile_id", "profile_id"), ("profile_name", "profile")] {
        if let Some(value) = object
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some((value.to_string(), field));
        }
    }
    None
}

fn browser_auto_invocation_with_request_profile(
    invocation: &PrimitiveActionInvocation,
    browser_profile_context: Option<&serde_json::Value>,
) -> PrimitiveActionInvocation {
    if !invocation.action_name.eq_ignore_ascii_case("browser_auto") {
        return invocation.clone();
    }
    let action = json_text(&invocation.arguments, "action")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "start_session".to_string());
    if action != "start_session" {
        return invocation.clone();
    }
    if json_text(&invocation.arguments, "profile_id").is_some()
        || json_text(&invocation.arguments, "profile").is_some()
    {
        return invocation.clone();
    }
    let Some((selector, field)) =
        browser_profile_selector_from_request_context(browser_profile_context)
    else {
        return invocation.clone();
    };
    let mut arguments = invocation.arguments.clone();
    if let Some(object) = arguments.as_object_mut() {
        object.insert(field.to_string(), serde_json::Value::String(selector));
    }
    PrimitiveActionInvocation {
        action_name: invocation.action_name.clone(),
        arguments,
    }
}

fn plan_code_exec(arguments: &serde_json::Value) -> PrimitivePlan {
    if let Some(command) = json_text(arguments, "command") {
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": command,
                "cwd": json_text(arguments, "cwd"),
            }),
        }]);
    }
    let mut payload = serde_json::Map::new();
    for key in ["language", "code", "files", "timeout_secs", "mode", "cwd"] {
        if let Some(value) = arguments.get(key).cloned() {
            payload.insert(key.to_string(), value);
        }
    }
    if !payload.contains_key("code") {
        return unsupported("code_exec requires either `command` or `code`.");
    }
    if payload
        .get("language")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_none_or(|value| value.is_empty())
    {
        return unsupported_with_extra(
            "code_exec with inline `code` requires `language`.",
            serde_json::json!({
                "field": "language",
                "hint": "Provide the runtime language for the code body, such as python, javascript, typescript, bash, or powershell."
            }),
        );
    }
    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: "code_execute".to_string(),
        arguments: serde_json::Value::Object(payload),
    }])
}

fn plan_pdf_generate(arguments: &serde_json::Value) -> PrimitivePlan {
    let Some(content) = json_text(arguments, "content") else {
        return unsupported("pdf_generate requires complete `content`.");
    };
    let mut payload = serde_json::Map::new();
    payload.insert("content".to_string(), serde_json::Value::String(content));
    if let Some(title) = json_text(arguments, "title") {
        payload.insert("title".to_string(), serde_json::Value::String(title));
    }
    if let Some(filename) = json_text(arguments, "filename") {
        payload.insert("filename".to_string(), serde_json::Value::String(filename));
    }
    payload.insert(
        "style".to_string(),
        serde_json::Value::String(
            json_text(arguments, "style").unwrap_or_else(|| "report".to_string()),
        ),
    );
    payload.insert(
        "document_visible".to_string(),
        serde_json::Value::Bool(true),
    );
    if let Some(metadata) = arguments.get("metadata").cloned() {
        payload.insert("metadata".to_string(), metadata);
    }
    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: "pdf_generate".to_string(),
        arguments: serde_json::Value::Object(payload),
    }])
}

fn plan_direct_action(action_name: &str, arguments: &serde_json::Value) -> PrimitivePlan {
    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: action_name.to_string(),
        arguments: arguments.clone(),
    }])
}

fn plan_action_call(
    arguments: &serde_json::Value,
    action_def: Option<&crate::actions::ActionDef>,
) -> PrimitivePlan {
    let Some(action_name) = json_text(arguments, "action_name") else {
        return unsupported_with_extra(
            "action_call requires `action_name`.",
            serde_json::json!({
                "field": "action_name",
                "hint": "Use an exact action id from the runtime action directory or a prior tool result."
            }),
        );
    };
    let action_name = action_name.trim().to_string();
    if action_name.is_empty() {
        return unsupported("action_call requires a non-empty `action_name`.");
    }
    if PRIMITIVE_NAMES.contains(&action_name.as_str()) {
        return unsupported_with_extra(
            "action_call cannot invoke spine primitives recursively.",
            serde_json::json!({
                "action_name": action_name,
                "hint": "Call the primitive directly with its declared schema."
            }),
        );
    }
    let Some(action_def) = action_def else {
        return unsupported_with_extra(
            "action_call requires an installed runtime action with this exact name.",
            serde_json::json!({
                "action_name": action_name,
                "status": "missing_runtime_action",
                "hint": "Use resource_rw to inspect/configure capabilities, or resolve the missing capability before trying an exact action call."
            }),
        );
    };
    if action_def.name != action_name {
        return unsupported_with_extra(
            "action_call action definition did not match the requested action name.",
            serde_json::json!({
                "requested_action": action_name,
                "resolved_action": action_def.name,
            }),
        );
    }
    let Some(action_arguments) = arguments.get("arguments") else {
        return unsupported_with_extra(
            "action_call requires `arguments`: a JSON object matching the action's declared schema.",
            serde_json::json!({
                "action_name": action_name,
                "field": "arguments",
            }),
        );
    };
    if !action_arguments.is_object() {
        return unsupported_with_extra(
            "action_call `arguments` must be a JSON object.",
            serde_json::json!({
                "action_name": action_name,
                "field": "arguments",
            }),
        );
    }

    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name,
        arguments: action_arguments.clone(),
    }])
}

fn plan_skill_manage(arguments: &serde_json::Value) -> PrimitivePlan {
    let mut payload = arguments.clone();
    let operation = json_text(&payload, "operation")
        .or_else(|| json_text(&payload, "op"))
        .unwrap_or_else(|| "list".to_string());
    if let Some(object) = payload.as_object_mut() {
        let nested_content = object
            .get("content")
            .and_then(|value| value.as_object())
            .cloned();
        if let Some(content_object) = nested_content {
            for (key, value) in content_object {
                object.entry(key).or_insert(value);
            }
        }
        object.insert(
            "operation".to_string(),
            serde_json::Value::String(operation),
        );
        object
            .entry("resource".to_string())
            .or_insert_with(|| serde_json::Value::String("skill".to_string()));
        if !object.contains_key("content") {
            if let Some(markdown) = object.get("markdown").cloned() {
                object.insert("content".to_string(), markdown);
            }
        }
        if !object.contains_key("name") {
            if let Some(target) = object
                .get("id")
                .or_else(|| object.get("query"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            {
                object.insert("name".to_string(), serde_json::Value::String(target));
            }
        }
        object.remove("op");
        object.remove("kind");
    }
    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: "manage_actions".to_string(),
        arguments: payload,
    }])
}

fn plan_resource_rw(arguments: &serde_json::Value) -> PrimitivePlan {
    let op = json_text(arguments, "op")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let kind = json_text(arguments, "kind")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();

    if let Some(plan) = plan_registered_resource_action(arguments, kind.as_str(), op.as_str()) {
        return plan;
    }

    match (kind.as_str(), op.as_str()) {
        ("file", "read") | ("file", "status") => {
            let Some(path) = json_text(arguments, "id")
                .or_else(|| json_text_path(arguments, &["content", "path"]))
                .or_else(|| json_text(arguments, "query"))
            else {
                return unsupported(
                    "resource_rw file read requires `id`, `query`, or `content.path`.",
                );
            };
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "file_read".to_string(),
                arguments: serde_json::json!({ "path": path }),
            }])
        }
        ("file", "list") => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "file_search".to_string(),
            arguments: merge_content_metadata(arguments),
        }]),
        ("file", "delete") => {
            let Some(path) = json_text(arguments, "id")
                .or_else(|| json_text_path(arguments, &["content", "path"]))
                .or_else(|| json_text(arguments, "query"))
            else {
                return unsupported_with_extra(
                    "resource_rw file delete requires `id`, `query`, or `content.path`.",
                    serde_json::json!({
                        "kind": kind,
                        "op": op,
                        "field": "id",
                        "hint": "Use a managed workspace/data-relative file path or resource id. Do not guess container paths."
                    }),
                );
            };
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "file_delete".to_string(),
                arguments: serde_json::json!({ "path": path }),
            }])
        }
        ("file", "create") | ("file", "update") => {
            let payload = merge_content_metadata(arguments);
            let suggested_primitive =
                if payload.get("patch").is_some() || payload.get("patches").is_some() {
                    "file_patch"
                } else {
                    "file_write"
                };
            unsupported_with_extra(
                "resource_rw file create/update no longer accepts file authoring payloads. Use file_write or file_patch directly.",
                serde_json::json!({
                    "kind": kind,
                    "op": op,
                    "terminal_observation": true,
                    "suggested_primitive": suggested_primitive
                }),
            )
        }
        ("app_service", "create")
        | ("app_service", "update")
        | ("dashboard", "create")
        | ("dashboard", "update") => unsupported_with_extra(
            "resource_rw app_service/dashboard no longer accepts generated app source. Use app_deploy for app creation, source replacement, or patches.",
            serde_json::json!({
                "kind": kind,
                "op": op,
                "terminal_observation": true,
                "suggested_primitive": "app_deploy"
            }),
        ),
        ("app_service", _) | ("dashboard", _) => {
            let payload = service_manage_payload_from_resource(arguments);
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "service_manage".to_string(),
                arguments: payload,
            }])
        }
        ("watcher", "create") | ("watcher", "update") => {
            match watcher_payload_from_resource(arguments, op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "watch".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("watcher", "list") | ("watcher", "read") | ("watcher", "status") => {
            match durable_work_manage_payload_from_resource(arguments, kind.as_str(), op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "background_work_manage".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("watcher", "delete")
        | ("watcher", "pause")
        | ("watcher", "resume")
        | ("watcher", "stop")
        | ("watcher", "cancel")
        | ("watcher", "update_delivery") => {
            match durable_work_manage_payload_from_resource(arguments, kind.as_str(), op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "background_work_manage".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("scheduled_task", "create") | ("scheduled_task", "update") => {
            match scheduled_task_payload_from_resource(arguments, op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "schedule_task".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("notification", "create") | ("notification", "update") => {
            match notification_payload_from_resource(arguments, op.as_str()) {
                Ok(action) => PrimitivePlan::Actions(vec![action]),
                Err(plan) => plan,
            }
        }
        ("scheduled_task", "list") | ("scheduled_task", "read") | ("scheduled_task", "status") => {
            match durable_work_manage_payload_from_resource(arguments, kind.as_str(), op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "background_work_manage".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("scheduled_task", "delete")
        | ("scheduled_task", "pause")
        | ("scheduled_task", "resume")
        | ("scheduled_task", "stop")
        | ("scheduled_task", "cancel")
        | ("scheduled_task", "update_delivery") => {
            match durable_work_manage_payload_from_resource(arguments, kind.as_str(), op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "background_work_manage".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("background_session", "create") => unsupported_with_extra(
            "resource_rw background_session create is not a standalone durable-work creation path. Create scheduled tasks for time-based work or watchers for condition/change-based monitoring; AgentArk will attach the created work to a background session.",
            serde_json::json!({
                "kind": kind,
                "op": op,
                "suggested_kinds": ["watcher", "scheduled_task"],
                "hint": "For recurring or conditional monitoring, create the underlying watcher or scheduled task directly. Use background_session only for lifecycle operations on existing background work."
            }),
        ),
        ("background_session", "update") => unsupported_with_extra(
            "resource_rw background_session update is lifecycle-only through specific operations such as status, pause, resume, stop, delete, or update_delivery.",
            serde_json::json!({
                "kind": kind,
                "op": op,
                "supported_ops": supported_resource_ops("background_session"),
            }),
        ),
        ("goal", "create")
        | ("goal", "update")
        | ("goal", "delete")
        | ("goal", "list")
        | ("goal", "read")
        | ("goal", "status") => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "goal_manage".to_string(),
            arguments: goal_resource_payload_from_resource(arguments, op.as_str()),
        }]),
        ("integration", "list") | ("integration", "read") | ("integration", "status") => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(query) = json_text(arguments, "query") {
                if let Some(object) = payload.as_object_mut() {
                    object.insert("query".to_string(), serde_json::Value::String(query));
                }
            }
            let has_specific_target = json_text(arguments, "id").is_some()
                || json_text_path(&payload, &["id"]).is_some()
                || json_text(arguments, "query").is_some();
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: if has_specific_target && op != "list" {
                    "inspect_integration"
                } else {
                    "list_integrations"
                }
                .to_string(),
                arguments: payload,
            }])
        }
        ("integration", "test") => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                object.insert("run_check".to_string(), serde_json::Value::Bool(true));
                object.remove("op");
                object.remove("kind");
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "inspect_integration".to_string(),
                arguments: payload,
            }])
        }
        ("integration", "create")
        | ("integration", "update")
        | ("integration", "install")
        | ("integration", "connect") => plan_integration_mutation(arguments, op.as_str()),
        ("custom_api", "test") => {
            let mut payload = merge_objects(
                serde_json::json!({ "surface": "custom_apis", "run_check": true }),
                merge_content_metadata(arguments),
            );
            if let Some(object) = payload.as_object_mut() {
                object.remove("op");
                object.remove("kind");
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "inspect_integration".to_string(),
                arguments: payload,
            }])
        }
        ("custom_api", "list") | ("custom_api", "read") | ("custom_api", "status") => {
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: if op == "list" {
                    "list_integrations"
                } else {
                    "inspect_integration"
                }
                .to_string(),
                arguments: merge_objects(
                    serde_json::json!({ "surface": "custom_apis" }),
                    merge_content_metadata(arguments),
                ),
            }])
        }
        ("custom_api", _) => unsupported_registered_resource_operation("custom_api", op.as_str()),
        ("custom_messaging_channel", "list")
        | ("custom_messaging_channel", "read")
        | ("custom_messaging_channel", "status") => {
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: if op == "list" {
                    "list_integrations"
                } else {
                    "inspect_integration"
                }
                .to_string(),
                arguments: merge_objects(
                    serde_json::json!({ "surface": "messaging_channels" }),
                    merge_content_metadata(arguments),
                ),
            }])
        }
        ("custom_messaging_channel", _) => {
            unsupported_registered_resource_operation("custom_messaging_channel", op.as_str())
        }
        ("extension_pack", "list") | ("extension_pack", "read") | ("extension_pack", "status") => {
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: if op == "list" {
                    "extension_pack_list"
                } else {
                    "inspect_integration"
                }
                .to_string(),
                arguments: merge_objects(
                    serde_json::json!({ "surface": "extension_packs" }),
                    merge_content_metadata(arguments),
                ),
            }])
        }
        ("extension_pack", _) => {
            unsupported_registered_resource_operation("extension_pack", op.as_str())
        }
        ("skill", "create") | ("skill", "update") | ("skill", "install") | ("skill", "test") => {
            unsupported_with_extra(
                "resource_rw skill no longer accepts skill authoring, import, install, or test payloads. Use skill_manage directly.",
                serde_json::json!({
                    "kind": kind,
                    "op": op,
                    "terminal_observation": true,
                    "suggested_primitive": "skill_manage"
                }),
            )
        }
        ("skill", "delete")
        | ("skill", "list")
        | ("skill", "read")
        | ("skill", "status")
        | ("skill", "enable")
        | ("skill", "disable") => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "manage_actions".to_string(),
            arguments: skill_resource_payload_from_resource(arguments, op.as_str()),
        }]),
        ("skill_marketplace", "create")
        | ("skill_marketplace", "update")
        | ("skill_marketplace", "delete")
        | ("skill_marketplace", "list")
        | ("skill_marketplace", "read")
        | ("skill_marketplace", "status")
        | ("skill_marketplace", "refresh")
        | ("skill_marketplace", "enable")
        | ("skill_marketplace", "disable") => {
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "manage_actions".to_string(),
                arguments: skill_marketplace_payload_from_resource(arguments, op.as_str()),
            }])
        }
        ("mcp_server", "create")
        | ("mcp_server", "update")
        | ("mcp_server", "delete")
        | ("mcp_server", "install")
        | ("mcp_server", "connect")
        | ("mcp_server", "list")
        | ("mcp_server", "read")
        | ("mcp_server", "status")
        | ("mcp_server", "refresh") => {
            unsupported_registered_resource_operation("mcp_server", op.as_str())
        }
        ("background_session", _) => {
            match durable_work_manage_payload_from_resource(arguments, kind.as_str(), op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "background_work_manage".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
        ("browser_profile", "create")
        | ("browser_profile", "read")
        | ("browser_profile", "update")
        | ("browser_profile", "delete")
        | ("browser_profile", "list")
        | ("browser_profile", "status")
        | ("browser_profile", "launch")
        | ("browser_profile", "close")
        | ("browser_profile", "resolve") => {
            let payload = browser_profile_payload_from_resource(arguments, op.as_str());
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "browser_profile_manage".to_string(),
                arguments: payload,
            }])
        }
        ("conversation", "read") | ("conversation", "list") => {
            PrimitivePlan::Conversation(ConversationPrimitiveOp::Read {
                limit: json_usize(arguments, "limit")
                    .or_else(|| json_usize_path(arguments, &["metadata", "limit"])),
            })
        }
        ("activity", "read")
        | ("activity", "list")
        | ("activity", "status")
        | ("activity", "refresh") => {
            let mut payload = serde_json::json!({
                "operation": "surface",
                "surface": "activity",
            });
            if let Some(limit) = json_usize(arguments, "limit")
                .or_else(|| json_usize_path(arguments, &["metadata", "limit"]))
            {
                if let Some(object) = payload.as_object_mut() {
                    object.insert("limit".to_string(), serde_json::json!(limit));
                }
            }
            if let Some(query) = json_text(arguments, "query") {
                if let Some(object) = payload.as_object_mut() {
                    object.insert("query".to_string(), serde_json::json!(query));
                }
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "ark_inspect".to_string(),
                arguments: payload,
            }])
        }
        _ => unsupported_resource_adapter(arguments, kind.as_str(), op.as_str()),
    }
}

fn unsupported_resource_adapter(
    arguments: &serde_json::Value,
    kind: &str,
    op: &str,
) -> PrimitivePlan {
    let mut extra = serde_json::json!({
        "kind": kind,
        "op": op,
        "valid_kinds": resource_valid_kinds(),
        "supported_ops": supported_resource_ops(kind),
    });
    if resource_payload_looks_like_notification(arguments) {
        if let Some(object) = extra.as_object_mut() {
            object.insert(
                "suggested_kind".to_string(),
                serde_json::Value::String("notification".to_string()),
            );
            object.insert(
                "suggested_op".to_string(),
                serde_json::Value::String("create".to_string()),
            );
            object.insert(
                "hint".to_string(),
                serde_json::Value::String(
                    "This payload has a user-facing notification shape. Retry as resource_rw kind=notification with the message/title and delivery route in content.delivery_channel, content.report_to, or content.channel.".to_string(),
                ),
            );
        }
    }
    unsupported_with_extra(
        format!(
            "resource_rw does not yet have a substrate adapter for kind `{}` and op `{}`.",
            kind, op
        ),
        extra,
    )
}

fn plan_registered_resource_action(
    arguments: &serde_json::Value,
    kind: &str,
    op: &str,
) -> Option<PrimitivePlan> {
    let contract = resource_action_contract(kind, op)?;
    let mut payload = transform_resource_payload(arguments, contract.transform, op);
    if !resource_payload_satisfies_requirement(&payload, contract.requirement, op) {
        if resource_payload_looks_like_notification(arguments)
            && contract.kind == "custom_messaging_channel"
            && matches!(op, "create" | "update")
        {
            return Some(
                match notification_payload_from_resource(arguments, "create") {
                    Ok(action) => PrimitivePlan::Actions(vec![action]),
                    Err(plan) => plan,
                },
            );
        }
        return Some(plan_resource_capability_resolution(
            arguments,
            contract.kind,
            op,
            contract.selected_action_for_resolution,
            contract.requirement,
        ));
    }
    if let Some(object) = payload.as_object_mut() {
        if contract.kind == "custom_api"
            && !matches!(op, "update" | "delete" | "enable" | "disable")
        {
            object.remove("id");
        }
        if (contract.kind == "custom_api" && matches!(op, "delete" | "enable" | "disable"))
            || (contract.kind == "custom_messaging_channel"
                && matches!(op, "delete" | "enable" | "disable" | "test"))
        {
            object.insert(
                "operation".to_string(),
                serde_json::Value::String(op.to_string()),
            );
        }
        if contract.kind == "extension_pack" && matches!(op, "enable" | "disable") {
            object.insert(
                "enabled".to_string(),
                serde_json::Value::Bool(op == "enable"),
            );
        }
    }
    Some(PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: contract.action_name.to_string(),
        arguments: payload,
    }]))
}

fn resource_action_contract(kind: &str, op: &str) -> Option<&'static ResourceActionContract> {
    RESOURCE_ACTION_CONTRACTS
        .iter()
        .find(|contract| contract.kind == kind && contract.ops.contains(&op))
}

fn resource_adapter_spec(kind: &str) -> Option<&'static ResourceAdapterSpec> {
    RESOURCE_RW_ADAPTERS
        .iter()
        .find(|adapter| adapter.kind == kind)
}

fn resource_valid_kinds() -> Vec<&'static str> {
    RESOURCE_RW_ADAPTERS
        .iter()
        .map(|adapter| adapter.kind)
        .collect()
}

fn resource_valid_ops() -> Vec<&'static str> {
    let mut ops = RESOURCE_RW_ADAPTERS
        .iter()
        .flat_map(|adapter| adapter.ops.iter().copied())
        .collect::<Vec<_>>();
    ops.sort_unstable();
    ops.dedup();
    ops
}

fn supported_resource_ops(kind: &str) -> Vec<&'static str> {
    resource_adapter_spec(kind)
        .map(|adapter| adapter.ops.to_vec())
        .unwrap_or_default()
}

fn transform_resource_payload(
    arguments: &serde_json::Value,
    transform: ResourcePayloadTransform,
    op: &str,
) -> serde_json::Value {
    match transform {
        ResourcePayloadTransform::StripResourceFields => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                object.remove("op");
                object.remove("kind");
            }
            payload
        }
        ResourcePayloadTransform::ExtensionPack => extension_pack_payload_from_resource(arguments),
        ResourcePayloadTransform::McpManage => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                let operation = match op {
                    "read" => "status",
                    "install" | "connect" => "create",
                    other => other,
                };
                object.insert(
                    "operation".to_string(),
                    serde_json::Value::String(operation.to_string()),
                );
                object.remove("op");
                object.remove("kind");
            }
            payload
        }
    }
}

fn resource_payload_satisfies_requirement(
    payload: &serde_json::Value,
    requirement: ResourcePayloadRequirement,
    op: &str,
) -> bool {
    match requirement {
        ResourcePayloadRequirement::None => true,
        ResourcePayloadRequirement::CustomApiAcquisitionSpec => {
            custom_api_payload_has_acquisition_spec(payload, op)
        }
        ResourcePayloadRequirement::CustomMessagingChannelSpec => {
            json_text(payload, "name").is_some()
                && payload.get("send").is_some_and(|value| value.is_object())
        }
        ResourcePayloadRequirement::ExtensionPackInstallTarget => {
            extension_pack_payload_has_install_target(payload)
        }
        ResourcePayloadRequirement::ExtensionPackId => extension_pack_payload_has_pack_id(payload),
        ResourcePayloadRequirement::McpServerConfig => mcp_server_payload_has_config(payload),
        ResourcePayloadRequirement::ResourceId => json_text(payload, "id")
            .or_else(|| json_text(payload, "pack_id"))
            .is_some(),
    }
}

fn mcp_server_payload_has_config(payload: &serde_json::Value) -> bool {
    json_text(payload, "url").is_some()
        || json_text(payload, "command").is_some()
        || payload
            .get("transport")
            .is_some_and(|value| value.is_object())
}

fn resource_requirement_labels(requirement: ResourcePayloadRequirement) -> &'static [&'static str] {
    match requirement {
        ResourcePayloadRequirement::None => &[],
        ResourcePayloadRequirement::CustomApiAcquisitionSpec => &[
            "name or id",
            "endpoint, OpenAPI, docs, operation shape, or update auth metadata",
        ],
        ResourcePayloadRequirement::CustomMessagingChannelSpec => &["name", "send specification"],
        ResourcePayloadRequirement::ExtensionPackInstallTarget => {
            &["pack id or extension-pack source/manifest"]
        }
        ResourcePayloadRequirement::ExtensionPackId => &["pack id"],
        ResourcePayloadRequirement::McpServerConfig => &["transport, url, or command"],
        ResourcePayloadRequirement::ResourceId => &["resource id"],
    }
}

fn scheduled_task_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> Result<serde_json::Value, PrimitivePlan> {
    let mut payload = merge_content_metadata(arguments);
    merge_top_level_resource_fields(
        arguments,
        &mut payload,
        &[
            "items",
            "message",
            "title",
            "task",
            "task_id",
            "cron",
            "at",
            "scheduled_for",
            "local_date",
            "local_time",
            "timezone",
            "timezone_offset_minutes",
            "date_policy",
            "report_to",
            "delivery_channel",
            "channel",
            "action",
            "action_arguments",
            "script",
            "script_language",
            "context_from",
            "workdir",
            "network_access",
            "validation",
            "max_attempts",
            "stall_timeout_secs",
            "retry_backoff_secs",
            "automation_policy",
        ],
    );
    normalize_scheduled_notification_payload(&mut payload);
    if op == "update" {
        let task_id = json_text(arguments, "id")
            .or_else(|| json_text(&payload, "task_id"))
            .or_else(|| json_text_path(arguments, &["content", "task_id"]));
        let Some(task_id) = task_id else {
            return Err(unsupported_with_extra(
                "resource_rw scheduled_task update requires `id` or `content.task_id`.",
                serde_json::json!({
                    "kind": "scheduled_task",
                    "op": op,
                    "field": "id",
                    "hint": "Use the existing scheduled task id from the task/resource result when changing its time, delivery route, or body."
                }),
            ));
        };
        if let Some(object) = payload.as_object_mut() {
            object.insert("task_id".to_string(), serde_json::Value::String(task_id));
        }
    }
    if let Some(object) = payload.as_object_mut() {
        object.remove("op");
        object.remove("kind");
        object.remove("id");
    }
    Ok(payload)
}

fn notification_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> Result<PrimitiveActionInvocation, PrimitivePlan> {
    let mut payload = merge_content_metadata(arguments);
    merge_top_level_resource_fields(
        arguments,
        &mut payload,
        &[
            "items",
            "message",
            "title",
            "task",
            "task_id",
            "cron",
            "at",
            "scheduled_for",
            "local_date",
            "local_time",
            "timezone",
            "timezone_offset_minutes",
            "date_policy",
            "report_to",
            "delivery_channel",
            "channel",
            "action",
            "action_arguments",
            "allow_duplicate",
            "validation",
            "max_attempts",
            "stall_timeout_secs",
            "retry_backoff_secs",
            "automation_policy",
        ],
    );

    if notification_payload_has_schedule(&payload) {
        normalize_scheduled_notification_payload(&mut payload);
        if op == "update" {
            let task_id = json_text(arguments, "id")
                .or_else(|| json_text(&payload, "task_id"))
                .or_else(|| json_text_path(arguments, &["content", "task_id"]));
            let Some(task_id) = task_id else {
                return Err(unsupported_with_extra(
                    "resource_rw notification update requires `id` or `content.task_id` when updating a scheduled notification.",
                    serde_json::json!({
                        "kind": "notification",
                        "op": op,
                        "field": "id",
                    }),
                ));
            };
            if let Some(object) = payload.as_object_mut() {
                object.insert("task_id".to_string(), serde_json::Value::String(task_id));
            }
        }
        if let Some(object) = payload.as_object_mut() {
            object.remove("op");
            object.remove("kind");
            object.remove("id");
            object.remove("delivery_channel");
            object.remove("channel");
            object.remove("message");
            object.remove("title");
        }
        return Ok(PrimitiveActionInvocation {
            action_name: "schedule_task".to_string(),
            arguments: payload,
        });
    }

    normalize_direct_notification_payload(&mut payload)?;
    if let Some(object) = payload.as_object_mut() {
        object.remove("op");
        object.remove("kind");
        object.remove("id");
        object.remove("query");
        object.remove("report_to");
        object.remove("channel");
        object.remove("task");
    }
    Ok(PrimitiveActionInvocation {
        action_name: "notify_user".to_string(),
        arguments: payload,
    })
}

fn resource_payload_looks_like_notification(arguments: &serde_json::Value) -> bool {
    let payload = merge_content_metadata(arguments);
    if payload.get("send").is_some_and(|value| value.is_object()) {
        return false;
    }

    let has_message_body = ["message", "title", "task"]
        .iter()
        .any(|key| json_text(&payload, key).is_some())
        || json_text_path(&payload, &["action_arguments", "message"]).is_some();
    let has_delivery_route = ["delivery_channel", "report_to", "channel", "recipient"]
        .iter()
        .any(|key| json_text(&payload, key).is_some());

    has_message_body && (has_delivery_route || notification_payload_has_schedule(&payload))
}

fn notification_payload_has_schedule(payload: &serde_json::Value) -> bool {
    ["cron", "at", "scheduled_for", "local_time"]
        .iter()
        .any(|key| json_text(payload, key).is_some())
        || payload
            .get("items")
            .and_then(|value| value.as_array())
            .is_some_and(|items| {
                items.iter().any(|item| {
                    ["cron", "at", "scheduled_for", "local_time"]
                        .iter()
                        .any(|key| json_text(item, key).is_some())
                })
            })
}

fn normalize_scheduled_notification_payload(payload: &mut serde_json::Value) {
    if let Some(items) = payload
        .get_mut("items")
        .and_then(|value| value.as_array_mut())
    {
        for item in items {
            normalize_single_scheduled_notification_payload(item);
        }
    }
    normalize_single_scheduled_notification_payload(payload);
}

fn normalize_single_scheduled_notification_payload(payload: &mut serde_json::Value) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };

    let delivery_channel = ["delivery_channel", "channel"].iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    if let Some(delivery_channel) = delivery_channel {
        object
            .entry("report_to".to_string())
            .or_insert(serde_json::Value::String(delivery_channel));
    }

    let message = object
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            object
                .get("action_arguments")
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            object
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    if let Some(message) = message {
        let top_level_notification_body = object
            .get("message")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || object
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
        object
            .entry("task".to_string())
            .or_insert_with(|| serde_json::Value::String(message.clone()));
        let action_is_notify_user = object
            .get("action")
            .and_then(|value| value.as_str())
            .is_none_or(|value| value.trim().eq_ignore_ascii_case("notify_user"));
        if top_level_notification_body || action_is_notify_user {
            object.insert(
                "action".to_string(),
                serde_json::Value::String("notify_user".to_string()),
            );
            let action_arguments = object
                .entry("action_arguments".to_string())
                .or_insert_with(|| serde_json::json!({}));
            if !action_arguments.is_object() {
                *action_arguments = serde_json::json!({});
            }
            if let Some(args_object) = action_arguments.as_object_mut() {
                args_object
                    .entry("message".to_string())
                    .or_insert_with(|| serde_json::Value::String(message));
                args_object
                    .entry("source".to_string())
                    .or_insert_with(|| serde_json::Value::String("reminder".to_string()));
                args_object
                    .entry("in_app_title".to_string())
                    .or_insert_with(|| serde_json::Value::String("Reminder".to_string()));
            }
        }
    }
}

fn normalize_direct_notification_payload(
    payload: &mut serde_json::Value,
) -> Result<(), PrimitivePlan> {
    let Some(object) = payload.as_object_mut() else {
        return Err(unsupported_with_extra(
            "resource_rw notification create requires a notification payload object.",
            serde_json::json!({
                "kind": "notification",
                "field": "content",
            }),
        ));
    };

    let delivery_channel = ["report_to", "channel"].iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    if let Some(delivery_channel) = delivery_channel {
        object
            .entry("delivery_channel".to_string())
            .or_insert(serde_json::Value::String(delivery_channel));
    }

    let message = object
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            object
                .get("task")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            object
                .get("query")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            object
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    let Some(message) = message else {
        return Err(unsupported_with_extra(
            "resource_rw notification create requires `content.message`.",
            serde_json::json!({
                "kind": "notification",
                "field": "content.message",
            }),
        ));
    };
    object
        .entry("message".to_string())
        .or_insert_with(|| serde_json::Value::String(message));
    Ok(())
}

fn merge_top_level_resource_fields(
    arguments: &serde_json::Value,
    payload: &mut serde_json::Value,
    keys: &[&str],
) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    for key in keys {
        if let Some(value) = arguments.get(*key) {
            object
                .entry((*key).to_string())
                .or_insert_with(|| value.clone());
        }
    }
}

fn goal_resource_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> serde_json::Value {
    let mut payload = merge_content_metadata(arguments);
    if let Some(object) = payload.as_object_mut() {
        let operation = match op {
            "read" | "status" => "report",
            other => other,
        };
        object.insert(
            "operation".to_string(),
            serde_json::Value::String(operation.to_string()),
        );
        if !object.contains_key("goal_id") {
            if let Some(id) = json_text(arguments, "id") {
                object.insert("goal_id".to_string(), serde_json::Value::String(id));
            }
        }
        if !object.contains_key("goal") {
            if let Some(query) = json_text(arguments, "query") {
                object.insert("goal".to_string(), serde_json::Value::String(query));
            }
        }
        object.remove("op");
        object.remove("kind");
        object.remove("id");
    }
    payload
}

fn watcher_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> Result<serde_json::Value, PrimitivePlan> {
    let mut payload = merge_content_metadata(arguments);
    if op == "update" {
        let watcher_id = json_text(arguments, "id")
            .or_else(|| json_text(&payload, "watcher_id"))
            .or_else(|| json_text_path(arguments, &["content", "watcher_id"]));
        let Some(watcher_id) = watcher_id else {
            return Err(unsupported_with_extra(
                "resource_rw watcher update requires `id` or `content.watcher_id`.",
                serde_json::json!({
                    "kind": "watcher",
                    "op": op,
                    "field": "id",
                    "hint": "Use the existing watcher id from the watcher/resource result when changing its target, cadence, condition, or delivery route."
                }),
            ));
        };
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "watcher_id".to_string(),
                serde_json::Value::String(watcher_id),
            );
        }
    }
    if let Some(object) = payload.as_object_mut() {
        object.remove("op");
        object.remove("kind");
        object.remove("id");
    }
    Ok(payload)
}

fn durable_work_manage_payload_from_resource(
    arguments: &serde_json::Value,
    kind: &str,
    op: &str,
) -> Result<serde_json::Value, PrimitivePlan> {
    let mut payload = merge_content_metadata(arguments);
    let operation = match op {
        "read" => "status",
        other => other,
    };
    let query = json_text(arguments, "query");
    let report_to = json_text(&payload, "report_to");
    let id = json_text(arguments, "id").or_else(|| json_text(&payload, "work_id"));
    let id_field = match kind {
        "scheduled_task" => "task_id",
        "watcher" => "watcher_id",
        "background_session" => "background_session_id",
        _ => "work_id",
    };

    let Some(object) = payload.as_object_mut() else {
        return Ok(payload);
    };
    object.insert(
        "operation".to_string(),
        serde_json::Value::String(operation.to_string()),
    );
    object.insert(
        "kind".to_string(),
        serde_json::Value::String(kind.to_string()),
    );
    object.remove("op");

    if let Some(query) = query {
        object
            .entry("reference_text".to_string())
            .or_insert(serde_json::Value::String(query));
    }
    if let Some(report_to) = report_to {
        object
            .entry("delivery_channel".to_string())
            .or_insert(serde_json::Value::String(report_to));
    }
    if let Some(id) = id {
        object.insert(id_field.to_string(), serde_json::Value::String(id));
    }
    object.remove("id");

    let has_identifier = object
        .get(id_field)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || object
            .get("reference_text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
    if !matches!(operation, "list") && !has_identifier {
        return Err(unsupported_with_extra(
            format!("resource_rw {} {} requires `id` or `query`.", kind, op),
            serde_json::json!({
                "kind": kind,
                "op": op,
                "field": "id",
                "hint": "Use the durable resource id returned when the work was created, or a semantic query only for status-style lookup."
            }),
        ));
    }

    Ok(payload)
}

fn unsupported_registered_resource_operation(kind: &str, op: &str) -> PrimitivePlan {
    let supported_ops = supported_resource_ops(kind);
    unsupported_resource_operation(kind, op, &supported_ops)
}

fn unsupported_resource_operation(kind: &str, op: &str, supported_ops: &[&str]) -> PrimitivePlan {
    unsupported_with_extra(
        format!(
            "resource_rw kind `{}` does not support op `{}` through this substrate.",
            kind, op
        ),
        serde_json::json!({
            "kind": kind,
            "op": op,
            "supported_ops": supported_ops,
            "terminal_observation": true,
            "hint": "Do not retry the same unsupported operation. Use one of the supported operations or explain the blocker."
        }),
    )
}
fn skill_resource_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> serde_json::Value {
    let mut payload = merge_content_metadata(arguments);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "resource".to_string(),
            serde_json::Value::String("skill".to_string()),
        );
        object.insert(
            "operation".to_string(),
            serde_json::Value::String(if op == "install" {
                "import".to_string()
            } else {
                op.to_string()
            }),
        );
        object.remove("op");
        object.remove("kind");
        if !object.contains_key("name") {
            if let Some(target) = object
                .get("id")
                .or_else(|| object.get("query"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            {
                object.insert("name".to_string(), serde_json::Value::String(target));
            }
        }
        if !object.contains_key("content") {
            if let Some(markdown) = object.get("markdown").cloned() {
                object.insert("content".to_string(), markdown);
            }
        }
    }
    payload
}

fn skill_marketplace_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> serde_json::Value {
    let mut payload = merge_content_metadata(arguments);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "resource".to_string(),
            serde_json::Value::String("skill_marketplace".to_string()),
        );
        object.insert(
            "operation".to_string(),
            serde_json::Value::String(op.to_string()),
        );
        object.remove("op");
        object.remove("kind");
        if !object.contains_key("id") {
            if let Some(target) = object
                .get("query")
                .or_else(|| object.get("name"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            {
                object.insert("id".to_string(), serde_json::Value::String(target));
            }
        }
    }
    payload
}

fn plan_integration_mutation(arguments: &serde_json::Value, op: &str) -> PrimitivePlan {
    let mut payload = merge_content_metadata(arguments);
    if let Some(object) = payload.as_object_mut() {
        object.remove("op");
        object.remove("kind");
    }

    if payload.get("send").is_some() {
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "custom_messaging_channel_upsert".to_string(),
            arguments: payload,
        }]);
    }

    if payload.get("pack_id").is_some()
        || payload.get("source_url").is_some()
        || payload.get("source_path").is_some()
        || payload.get("manifest_text").is_some()
        || payload.get("manifest").is_some()
    {
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: if op == "connect" {
                "extension_pack_connect"
            } else {
                "extension_pack_install"
            }
            .to_string(),
            arguments: payload,
        }]);
    }

    if payload.get("base_url").is_some()
        || payload.get("path").is_some()
        || payload.get("openapi_url").is_some()
        || payload.get("openapi_text").is_some()
        || payload.get("docs_url").is_some()
        || payload.get("docs_text").is_some()
    {
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "capability_acquire".to_string(),
            arguments: payload,
        }]);
    }

    plan_resource_capability_resolution(
        arguments,
        "integration",
        op,
        None,
        ResourcePayloadRequirement::None,
    )
}

fn plan_resource_capability_resolution(
    arguments: &serde_json::Value,
    kind: &str,
    op: &str,
    selected_action: Option<&str>,
    requirement: ResourcePayloadRequirement,
) -> PrimitivePlan {
    let requested_capability = resource_resolution_target(arguments)
        .unwrap_or_else(|| format!("{} {}", kind.replace('_', " "), op));
    let goal = format!(
        "Resolve how to {} {} resource `{}` and identify the correct AgentArk capability route before mutating state.",
        op,
        kind.replace('_', " "),
        requested_capability
    );
    let failure_output = serde_json::json!({
        "status": "needs_capability_resolution",
        "reason": "missing structured resource fields",
        "kind": kind,
        "op": op,
        "required_fields": resource_requirement_labels(requirement),
        "provided": resource_payload_shape(arguments),
        "policy": "discover_or_inspect_first_then_mutate"
    })
    .to_string();
    let mut payload = serde_json::json!({
        "goal": goal,
        "requested_capability": requested_capability,
        "failure_output": failure_output,
    });
    if let (Some(action), Some(object)) = (selected_action, payload.as_object_mut()) {
        object.insert(
            "selected_action".to_string(),
            serde_json::Value::String(action.to_string()),
        );
    }

    PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
        action_name: "capability_resolve".to_string(),
        arguments: payload,
    }])
}

fn resource_resolution_target(arguments: &serde_json::Value) -> Option<String> {
    json_text(arguments, "query")
        .or_else(|| json_text(arguments, "id"))
        .or_else(|| json_text_path(arguments, &["content", "name"]))
        .or_else(|| json_text_path(arguments, &["content", "id"]))
        .or_else(|| json_text_path(arguments, &["content", "pack_id"]))
        .or_else(|| json_text_path(arguments, &["content", "base_url"]))
        .or_else(|| json_text_path(arguments, &["content", "docs_url"]))
        .or_else(|| json_text_path(arguments, &["content", "openapi_url"]))
}

fn resource_payload_shape(arguments: &serde_json::Value) -> serde_json::Value {
    let payload = merge_content_metadata(arguments);
    let Some(object) = payload.as_object() else {
        return serde_json::json!({ "keys": [] });
    };
    let mut keys = object
        .keys()
        .filter(|key| key.as_str() != "op" && key.as_str() != "kind")
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    serde_json::json!({ "keys": keys })
}

fn custom_api_payload_has_acquisition_spec(payload: &serde_json::Value, op: &str) -> bool {
    if custom_api_payload_has_endpoint_or_source(payload) {
        return true;
    }
    let has_target_id = json_text(payload, "id").is_some();
    if op == "update" && has_target_id {
        return custom_api_payload_has_operation_detail(payload)
            || custom_api_payload_has_auth_metadata(payload)
            || json_text(payload, "description").is_some()
            || json_text(payload, "name").is_some();
    }
    false
}

fn custom_api_payload_has_endpoint_or_source(payload: &serde_json::Value) -> bool {
    json_text(payload, "base_url").is_some()
        || crate::core::request_contract::has_source_alias(payload)
        || json_text(payload, "path")
            .as_deref()
            .is_some_and(is_absolute_http_url)
        || payload
            .get("operation")
            .and_then(|value| value.as_object())
            .is_some_and(custom_api_operation_has_endpoint_or_source)
        || payload
            .get("operations")
            .and_then(|value| value.as_array())
            .is_some_and(|operations| {
                operations.iter().any(|operation| {
                    operation
                        .as_object()
                        .is_some_and(custom_api_operation_has_endpoint_or_source)
                })
            })
}

fn custom_api_operation_has_endpoint_or_source(
    operation: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    operation
        .get("base_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || crate::core::request_contract::has_source_alias(&serde_json::Value::Object(
            operation.clone(),
        ))
        || operation
            .get("url")
            .or_else(|| operation.get("path"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(is_absolute_http_url)
}

fn custom_api_payload_has_operation_detail(payload: &serde_json::Value) -> bool {
    [
        "path",
        "method",
        "operation",
        "operations",
        "default_headers",
        "headers",
        "default_query",
        "required_inputs",
        "read_only",
        "body_required",
        "body",
        "body_template",
        "parameters",
        "response_notes",
    ]
    .iter()
    .any(|key| payload.get(*key).is_some())
}

fn custom_api_payload_has_auth_metadata(payload: &serde_json::Value) -> bool {
    [
        "auth",
        "auth_type",
        "auth_mode",
        "auth_header",
        "auth_header_name",
        "auth_name",
        "auth_username",
        "auth_profile_id",
        "clear_secret",
    ]
    .iter()
    .any(|key| payload.get(*key).is_some())
}

fn extension_pack_payload_from_resource(arguments: &serde_json::Value) -> serde_json::Value {
    let mut payload = merge_content_metadata(arguments);
    if let Some(object) = payload.as_object_mut() {
        if !object.contains_key("pack_id") {
            if let Some(id) = object
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            {
                object.insert("pack_id".to_string(), serde_json::Value::String(id));
            }
        }
        object.remove("op");
        object.remove("kind");
    }
    payload
}

fn extension_pack_payload_has_pack_id(payload: &serde_json::Value) -> bool {
    json_text(payload, "pack_id").is_some()
}

fn extension_pack_payload_has_install_target(payload: &serde_json::Value) -> bool {
    extension_pack_payload_has_pack_id(payload)
        || ["source_url", "source_path", "manifest_text"]
            .iter()
            .any(|key| json_text(payload, key).is_some())
        || payload
            .get("manifest")
            .is_some_and(|value| value.is_object())
}

fn is_absolute_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn plan_memory_rw_for_caller(
    arguments: &serde_json::Value,
    caller_kind: CallerKind,
) -> PrimitivePlan {
    let op = json_text(arguments, "op")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if memory_mutations_deferred_for_caller(caller_kind)
        && matches!(op.as_str(), "write" | "update" | "delete")
    {
        return PrimitivePlan::Memory(MemoryPrimitiveOp::DeferredMutation {
            op,
            intent_summary: json_text(arguments, "intent_summary")
                .or_else(|| json_text_path(arguments, &["metadata", "reason"])),
        });
    }
    match op.as_str() {
        "search" | "read" => {
            let query = json_text(arguments, "query")
                .or_else(|| json_text(arguments, "id"))
                .unwrap_or_default();
            if query.trim().is_empty() {
                return unsupported("memory_rw search/read requires `query` or `id`.");
            }
            PrimitivePlan::Memory(MemoryPrimitiveOp::Search {
                query,
                limit: json_usize(arguments, "limit"),
            })
        }
        "write" | "update" => {
            if !json_bool(arguments, "explicit_user_request").unwrap_or(false) {
                return unsupported(
                    "memory_rw write/update requires active user intent to manage saved memory; incidental user-provided information is handled by background memory capture.",
                );
            }
            let Some(key) = json_text_path(arguments, &["content", "key"])
                .or_else(|| json_text(arguments, "id"))
            else {
                return unsupported("memory_rw write/update requires `content.key` or `id`.");
            };
            let Some(value) = json_text_path(arguments, &["content", "value"])
                .or_else(|| json_text_path(arguments, &["content", "text"]))
            else {
                return unsupported(
                    "memory_rw write/update requires `content.value` or `content.text`.",
                );
            };
            PrimitivePlan::Memory(MemoryPrimitiveOp::Write {
                key,
                value,
                kind: json_text_path(arguments, &["metadata", "kind"])
                    .or_else(|| json_text_path(arguments, &["content", "kind"])),
                scope: json_text_path(arguments, &["metadata", "scope"]),
                slot_cardinality: json_text_path(arguments, &["metadata", "slot_cardinality"])
                    .or_else(|| json_text_path(arguments, &["content", "slot_cardinality"])),
                confidence: json_f32_path(arguments, &["metadata", "confidence"]),
                reason: json_text_path(arguments, &["metadata", "reason"]),
                intent_summary: json_text(arguments, "intent_summary"),
            })
        }
        "delete" => {
            if !json_bool(arguments, "explicit_user_request").unwrap_or(false) {
                return unsupported(
                    "memory_rw delete requires active user intent to manage saved memory; incidental user-provided information is handled by background memory capture.",
                );
            }
            let Some(key) = json_text(arguments, "id")
                .or_else(|| json_text(arguments, "query"))
                .or_else(|| json_text_path(arguments, &["content", "key"]))
            else {
                return unsupported("memory_rw delete requires `id`, `query`, or `content.key`.");
            };
            PrimitivePlan::Memory(MemoryPrimitiveOp::Delete {
                key,
                kind: json_text_path(arguments, &["metadata", "kind"]),
                scope: json_text_path(arguments, &["metadata", "scope"]),
                reason: json_text_path(arguments, &["metadata", "reason"]),
                intent_summary: json_text(arguments, "intent_summary"),
            })
        }
        _ => unsupported("memory_rw requires op search, read, write, update, or delete."),
    }
}

fn unsupported(reason: impl Into<String>) -> PrimitivePlan {
    PrimitivePlan::Unsupported {
        reason: reason.into(),
        extra: None,
    }
}

fn unsupported_with_extra(reason: impl Into<String>, extra: serde_json::Value) -> PrimitivePlan {
    PrimitivePlan::Unsupported {
        reason: reason.into(),
        extra: Some(extra),
    }
}

fn json_text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn json_text_path(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn json_usize(value: &serde_json::Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
}

fn json_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|value| value.as_bool())
}

fn json_usize_path(value: &serde_json::Value, path: &[&str]) -> Option<usize> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
}

fn json_f32_path(value: &serde_json::Value, path: &[&str]) -> Option<f32> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_f64().map(|value| value as f32)
}

fn canonical_json_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
        }
        serde_json::Value::Array(items) => {
            let values = items
                .iter()
                .map(canonical_json_string)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{}]", values)
        }
        serde_json::Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let values = keys
                .into_iter()
                .filter_map(|key| {
                    object.get(key).map(|value| {
                        format!(
                            "{}:{}",
                            serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                            canonical_json_string(value)
                        )
                    })
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{}}}", values)
        }
    }
}

fn merge_content_metadata(arguments: &serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    if let Some(object) = arguments.get("content").and_then(|value| value.as_object()) {
        for (key, value) in object {
            out.insert(key.clone(), value.clone());
        }
    }
    if let Some(object) = arguments
        .get("metadata")
        .and_then(|value| value.as_object())
    {
        for (key, value) in object {
            out.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    for key in [
        "op",
        "kind",
        "id",
        "query",
        "url",
        "method",
        "headers",
        "operation",
        "operation_id",
        "action_name",
        "body",
        "arguments",
        "integration",
        "timeout_secs",
        "save_to",
        "as_resource",
        "suggested_name",
        "persist_response",
        "duplicate_policy",
        "allow_duplicate",
    ] {
        if let Some(value) = arguments.get(key) {
            out.entry(key.to_string()).or_insert_with(|| value.clone());
        }
    }
    serde_json::Value::Object(out)
}

fn browser_profile_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> serde_json::Value {
    let mut out = serde_json::Map::new();

    if let Some(object) = arguments.get("content").and_then(|value| value.as_object()) {
        for (key, value) in object {
            if skip_browser_profile_resource_wrapper_key(key) {
                continue;
            }
            out.insert(key.clone(), value.clone());
        }
    }
    if let Some(metadata) = arguments.get("metadata").cloned() {
        out.entry("metadata".to_string()).or_insert(metadata);
    }
    if let Some(object) = arguments.as_object() {
        for (key, value) in object {
            if skip_browser_profile_resource_wrapper_key(key) {
                continue;
            }
            out.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    out.insert(
        "operation".to_string(),
        serde_json::Value::String(op.to_string()),
    );
    serde_json::Value::Object(out)
}

fn skip_browser_profile_resource_wrapper_key(key: &str) -> bool {
    matches!(key, "kind" | "op" | "content" | "metadata")
        || key.starts_with('_')
        || is_sensitive_tool_call_argument_key(key)
}

fn service_manage_payload_from_resource(arguments: &serde_json::Value) -> serde_json::Value {
    let mut payload = merge_content_metadata(arguments);
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    if let Some(metadata) = arguments.get("metadata").cloned() {
        object.entry("metadata".to_string()).or_insert(metadata);
    }
    if let Some(identity) = arguments
        .get("metadata")
        .and_then(|metadata| metadata.get("artifact_identity"))
        .or_else(|| arguments.get("artifact_identity"))
        .cloned()
    {
        object
            .entry("artifact_identity".to_string())
            .or_insert(identity);
    }

    let operation = object
        .get("operation")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            object
                .get("op")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| match value.to_ascii_lowercase().as_str() {
                    "read" => "status".to_string(),
                    "pause" => "stop".to_string(),
                    "resume" => "start".to_string(),
                    other => other.to_string(),
                })
        })
        .unwrap_or_else(|| "status".to_string());
    object.insert("operation".to_string(), serde_json::json!(operation));
    object.remove("op");

    if let Some(id) = object.get("id").cloned() {
        object.entry("service_id".to_string()).or_insert(id);
    }

    if object
        .get("kind")
        .and_then(|value| value.as_str())
        .map(|value| matches!(value.trim(), "app_service" | "dashboard"))
        .unwrap_or(false)
    {
        object.insert("kind".to_string(), serde_json::json!("auto"));
    }

    normalize_service_manage_files_payload(object);

    payload
}

fn normalize_service_manage_files_payload(object: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(files) = object.get("files").and_then(|value| value.as_object()) {
        let mut normalized = files
            .iter()
            .filter_map(|(path, value)| file_body_text_from_value(value).map(|body| (path, body)))
            .map(|(path, body)| (path.clone(), serde_json::Value::String(body)))
            .collect::<serde_json::Map<_, _>>();
        if !service_files_have_browser_entry(&normalized) {
            if let Some(body) = single_html_document_body(&normalized) {
                normalized.insert("index.html".to_string(), serde_json::Value::String(body));
            }
        }
        if !normalized.is_empty() {
            object.insert("files".to_string(), serde_json::Value::Object(normalized));
        }
        return;
    }

    if object
        .get("source_dir")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || object
            .get("repo_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    {
        return;
    }

    if let Some(body) = object
        .get("content")
        .and_then(file_body_text_from_value)
        .filter(|body| text_looks_like_html_document(body))
        .or_else(|| single_html_document_body(object))
    {
        let mut files = serde_json::Map::new();
        files.insert("index.html".to_string(), serde_json::Value::String(body));
        object.insert("files".to_string(), serde_json::Value::Object(files));
    }
}

fn file_body_text_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(object) => object
            .get("content")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        _ => None,
    }
}

fn text_looks_like_html_document(body: &str) -> bool {
    let trimmed = body.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || (lower.contains("<html") && (lower.contains("<head") || lower.contains("<body")))
}

fn file_path_is_html_entry(path: &str) -> bool {
    let normalized = path.trim().to_ascii_lowercase();
    normalized.ends_with(".html") || normalized.ends_with(".htm")
}

fn service_files_have_browser_entry(files: &serde_json::Map<String, serde_json::Value>) -> bool {
    files.iter().any(|(path, value)| {
        file_path_is_html_entry(path) && value.as_str().is_some_and(text_looks_like_html_document)
    })
}

fn single_html_document_body(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let mut matches = object
        .values()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|body| text_looks_like_html_document(body))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
}

fn merge_objects(left: serde_json::Value, right: serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    if let Some(object) = left.as_object() {
        out.extend(
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    if let Some(object) = right.as_object() {
        out.extend(
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    serde_json::Value::Object(out)
}

fn tool_result_error(
    primitive: &str,
    error: impl Into<String>,
    message: impl Into<String>,
) -> serde_json::Value {
    tool_result_error_with_extra(primitive, error, message, serde_json::json!({}))
}

fn tool_result_error_with_extra(
    primitive: &str,
    error: impl Into<String>,
    message: impl Into<String>,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("ok".to_string(), serde_json::Value::Bool(false));
    object.insert(
        "primitive".to_string(),
        serde_json::Value::String(primitive.to_string()),
    );
    object.insert("error".to_string(), serde_json::Value::String(error.into()));
    object.insert(
        "message".to_string(),
        serde_json::Value::String(message.into()),
    );
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(object)
}

#[cfg(test)]
fn spine_tool_result_value_for_model(
    primitive: &str,
    action_name: &str,
    content: String,
) -> serde_json::Value {
    spine_tool_result_output_for_model(primitive, action_name, content).value
}

#[cfg(test)]
fn spine_tool_result_output_for_model(
    primitive: &str,
    action_name: &str,
    content: String,
) -> crate::core::agent::ark_distill::ArkDistillOutput {
    let profile = crate::core::agent::ark_distill::ArkDistillProfile::default();
    spine_tool_result_output_for_model_with_profile(primitive, action_name, content, &profile)
}

fn spine_tool_result_output_for_model_with_profile(
    primitive: &str,
    action_name: &str,
    content: String,
    arkdistill_profile: &crate::core::agent::ark_distill::ArkDistillProfile,
) -> crate::core::agent::ark_distill::ArkDistillOutput {
    let value = spine_raw_tool_result_value_for_model(primitive, action_name, content);
    crate::core::agent::ark_distill::distill_tool_output_for_model(
        primitive,
        action_name,
        value,
        arkdistill_profile,
    )
}

fn spine_raw_tool_result_value_for_model(
    primitive: &str,
    action_name: &str,
    content: String,
) -> serde_json::Value {
    if action_name == "service_manage" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) {
            return sanitize_service_manage_result_for_model(primitive, &value);
        }
    }
    if action_name == "app_deploy" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) {
            return sanitize_app_deploy_result_for_model(primitive, &value);
        }
    }

    if let Some(value) = parse_structured_tool_completion_for_model(&content) {
        if action_name == "service_manage" {
            let data = value.get("data").unwrap_or(&value);
            return sanitize_service_manage_result_for_model(primitive, data);
        }
        if action_name == "app_deploy" {
            let data = value.get("data").unwrap_or(&value);
            return sanitize_app_deploy_result_for_model(primitive, data);
        }
        if matches!(action_name, "file_write" | "pdf_generate") {
            return sanitize_managed_file_result_for_model(primitive, action_name, &value);
        }
        return sanitize_structured_tool_result_for_model(primitive, action_name, &value);
    }

    serde_json::json!({
        "ok": true,
        "primitive": primitive,
        "content": content,
    })
}

async fn log_arkdistill_tool_output(
    cx: &SpineContext,
    primitive: &str,
    action_name: &str,
    stats: &crate::core::agent::ark_distill::ArkDistillStats,
) {
    if stats.saved_chars == 0 && stats.transformed_fields.is_empty() {
        return;
    }
    let cost_saved =
        estimate_arkdistill_prompt_cost_saved_usd(&cx.agent, stats.estimated_saved_tokens);
    let model_context = arkdistill_model_pricing_context(&cx.agent);
    let mut payload = serde_json::to_value(stats).unwrap_or_else(|_| serde_json::json!({}));
    if let serde_json::Value::Object(object) = &mut payload {
        object.insert(
            "trace_kind".to_string(),
            serde_json::Value::String("arkdistill_telemetry".to_string()),
        );
        object.insert(
            "primitive".to_string(),
            serde_json::Value::String(primitive.to_string()),
        );
        object.insert(
            "action".to_string(),
            serde_json::Value::String(action_name.to_string()),
        );
        if let Some(cost_saved) = cost_saved {
            object.insert(
                "estimated_prompt_cost_saved_usd".to_string(),
                serde_json::json!(cost_saved),
            );
        }
        if let Some((provider, model)) = model_context {
            object.insert(
                "model_provider".to_string(),
                serde_json::Value::String(provider),
            );
            object.insert("model".to_string(), serde_json::Value::String(model));
        }
    }
    cx.emit(SpineTraceEvent::ArkDistillTelemetry {
        data: payload.clone(),
    })
    .await;
    cx.agent
        .log_operational_event(OperationalEvent {
            event_type: crate::core::agent::ark_distill::ARKDISTILL_EVENT_TYPE,
            channel: &cx.request.channel,
            success: true,
            outcome: "distilled",
            trace_id: None,
            conversation_id: cx.request.conversation_id.as_deref(),
            tool_name: Some(primitive),
            latency_ms: None,
            arguments: None,
            payload: Some(&payload),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            specialist_prompt_version: None,
            model_slot: Some(&cx.agent.primary_model_id),
        })
        .await;
}

fn arkdistill_model_pricing_context(agent: &Agent) -> Option<(String, String)> {
    let (slot, _) = agent.model_pool.get(&agent.primary_model_id)?;
    match &slot.provider {
        LlmProvider::OpenAI {
            model, base_url, ..
        } => Some((
            crate::core::model::llm_provider::openai_provider_label(base_url.as_deref())
                .to_string(),
            model.trim().to_string(),
        )),
        LlmProvider::Ollama { model, .. } => Some(("ollama".to_string(), model.trim().to_string())),
        LlmProvider::Anthropic { model, .. } => {
            Some(("anthropic".to_string(), model.trim().to_string()))
        }
    }
    .filter(|(_, model)| !model.trim().is_empty())
}

fn estimate_arkdistill_prompt_cost_saved_usd(agent: &Agent, saved_tokens: usize) -> Option<f64> {
    if saved_tokens == 0 {
        return None;
    }
    let (slot, _) = agent.model_pool.get(&agent.primary_model_id)?;
    match &slot.provider {
        LlmProvider::OpenAI {
            model, base_url, ..
        } => {
            let provider =
                crate::core::model::llm_provider::openai_provider_label(base_url.as_deref());
            let cost = estimate_cost_usd(provider, model, saved_tokens as u64, 0);
            (cost > 0.0).then_some(cost)
        }
        LlmProvider::Ollama { .. } => Some(0.0),
        LlmProvider::Anthropic { .. } => None,
    }
}

fn sanitize_structured_tool_result_for_model(
    primitive: &str,
    action_name: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    let mut out = value.clone();
    if let Some(object) = out.as_object_mut() {
        object.entry("ok".to_string()).or_insert_with(|| {
            let ok = super::tool_responses::structured_tool_value_outcome(value)
                .map(|outcome| {
                    matches!(
                        outcome.state,
                        super::tool_responses::StructuredToolOutcomeState::Success
                    )
                })
                .unwrap_or(true);
            serde_json::Value::Bool(ok)
        });
        object
            .entry("primitive".to_string())
            .or_insert_with(|| serde_json::Value::String(primitive.to_string()));
        object
            .entry("tool".to_string())
            .or_insert_with(|| serde_json::Value::String(action_name.to_string()));
    }
    out
}

fn parse_structured_tool_completion_for_model(content: &str) -> Option<serde_json::Value> {
    content
        .trim_start()
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
        .and_then(|payload| {
            serde_json::from_str::<serde_json::Value>(
                payload.lines().next().unwrap_or(payload).trim(),
            )
            .ok()
        })
}

fn sanitize_app_deploy_result_for_model(
    primitive: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    let mut out = sanitize_service_manage_result_for_model(primitive, value);
    if let Some(object) = out.as_object_mut() {
        object.insert(
            "tool".to_string(),
            serde_json::Value::String("app_deploy".to_string()),
        );
    }
    out
}

fn sanitize_service_manage_result_for_model(
    primitive: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    if value.get("service").is_some()
        || value.get("services").is_some()
        || json_value_text(value, "status").as_deref() == Some("not_found")
    {
        return sanitize_service_manage_lifecycle_result_for_model(primitive, value);
    }

    let app_id = json_value_text(value, "app_id");
    let title = json_value_text(value, "title");
    let url = json_value_text(value, "access_url").or_else(|| json_value_text(value, "url"));
    let status = json_value_text(value, "status").unwrap_or_else(|| "completed".to_string());
    let app_type = json_value_text(value, "type");
    let quality_check = value
        .get("quality_check")
        .map(sanitize_app_quality_check_for_model)
        .unwrap_or(serde_json::Value::Null);
    let duplicate_skipped = value
        .get("duplicate_skipped")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || status == "duplicate_skipped";
    let app = serde_json::json!({
        "id": app_id,
        "title": title,
        "url": url,
        "type": app_type,
        "updated_existing": value.get("updated_existing").and_then(|value| value.as_bool()),
        "duplicate_skipped": duplicate_skipped,
        "expose_public": value.get("expose_public").and_then(|value| value.as_bool()),
        "access_guard_enabled": value.get("access_guard_enabled").and_then(|value| value.as_bool()),
    });
    let message = if duplicate_skipped {
        match (
            app.get("title").and_then(|value| value.as_str()),
            app.get("url").and_then(|value| value.as_str()),
        ) {
            (Some(title), Some(url)) if !title.trim().is_empty() && !url.trim().is_empty() => {
                format!(
                    "A matching app `{}` already exists at {}; skipped creating a duplicate.",
                    title.trim(),
                    url.trim()
                )
            }
            (_, Some(url)) if !url.trim().is_empty() => {
                format!(
                    "A matching app already exists at {}; skipped creating a duplicate.",
                    url.trim()
                )
            }
            _ => "A matching app already exists; skipped creating a duplicate.".to_string(),
        }
    } else {
        match (
            app.get("title").and_then(|value| value.as_str()),
            app.get("url").and_then(|value| value.as_str()),
        ) {
            (Some(title), Some(url)) if !title.trim().is_empty() && !url.trim().is_empty() => {
                format!("App `{}` is available at {}.", title.trim(), url.trim())
            }
            (_, Some(url)) if !url.trim().is_empty() => {
                format!("App is available at {}.", url.trim())
            }
            _ => "App service operation completed.".to_string(),
        }
    };
    serde_json::json!({
        "ok": true,
        "primitive": primitive,
        "tool": "service_manage",
        "status": status,
        "app": app,
        "quality_check": quality_check,
        "message": message,
    })
}

fn sanitize_app_quality_check_for_model(value: &serde_json::Value) -> serde_json::Value {
    let concerns = value
        .get("concerns")
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .take(5)
                .map(|item| safe_truncate(item, 500))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "status": json_value_text(value, "status"),
        "needs_repair": value.get("needs_repair").and_then(|value| value.as_bool()).unwrap_or(false),
        "request_context_attached": value.get("request_context_attached").and_then(|value| value.as_bool()),
        "acceptance_criteria_count": value.get("acceptance_criteria_count").and_then(|value| value.as_u64()),
        "browser_ok": value.get("browser_ok").and_then(|value| value.as_bool()),
        "judge_available": value.get("judge_available").and_then(|value| value.as_bool()),
        "judge_passed": value.get("judge_passed").and_then(|value| value.as_bool()),
        "judge_summary": json_value_text(value, "judge_summary").map(|summary| safe_truncate(&summary, 900)),
        "concerns": concerns,
    })
}

fn sanitize_service_manage_lifecycle_result_for_model(
    primitive: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    let raw_status = json_value_text(value, "status").unwrap_or_else(|| "ok".to_string());
    let service = value
        .get("service")
        .map(sanitize_service_manage_service_for_model);
    let services = value
        .get("services")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .map(sanitize_service_manage_service_for_model)
                .collect::<Vec<_>>()
        });
    let service_count = services
        .as_ref()
        .map(Vec::len)
        .or_else(|| service.as_ref().map(|_| 1))
        .unwrap_or(0);
    let status = if raw_status == "ok" && services.as_ref().is_some_and(Vec::is_empty) {
        "empty".to_string()
    } else {
        raw_status
    };
    let query = json_value_text(value, "query");
    let service_id = json_value_text(value, "service_id").or_else(|| {
        service
            .as_ref()
            .and_then(|service| json_value_text(service, "id"))
    });
    let message = match status.as_str() {
        "not_found" => match query.as_deref() {
            Some(query) if !query.trim().is_empty() => {
                format!(
                    "Managed app/service not found for `{}` in the current registry.",
                    query.trim()
                )
            }
            _ => "No matching managed app/service was found in the current registry.".to_string(),
        },
        "empty" => "No managed apps/services are currently registered.".to_string(),
        _ => {
            if let Some(service) = service.as_ref() {
                let title = json_value_text(service, "title")
                    .or_else(|| json_value_text(service, "id"))
                    .unwrap_or_else(|| "Managed service".to_string());
                format!(
                    "Managed app/service `{}` is present in the current registry.",
                    title
                )
            } else {
                format!(
                    "{} managed app/service item(s) found in the current registry.",
                    service_count
                )
            }
        }
    };
    let terminal_observation = matches!(status.as_str(), "empty" | "not_found" | "ok");

    serde_json::json!({
        "ok": true,
        "primitive": primitive,
        "tool": "service_manage",
        "status": status,
        "service_id": service_id,
        "query": query,
        "service": service,
        "services": services.unwrap_or_default(),
        "service_count": service_count,
        "terminal_observation": terminal_observation,
        "message": message,
        "assistant_instruction": "answer from this result. Do not repeat the same app/service status or list check unless the user asks to recheck after a change.",
    })
}

fn sanitize_service_manage_service_for_model(service: &serde_json::Value) -> serde_json::Value {
    let id = json_value_text(service, "id")
        .or_else(|| json_value_text(service, "app_id"))
        .or_else(|| json_value_text(service, "service_id"));
    let title = json_value_text(service, "title").or_else(|| json_value_text(service, "name"));
    let url = json_value_text(service, "access_url").or_else(|| json_value_text(service, "url"));
    serde_json::json!({
        "id": id,
        "title": title,
        "url": url,
        "status": json_value_text(service, "status"),
        "type": json_value_text(service, "type"),
        "enabled": service.get("enabled").and_then(|value| value.as_bool()),
        "running": service.get("running").and_then(|value| value.as_bool()),
        "is_static": service.get("is_static").and_then(|value| value.as_bool()),
        "runtime_mode": json_value_text(service, "runtime_mode"),
        "created_at": json_value_text(service, "created_at"),
        "updated_at": json_value_text(service, "updated_at"),
        "access_guard_enabled": service.get("access_guard_enabled").and_then(|value| value.as_bool()),
        "public_access_guard_enabled": service.get("public_access_guard_enabled").and_then(|value| value.as_bool()),
    })
}

fn sanitize_managed_file_result_for_model(
    primitive: &str,
    action_name: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    let data = value.get("data").unwrap_or(value);
    let resource = data
        .get("payload")
        .and_then(|payload| payload.get("resource"));
    let write = data.get("write");
    let document = data.get("document");
    let label = document
        .and_then(|doc| json_value_text(doc, "filename"))
        .or_else(|| {
            data.get("artifact")
                .and_then(|artifact| json_value_text(artifact, "label"))
        })
        .or_else(|| write.and_then(|write| json_value_text(write, "label")))
        .or_else(|| {
            resource
                .and_then(|resource| json_value_text(resource, "path"))
                .and_then(|path| resource_label_from_path_text(&path))
        })
        .unwrap_or_else(|| "managed file".to_string());
    let content_type = document
        .and_then(|doc| json_value_text(doc, "content_type"))
        .or_else(|| write.and_then(|write| json_value_text(write, "content_type")))
        .or_else(|| resource.and_then(|resource| json_value_text(resource, "mime")));
    let bytes = document
        .and_then(|doc| doc.get("file_size").and_then(|value| value.as_u64()))
        .or_else(|| write.and_then(|write| write.get("bytes").and_then(|value| value.as_u64())))
        .or_else(|| {
            resource.and_then(|resource| resource.get("bytes").and_then(|value| value.as_u64()))
        });
    let resource_id = resource.and_then(|resource| json_value_text(resource, "id"));
    let mut artifact = serde_json::Map::new();
    artifact.insert(
        "kind".to_string(),
        serde_json::Value::String("managed_file".to_string()),
    );
    artifact.insert(
        "label".to_string(),
        serde_json::Value::String(label.clone()),
    );
    if let Some(id) = resource_id {
        artifact.insert("id".to_string(), serde_json::Value::String(id));
    }
    if let Some(content_type) = content_type.clone() {
        artifact.insert(
            "content_type".to_string(),
            serde_json::Value::String(content_type),
        );
    }
    if let Some(bytes) = bytes {
        artifact.insert("bytes".to_string(), serde_json::json!(bytes));
    }
    if let Some(source_artifact) = data.get("artifact") {
        for key in ["url", "download_url"] {
            if let Some(href) = json_value_text(source_artifact, key)
                .filter(|href| safe_model_visible_artifact_href(href))
            {
                artifact.insert(key.to_string(), serde_json::Value::String(href));
            }
        }
    }
    if !artifact.contains_key("download_url") {
        if let Some(href) = document
            .and_then(|doc| json_value_text(doc, "download_url"))
            .filter(|href| safe_model_visible_artifact_href(href))
        {
            artifact.insert("download_url".to_string(), serde_json::Value::String(href));
        }
    }

    let document_ref = document.and_then(|doc| {
        let id = json_value_text(doc, "id")?;
        let filename = json_value_text(doc, "filename").unwrap_or_else(|| label.clone());
        let mut document_ref = serde_json::json!({
            "id": id,
            "filename": filename,
            "url": json_value_text(doc, "url").unwrap_or_else(|| "/ui/documents".to_string()),
            "duplicate_skipped": doc.get("duplicate_skipped").and_then(|value| value.as_bool()).unwrap_or(false),
        });
        if let Some(href) = json_value_text(doc, "download_url")
            .filter(|href| safe_model_visible_artifact_href(href))
        {
            document_ref["download_url"] = serde_json::Value::String(href);
        }
        Some(document_ref)
    });
    if let Some(document_ref) = document_ref.clone() {
        artifact.insert("document".to_string(), document_ref);
    }

    let duplicate_document_skipped = document_ref
        .as_ref()
        .and_then(|doc| {
            doc.get("duplicate_skipped")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false);
    let message = if duplicate_document_skipped {
        format!(
            "Saved managed file `{}`. An identical document already exists, so Documents ingestion was skipped.",
            label
        )
    } else if document_ref.is_some() {
        format!("Saved managed file `{}` and added it to Documents.", label)
    } else {
        format!("Saved managed file `{}`.", label)
    };

    serde_json::json!({
        "ok": true,
        "primitive": primitive,
        "tool": action_name,
        "status": value.get("status").and_then(|value| value.as_str()).unwrap_or("completed"),
        "artifact": serde_json::Value::Object(artifact),
        "message": message,
    })
}

fn json_value_text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn safe_model_visible_artifact_href(value: &str) -> bool {
    (value == "/ui/documents" || value.starts_with("/ui/documents?"))
        || (value.starts_with("/api/outputs/") && !value.contains("..") && !value.contains('\\'))
}

fn resource_label_from_path_text(path: &str) -> Option<String> {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn estimate_prompt_tokens(messages: &[SpineMessage]) -> usize {
    let chars = messages
        .iter()
        .map(|message| match message {
            SpineMessage::System { content }
            | SpineMessage::User { content }
            | SpineMessage::Tool { content, .. } => content.len(),
            SpineMessage::Assistant {
                content,
                tool_calls,
            } => {
                content.as_deref().unwrap_or_default().len()
                    + serde_json::to_string(tool_calls)
                        .map(|value| value.len())
                        .unwrap_or(0)
            }
        })
        .sum::<usize>();
    chars.div_ceil(4)
}

fn needs_input_message_from_tool_results(results: &[ToolResult]) -> Option<String> {
    results.iter().find_map(|result| {
        let outcome = super::tool_responses::structured_tool_value_outcome(&result.value)?;
        if outcome.state != super::tool_responses::StructuredToolOutcomeState::NeedsInput {
            return None;
        }
        needs_input_message_from_tool_value(&result.value)
    })
}

fn failed_search_message_from_tool_results(results: &[ToolResult]) -> Option<String> {
    results
        .iter()
        .find_map(failed_search_message_from_tool_result)
}

fn failed_search_message_from_tool_result(result: &ToolResult) -> Option<String> {
    if result.ok {
        return None;
    }

    let value = &result.value;
    let domain = first_structured_string(value, &["domain"])
        .and_then(|item| item.parse::<crate::actions::ActionErrorDomain>().ok());
    let message = first_structured_string(value, &["message", "detail"])
        .or_else(|| json_text_path(value, &["diagnostics", "message"]))
        .unwrap_or_default();
    if domain != Some(crate::actions::ActionErrorDomain::Search) {
        return None;
    }

    Some(search_backend_unavailable_user_message(&message))
}

fn search_backend_unavailable_user_message(detail: &str) -> String {
    let mut message = crate::actions::search::SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE.to_string();
    let detail = detail.trim();
    if !detail.is_empty() {
        message.push_str(
            "\n\nThe free built-in search fallback failed or is currently unavailable. Configure an API-backed search provider or SearXNG in Settings -> Search for reliable live search.",
        );
    }
    message
}

fn needs_input_message_from_tool_value(value: &serde_json::Value) -> Option<String> {
    super::tool_responses::structured_user_question_from_value(value)
        .or_else(|| credential_handoff_message_from_tool_value(value))
}

fn credential_handoff_message_from_tool_value(value: &serde_json::Value) -> Option<String> {
    let request = find_credential_request_value(value)
        .or_else(|| infer_credential_request_from_result(value))?;
    if !credential_request_requires_secure_input(&request) {
        return None;
    }
    if let Some(message) = find_text_field(value, "message")
        .or_else(|| find_text_field(value, "detail"))
        .filter(|message| !message.trim().is_empty())
    {
        return Some(message);
    }

    let mut message = "I saved the available setup, but credentials are required before it can be used. Use the secure credential form shown in this chat".to_string();
    if let Some(settings_path) = credential_value_text(&request, "settings_path") {
        message.push_str(" or ");
        message.push_str(&settings_path);
    }
    message.push('.');
    Some(message)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionVerdict {
    Complete,
    Incomplete { evidence_gap: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionStep {
    Accept,
    Reprompt { prompt: String },
    AcceptWithCaveat { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalAuditReason {
    CurrentTurnToolEvidence,
    BlockedTerminal,
    CompletionReprompt,
    CapabilityReadinessReloop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalAuditDecision {
    Skip,
    Audit { reason: TerminalAuditReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FreshnessVerdict {
    Fresh,
    RefreshRequired {
        reason: String,
        query: Option<String>,
    },
}

const MAX_COMPLETION_REPROMPTS: usize = 1;
const COMPLETION_REPROMPT_CONTINUATION_TURNS: usize = 2;
const MAX_TOOL_REPAIR_ATTEMPTS: usize = 2;

/// Run-wide ceiling on completion-verification reprompts. Per-evidence
/// reprompts may reopen after successful tool progress, but a run that keeps
/// failing to produce the requested user-visible outcome must eventually stop
/// and report the current status instead of spending the whole turn budget.
const MAX_TOTAL_COMPLETION_REPROMPTS: usize = 2;

fn completion_guarded_max_turns(request_max_turns: usize) -> usize {
    request_max_turns
        .max(1)
        .saturating_add(COMPLETION_REPROMPT_CONTINUATION_TURNS)
}

fn completion_reprompts_after_tool_evidence(_completion_reprompts: usize) -> usize {
    0
}

fn current_turn_has_tool_evidence(
    messages: &[SpineMessage],
    current_turn_evidence_start: usize,
) -> bool {
    messages
        .iter()
        .skip(current_turn_evidence_start)
        .any(|message| matches!(message, SpineMessage::Tool { .. }))
}

fn terminal_audit_decision(
    kind: SpineTerminalTextKind,
    provenance: TerminalTextProvenance,
    caller_kind: CallerKind,
    messages: &[SpineMessage],
    current_turn_evidence_start: usize,
    completion_reprompts: usize,
    capability_readiness_reloops: usize,
) -> TerminalAuditDecision {
    if !terminal_kind_requires_completion_verification(kind, provenance, caller_kind) {
        return TerminalAuditDecision::Skip;
    }
    if kind == SpineTerminalTextKind::Blocked {
        return TerminalAuditDecision::Audit {
            reason: TerminalAuditReason::BlockedTerminal,
        };
    }
    if current_turn_has_tool_evidence(messages, current_turn_evidence_start) {
        return TerminalAuditDecision::Audit {
            reason: TerminalAuditReason::CurrentTurnToolEvidence,
        };
    }
    if completion_reprompts > 0 {
        return TerminalAuditDecision::Audit {
            reason: TerminalAuditReason::CompletionReprompt,
        };
    }
    if capability_readiness_reloops > 0 {
        return TerminalAuditDecision::Audit {
            reason: TerminalAuditReason::CapabilityReadinessReloop,
        };
    }
    TerminalAuditDecision::Skip
}

fn current_turn_tool_evidence_text(messages: &[SpineMessage], start_index: usize) -> String {
    let mut remaining = COMPLETION_VERIFIER_EVIDENCE_CHAR_BUDGET;
    let mut selected = Vec::new();

    for content in messages
        .iter()
        .skip(start_index)
        .filter_map(|message| match message {
            SpineMessage::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .rev()
    {
        if remaining == 0 {
            break;
        }
        let content_chars = content.chars().count();
        if content_chars <= remaining {
            selected.push(content.to_string());
            remaining -= content_chars;
        } else {
            selected.push(tail_chars(content, remaining));
            break;
        }
    }

    selected.reverse();
    selected.join("\n---\n")
}

/// Structural grounding: true when the current turn produced at least one
/// tool result whose normalized evidence envelope reports status "ok"
/// (normalize_tool_evidence_envelope writes that top-level status into every
/// Tool message). An answer backed by same-turn successful tool evidence is
/// fresh BY CONSTRUCTION — no LLM freshness judgment is needed. Pure message
/// shape; no intent classification.
fn current_turn_has_successful_tool_evidence(
    messages: &[SpineMessage],
    current_turn_evidence_start: usize,
) -> bool {
    messages
        .iter()
        .skip(current_turn_evidence_start)
        .any(|message| match message {
            SpineMessage::Tool { content, .. } => {
                serde_json::from_str::<serde_json::Value>(content)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("status")
                            .and_then(|status| status.as_str())
                            .map(|status| status == "ok")
                    })
                    .unwrap_or(false)
            }
            _ => false,
        })
}

const CAPABILITY_READINESS_CONTEXT_PREFIX: &str = "Capability readiness context:\n";

fn has_capability_readiness_context_evidence(messages: &[SpineMessage]) -> bool {
    messages.iter().any(|message| {
        let SpineMessage::System { content } = message else {
            return false;
        };
        let Some(raw_json) = content.strip_prefix(CAPABILITY_READINESS_CONTEXT_PREFIX) else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw_json) else {
            return false;
        };
        value
            .get("generation")
            .and_then(|value| value.as_u64())
            .is_some()
            && value
                .get("entries")
                .and_then(|value| value.as_array())
                .is_some()
    })
}

/// ONE verifier round-trip producing BOTH terminal judgments (freshness +
/// completion) over the same inputs, replacing two sequential LLM calls.
async fn combined_terminal_verdicts(
    server: &dyn SpineLlmServer,
    cx: &SpineContext,
    current_user_request: &str,
    messages: &[SpineMessage],
    current_turn_evidence_start: usize,
    turn: usize,
    proposed_answer: &str,
) -> (FreshnessVerdict, CompletionVerdict) {
    cx.emit(SpineTraceEvent::CompletionVerificationStarted {
        turn,
        proposed_answer_chars: proposed_answer.chars().count(),
    })
    .await;
    let prompt = build_combined_verification_prompt(
        current_user_request,
        proposed_answer,
        &recent_user_context_text(messages),
        &current_turn_tool_evidence_text(messages, current_turn_evidence_start),
        &recent_tool_evidence_text(messages),
    );
    let started = std::time::Instant::now();
    let (freshness, completion) = match server.terminal_audit_completion(prompt).await {
        Ok(response) => {
            tracing::debug!(
                response_preview = %safe_truncate(&response.text, 320),
                "Spine merged terminal verifier raw response"
            );
            parse_combined_verdicts(&response.text)
        }
        Err(_) => (FreshnessVerdict::Fresh, CompletionVerdict::Complete),
    };
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        requires_refresh = matches!(freshness, FreshnessVerdict::RefreshRequired { .. }),
        complete = matches!(completion, CompletionVerdict::Complete),
        "Spine merged terminal verifier completed"
    );
    cx.emit(SpineTraceEvent::CompletionVerificationCompleted {
        turn,
        complete: matches!(completion, CompletionVerdict::Complete),
    })
    .await;
    (freshness, completion)
}

fn build_combined_verification_prompt(
    user_goal: &str,
    proposed_answer: &str,
    recent_user_context: &str,
    current_turn_evidence: &str,
    recent_evidence: &str,
) -> String {
    format!(
        "You are the terminal verifier for an agent answer. Make TWO independent judgments from underlying meaning, not wording, casing, grammar, punctuation, order, or style.\n\n\
         USER REQUEST:\n{user_goal}\n\n\
         RECENT USER CONTEXT:\n{recent_user_context}\n\n\
         PROPOSED FINAL ANSWER:\n{proposed_answer}\n\n\
         CURRENT-TURN LIVE EVIDENCE:\n{current_turn_evidence}\n\n\
         EVIDENCE PRODUCED SO FAR:\n{recent_evidence}\n\n\
         Judgment 1 — freshness: does the proposed answer make or rely on a current external-capability state claim (install, connection, authentication, credential, readiness, availability, provider access) that the current-turn live evidence does not already establish? If yes, set requires_refresh true with a brief semantic reason and the target capability as query when inferable.\n\
         Judgment 2 — completion: has every meaningful part of the user's request actually been fulfilled per the produced evidence, not merely announced or intended? If not, set complete false with evidence_gap describing the concrete missing user-visible outcome and how to tell — never internal procedure, tool names, function names, JSON keys, route names, or a prescribed mechanism.\n\n\
         Reply with only one JSON object: {{\"requires_refresh\": <bool>, \"reason\": \"<only when requires_refresh>\", \"query\": \"<only when requires_refresh and inferable>\", \"complete\": <bool>, \"evidence_gap\": \"<only when complete is false>\"}}. Judge without assuming any task category, tool, wording, or response shape."
    )
}

/// Lenient parse mirroring the single-judgment parsers: malformed output
/// defaults to (Fresh, Complete) so a verifier hiccup can never block a turn.
fn parse_combined_verdicts(text: &str) -> (FreshnessVerdict, CompletionVerdict) {
    let Some(object_text) = extract_first_json_object(text) else {
        return (FreshnessVerdict::Fresh, CompletionVerdict::Complete);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(object_text) else {
        return (FreshnessVerdict::Fresh, CompletionVerdict::Complete);
    };
    let freshness = if value
        .get("requires_refresh")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        let reason = value
            .get("reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(
                "the answer depends on current external integration state without live evidence",
            )
            .to_string();
        let query = value
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        FreshnessVerdict::RefreshRequired { reason, query }
    } else {
        FreshnessVerdict::Fresh
    };
    let completion = if value
        .get("complete")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
    {
        CompletionVerdict::Complete
    } else if let Some(evidence_gap) = value
        .get("evidence_gap")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        CompletionVerdict::Incomplete {
            evidence_gap: evidence_gap.to_string(),
        }
    } else {
        CompletionVerdict::Complete
    };
    (freshness, completion)
}

#[cfg(test)]
fn parse_freshness_verdict(text: &str) -> FreshnessVerdict {
    let Some(object_text) = extract_first_json_object(text) else {
        return FreshnessVerdict::Fresh;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(object_text) else {
        return FreshnessVerdict::Fresh;
    };
    if !value
        .get("requires_refresh")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return FreshnessVerdict::Fresh;
    }
    let reason = value
        .get("reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("the answer depends on current external integration state without live evidence")
        .to_string();
    let query = value
        .get("query")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    FreshnessVerdict::RefreshRequired { reason, query }
}

#[cfg(test)]
fn build_freshness_prompt(
    user_goal: &str,
    proposed_answer: &str,
    current_turn_evidence: &str,
) -> String {
    format!(
        "You are a freshness verifier for external capability state. Decide from meaning whether the proposed answer depends on current install, connection, authentication, credential, readiness, availability, or provider-access state for an external capability.\n\n\
         USER REQUEST:\n{user_goal}\n\n\
         PROPOSED ANSWER:\n{proposed_answer}\n\n\
         CURRENT-TURN LIVE EVIDENCE:\n{current_turn_evidence}\n\n\
         If the proposed answer makes or relies on a current external-capability state claim and the current-turn evidence does not already establish that state, return {{\"requires_refresh\": true, \"reason\": \"<brief semantic reason>\", \"query\": \"<target capability if inferable>\"}}.\n\
         If the answer does not rely on current external-capability state, or current-turn live evidence already establishes the state, return {{\"requires_refresh\": false}}.\n\
         Judge by underlying meaning, not wording, casing, grammar, punctuation, order, or style. Return only one JSON object."
    )
}

fn freshness_refresh_tool_call(verdict: &FreshnessVerdict) -> Option<SpineToolCall> {
    let FreshnessVerdict::RefreshRequired { query, .. } = verdict else {
        return None;
    };
    let arguments = if let Some(query) = query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.split_whitespace().count() == 1)
    {
        // A tight single identity token targets one record via inspect.
        serde_json::json!({
            "kind": "integration",
            "op": "status",
            "query": query,
        })
    } else {
        // Descriptive multi-word phrases ("Linear integration", "GraphQL API
        // connection") routinely contain words no stored record carries;
        // routing them into the targeted matcher produced false no-match
        // results that verifiers read as absence. The compact full inventory
        // lets the model resolve by exact id instead. Shape-gated only —
        // never phrase- or provider-gated.
        serde_json::json!({
            "kind": "integration",
            "op": "list",
        })
    };
    Some(SpineToolCall {
        id: format!("freshness_refresh_{}", uuid::Uuid::new_v4().simple()),
        name: "resource_rw".to_string(),
        arguments,
        activity_label: Some("Checking current integration state".to_string()),
    })
}

fn structured_bool_field_recursive(
    value: &serde_json::Value,
    key: &str,
    depth: usize,
) -> Option<bool> {
    if depth > 4 {
        return None;
    }
    let object = value.as_object()?;
    if let Some(found) = object.get(key).and_then(|value| value.as_bool()) {
        return Some(found);
    }
    for nested_key in ["data", "result", "diagnostics"] {
        if let Some(found) = object
            .get(nested_key)
            .and_then(|value| match value {
                serde_json::Value::String(text) => {
                    serde_json::from_str::<serde_json::Value>(text).ok()
                }
                serde_json::Value::Object(_) => Some(value.clone()),
                _ => None,
            })
            .and_then(|nested| structured_bool_field_recursive(&nested, key, depth + 1))
        {
            return Some(found);
        }
    }
    None
}

/// Records a repair attempt for `tool_name` and returns the own-voice
/// exhaustion instruction once the budget is spent. The model gets
/// MAX_TOOL_REPAIR_ATTEMPTS silent repair turns per tool; the instruction
/// fires on the failure AFTER the budget (attempt N+1) and on every
/// repairable failure thereafter. Loop termination itself is bounded by
/// max_turns, not this counter.
fn note_tool_repair_attempt(
    tool_repair_attempts: &mut HashMap<String, usize>,
    tool_name: &str,
) -> Option<String> {
    let attempts = tool_repair_attempts
        .entry(tool_name.to_string())
        .and_modify(|count| *count += 1)
        .or_insert(1);
    (*attempts > MAX_TOOL_REPAIR_ATTEMPTS).then(|| {
        format!(
            "The {} tool has reached its repair limit for this run. Ask the user one concise question in your own words if a missing fact is required; otherwise explain what cannot be completed without quoting raw tool output.",
            readable_spine_tool_name(tool_name)
        )
    })
}

fn tool_result_progresses_completion_reprompt_budget(result: &ToolResult) -> bool {
    match super::tool_responses::structured_tool_value_outcome(&result.value) {
        Some(outcome) => {
            outcome.state == super::tool_responses::StructuredToolOutcomeState::Success
        }
        None => result.ok,
    }
}

fn tool_result_requires_model_repair(result: &ToolResult) -> bool {
    let Some(outcome) = super::tool_responses::structured_tool_value_outcome(&result.value) else {
        return !result.ok;
    };
    match outcome.state {
        super::tool_responses::StructuredToolOutcomeState::Success => false,
        super::tool_responses::StructuredToolOutcomeState::NeedsInput => {
            needs_input_message_from_tool_value(&result.value).is_none()
        }
        super::tool_responses::StructuredToolOutcomeState::Failure => {
            structured_bool_field_recursive(&result.value, "recoverable_by_model", 0)
                .or_else(|| structured_bool_field_recursive(&result.value, "retryable", 0))
                .unwrap_or(false)
        }
    }
}

fn completion_verification_required_for_caller(caller_kind: CallerKind) -> bool {
    matches!(
        caller_kind,
        CallerKind::Chat
            | CallerKind::Task
            | CallerKind::Watcher
            | CallerKind::Cron
            | CallerKind::Gateway
            | CallerKind::Companion
    )
}

fn terminal_kind_requires_completion_verification(
    kind: SpineTerminalTextKind,
    provenance: TerminalTextProvenance,
    caller_kind: CallerKind,
) -> bool {
    if provenance == TerminalTextProvenance::StructuredUserQuestion {
        return false;
    }
    if kind == SpineTerminalTextKind::NeedsInput {
        return false;
    }
    completion_verification_required_for_caller(caller_kind)
}

async fn verification_verdict_for_terminal_candidate(
    server: &dyn SpineLlmServer,
    cx: &SpineContext,
    current_user_request: &str,
    messages: &[SpineMessage],
    caller_kind: CallerKind,
    kind: SpineTerminalTextKind,
    provenance: TerminalTextProvenance,
    turn: usize,
    final_text: &str,
) -> CompletionVerdict {
    if !terminal_kind_requires_completion_verification(kind, provenance, caller_kind) {
        return CompletionVerdict::Complete;
    }
    cx.emit(SpineTraceEvent::CompletionVerificationStarted {
        turn,
        proposed_answer_chars: final_text.chars().count(),
    })
    .await;
    let completion_verdict =
        verify_completion(server, current_user_request, messages, final_text).await;
    cx.emit(SpineTraceEvent::CompletionVerificationCompleted {
        turn,
        complete: matches!(completion_verdict, CompletionVerdict::Complete),
    })
    .await;
    completion_verdict
}

fn next_completion_step(
    verdict: CompletionVerdict,
    completion_reprompts: usize,
    total_completion_reprompts: usize,
    turn: usize,
    max_turns: usize,
) -> CompletionStep {
    match verdict {
        CompletionVerdict::Complete => CompletionStep::Accept,
        CompletionVerdict::Incomplete { evidence_gap }
            if completion_reprompts < MAX_COMPLETION_REPROMPTS
                && total_completion_reprompts < MAX_TOTAL_COMPLETION_REPROMPTS
                && turn + 1 < max_turns =>
        {
            CompletionStep::Reprompt {
                prompt: completion_continue_prompt(&public_completion_gap(&evidence_gap)),
            }
        }
        CompletionVerdict::Incomplete { evidence_gap } => CompletionStep::AcceptWithCaveat {
            message: completion_incomplete_message(&public_completion_gap(&evidence_gap)),
        },
    }
}

fn extract_first_json_object(text: &str) -> Option<&str> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in text.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(idx);
                depth = 1;
            }
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let start = start?;
                    return Some(&text[start..=idx]);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_verification_verdict(text: &str) -> CompletionVerdict {
    let Some(object_text) = extract_first_json_object(text) else {
        return CompletionVerdict::Complete;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(object_text) else {
        return CompletionVerdict::Complete;
    };
    if value
        .get("complete")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
    {
        return CompletionVerdict::Complete;
    }
    let Some(evidence_gap) = value
        .get("evidence_gap")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return CompletionVerdict::Complete;
    };
    CompletionVerdict::Incomplete {
        evidence_gap: evidence_gap.to_string(),
    }
}

fn build_verification_prompt(
    user_goal: &str,
    proposed_answer: &str,
    recent_user_context: &str,
    tool_evidence: &str,
) -> String {
    format!(
        "You are a completion verifier. Decide whether the user's request has actually been fulfilled using the produced evidence, not merely announced or intended.\n\n\
         USER REQUEST:\n{user_goal}\n\n\
         RECENT USER CONTEXT:\n{recent_user_context}\n\n\
         PROPOSED FINAL ANSWER:\n{proposed_answer}\n\n\
         EVIDENCE PRODUCED SO FAR:\n{tool_evidence}\n\n\
         Reply with only a JSON object. Use {{\"complete\": true}} when every meaningful part of the request is genuinely done. Use {{\"complete\": false, \"evidence_gap\": \"<concrete missing user-visible outcome and how to tell>\"}} when the outcome is not yet evidenced. The evidence may contain internal traces, but evidence_gap must describe the missing user-visible outcome and visible proof, not internal procedure, tool names, function names, JSON keys, exact route names, or a prescribed implementation mechanism. Judge the outcome by the user's intent and the evidence, without assuming any particular task category, tool, wording, or response shape."
    )
}

fn token_looks_like_internal_identifier(raw: &str) -> bool {
    let token = raw
        .trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | ':' | '.' | '/'));
    if token.len() < 3 || !token.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    token.contains('_') || token.contains("::") || token.ends_with("()")
}

fn redact_internal_identifier_shapes(text: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();

    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if token_looks_like_internal_identifier(token) {
            out.push_str("internal operation");
        } else {
            out.push_str(token);
        }
        token.clear();
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '.' | '/' | '(' | ')') {
            token.push(ch);
        } else {
            flush(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn strip_tool_completion_fragments(text: &str) -> String {
    let marker = crate::runtime::TOOL_COMPLETION_MARKER;
    let mut output = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(marker) {
        output.push_str(&rest[..index]);
        let after_marker = &rest[index + marker.len()..];
        let trimmed_start = after_marker
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(after_marker.len());
        let after_ws = &after_marker[trimmed_start..];
        if after_ws.starts_with('{') {
            if let Some(json_end) = matching_json_object_end(after_ws) {
                rest = &after_ws[json_end + 1..];
                continue;
            }
        }
        rest = after_ws
            .split_once('\n')
            .map(|(_, tail)| tail)
            .unwrap_or("");
    }
    output.push_str(rest);
    output
}

fn matching_json_object_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn generic_terminal_fallback(kind: SpineTerminalTextKind) -> &'static str {
    match kind {
        SpineTerminalTextKind::Completed => {
            "I could not safely render the final answer from the available output."
        }
        SpineTerminalTextKind::Blocked => {
            "I could not complete that yet, and the internal failure details were withheld."
        }
        SpineTerminalTextKind::NeedsInput => "I need one more detail before I can continue.",
    }
}

fn sanitize_user_visible_text(text: &str, redact_internal_identifiers: bool) -> String {
    let stripped = crate::core::model::llm_context_sanitizer::strip_internal_tool_transcript(text);
    let stripped = strip_tool_completion_fragments(&stripped);
    let redacted = crate::security::redact_secret_input(&stripped).text;
    let redacted = crate::security::redact_pii(&redacted);
    let redacted = if redact_internal_identifiers {
        redact_internal_identifier_shapes(&redacted)
    } else {
        redacted
    };
    normalize_terminal_whitespace(&redacted)
}

/// Structure-preserving whitespace cleanup for terminal answers: trims
/// line-trailing whitespace and caps blank-line runs at one, but never folds
/// newlines into spaces — markdown paragraphs, lists, and fenced code blocks
/// must survive the funnel intact. Whitespace *collapsing* stays reserved for
/// short single-line gap descriptors (`compact_public_gap_text`).
fn normalize_terminal_whitespace(text: &str) -> String {
    let mut out = String::new();
    let mut blank_run = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_run += 1;
        } else {
            if !out.is_empty() {
                out.push('\n');
                if blank_run > 0 {
                    out.push('\n');
                }
            }
            blank_run = 0;
            out.push_str(trimmed);
        }
    }
    out
}

fn finalize_user_text(
    kind: SpineTerminalTextKind,
    text: &str,
    provenance: TerminalTextProvenance,
) -> String {
    if provenance == TerminalTextProvenance::ToolOrigin {
        return generic_terminal_fallback(kind).to_string();
    }
    let redact_internal_identifiers = !(kind == SpineTerminalTextKind::Completed
        && provenance == TerminalTextProvenance::ModelAuthored);
    let sanitized = sanitize_user_visible_text(text, redact_internal_identifiers);
    if sanitized.trim().is_empty() {
        generic_terminal_fallback(kind).to_string()
    } else {
        sanitized
    }
}

fn compact_public_gap_text(text: &str) -> String {
    let mut out = String::new();
    let mut previous_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !previous_space && !out.is_empty() {
                out.push(' ');
            }
            previous_space = true;
        } else {
            out.push(ch);
            previous_space = false;
        }
    }
    out.trim().to_string()
}

fn public_completion_gap(evidence_gap: &str) -> String {
    let gap = compact_public_gap_text(&redact_internal_identifier_shapes(evidence_gap));
    if gap.is_empty() {
        "the requested outcome is not yet evidenced".to_string()
    } else {
        gap
    }
}

fn completion_continue_prompt(evidence_gap: &str) -> String {
    format!(
        "The requested outcome is not yet fully done. Gap: {}. Continue by taking the next necessary action; do not summarize instead of finishing.",
        evidence_gap.trim()
    )
}

fn completion_incomplete_message(evidence_gap: &str) -> String {
    let evidence_gap = evidence_gap.trim();
    if evidence_gap.is_empty() {
        "I wasn't able to complete that request. Current status: the requested outcome is not yet evidenced."
            .to_string()
    } else {
        format!(
            "I wasn't able to complete that request. Current status: {}",
            evidence_gap
        )
    }
}

fn incomplete_terminal_acceptance_text(
    incomplete_status: &str,
    candidate_text: &str,
    provenance: TerminalTextProvenance,
) -> String {
    let incomplete_status = incomplete_status.trim();
    let candidate_text = candidate_text.trim();
    if matches!(
        provenance,
        TerminalTextProvenance::ModelAuthored | TerminalTextProvenance::ToolOrigin
    ) || candidate_text.is_empty()
    {
        return incomplete_status.to_string();
    }
    format!("{}\n\n{}", incomplete_status, candidate_text)
}

/// The CURRENT turn's user request: the last User message of the initial
/// request snapshot. Call this on `request.messages` (pristine), not the
/// working message list — the run loop appends synthetic User messages
/// (completion reprompts, freshness refresh notes) that must never become
/// the verification anchor.
fn current_user_request_text(messages: &[SpineMessage]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            SpineMessage::User { content } => Some(content.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

const COMPLETION_VERIFIER_EVIDENCE_CHAR_BUDGET: usize = 12_000;
const COMPLETION_VERIFIER_USER_CONTEXT_CHAR_BUDGET: usize = 2_000;

fn tail_chars(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    let start = text
        .char_indices()
        .nth(char_count - max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    format!("[truncated]\n{}", &text[start..])
}

fn recent_tool_evidence_text(messages: &[SpineMessage]) -> String {
    let mut remaining = COMPLETION_VERIFIER_EVIDENCE_CHAR_BUDGET;
    let mut selected = Vec::new();

    for content in messages.iter().rev().filter_map(|message| match message {
        SpineMessage::Tool { content, .. } => Some(content.as_str()),
        _ => None,
    }) {
        if remaining == 0 {
            break;
        }
        let content_chars = content.chars().count();
        if content_chars <= remaining {
            selected.push(content.to_string());
            remaining -= content_chars;
        } else {
            selected.push(tail_chars(content, remaining));
            break;
        }
    }

    selected.reverse();
    selected.join("\n---\n")
}

fn recent_user_context_text(messages: &[SpineMessage]) -> String {
    let mut remaining = COMPLETION_VERIFIER_USER_CONTEXT_CHAR_BUDGET;
    let mut selected = Vec::new();

    for content in messages.iter().rev().filter_map(|message| match message {
        SpineMessage::User { content } => Some(content.trim()),
        _ => None,
    }) {
        if remaining == 0 {
            break;
        }
        if content.is_empty() {
            continue;
        }
        let content_chars = content.chars().count();
        if content_chars <= remaining {
            selected.push(content.to_string());
            remaining -= content_chars;
        } else {
            selected.push(tail_chars(content, remaining));
            break;
        }
    }

    selected.reverse();
    selected.join("\n---\n")
}

async fn verify_completion(
    server: &dyn SpineLlmServer,
    current_user_request: &str,
    messages: &[SpineMessage],
    proposed_answer: &str,
) -> CompletionVerdict {
    let recent_user_context = recent_user_context_text(messages);
    let tool_evidence = recent_tool_evidence_text(messages);
    let prompt = build_verification_prompt(
        current_user_request,
        proposed_answer,
        &recent_user_context,
        &tool_evidence,
    );
    let started = std::time::Instant::now();
    let verdict = match server.terminal_audit_completion(prompt).await {
        Ok(response) => {
            tracing::debug!(
                response_preview = %safe_truncate(&response.text, 320),
                "Spine completion verifier raw response"
            );
            parse_verification_verdict(&response.text)
        }
        Err(_) => CompletionVerdict::Complete,
    };
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        complete = matches!(verdict, CompletionVerdict::Complete),
        "Spine completion verifier completed"
    );
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_anchor_is_the_current_turn_request_not_the_first() {
        // Live regression (2026-06-07): in a multi-turn conversation the
        // completion/freshness verifiers judged "list 5 European countries"
        // against the FIRST user message ("install the countries graphql
        // api"), producing a false incomplete verdict and a wasted reprompt
        // turn. The anchor must be the latest user request of the pristine
        // request snapshot.
        let messages = vec![
            SpineMessage::User {
                content: "install the countries graphql api".to_string(),
            },
            SpineMessage::User {
                content: "list 5 European countries with their capitals".to_string(),
            },
        ];
        assert_eq!(
            current_user_request_text(&messages),
            "list 5 European countries with their capitals"
        );
        assert_eq!(current_user_request_text(&[]), "");
    }

    #[test]
    fn blank_no_tool_model_response_is_invalid_terminal_spine_output() {
        assert!(spine_response_is_empty_terminal("  \n", 0));
        assert!(!spine_response_is_empty_terminal("Done.", 0));
        assert!(!spine_response_is_empty_terminal("", 1));

        let error = empty_terminal_spine_response_error();
        assert_eq!(error.code, "empty_model_response");
        assert!(!error.message.trim().is_empty());
    }

    #[test]
    fn provider_platform_failure_message_redacts_nested_provider_payload() {
        let raw = r#"Provider API error: {"error":{"message":"This request requires more credits or fewer tokens. Visit https://provider.example/settings/credits. You requested up to 65536 tokens, but can only afford 13902.","code":402,"metadata":{"raw":"nested provider payload"}}}"#;

        let message = user_visible_platform_failure_message(raw);

        assert!(message.contains("model provider"));
        assert!(message.contains("quota"));
        assert!(message.contains("billing"));
        assert!(!message.contains('{'));
        assert!(!message.contains("provider.example"));
        assert!(!message.contains("65536"));
        assert!(message.chars().count() <= 360);
    }

    #[test]
    fn provider_platform_failure_message_handles_rate_limit_without_raw_error_dump() {
        let raw = r#"upstream failed: {"error":{"message":"Too many requests for this model","status":429,"details":["retry later","burst limit"]}}"#;

        let message = user_visible_platform_failure_message(raw);

        assert!(message.contains("model provider"));
        assert!(message.contains("rate limit"));
        assert!(!message.contains('{'));
        assert!(!message.contains("Too many requests"));
        assert!(message.chars().count() <= 360);
    }

    #[test]
    fn primitive_registry_matches_declared_spine_primitives() {
        let registry = ToolRegistry::new();
        let names = registry
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert_eq!(names, PRIMITIVE_NAMES);
    }

    #[test]
    fn primitive_registry_exposes_generic_runtime_action_escape_hatch() {
        let schema = ToolRegistry::new()
            .schemas()
            .into_iter()
            .find(|schema| schema.name == "action_call")
            .expect("spine should expose a bounded generic runtime action primitive");

        assert!(schema
            .description
            .contains("specific installed runtime action"));
        let required = schema
            .input_schema
            .get("required")
            .and_then(|value| value.as_array())
            .expect("action_call schema should declare required fields")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert!(required.contains(&"action_name"));
        assert!(required.contains(&"arguments"));
    }

    #[test]
    fn action_call_plan_requires_exact_installed_action_and_argument_envelope() {
        let installed = crate::actions::ActionDef {
            name: "provider_task_create".to_string(),
            description: "Create provider tasks from a structured payload.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["title"],
                "properties": {
                    "title": {"type": "string"}
                }
            }),
            ..crate::actions::ActionDef::default()
        };

        match plan_action_call(
            &serde_json::json!({
                "action_name": "provider_task_create",
                "arguments": {"title": "Follow up"}
            }),
            Some(&installed),
        ) {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "provider_task_create");
                assert_eq!(actions[0].arguments["title"], "Follow up");
            }
            other => panic!("unexpected plan: {other:?}"),
        }

        assert!(matches!(
            plan_action_call(
                &serde_json::json!({
                    "action_name": "provider_task_create",
                    "title": "Follow up"
                }),
                Some(&installed),
            ),
            PrimitivePlan::Unsupported { .. }
        ));
        assert!(matches!(
            plan_action_call(
                &serde_json::json!({
                    "action_name": "missing_provider_action",
                    "arguments": {"title": "Follow up"}
                }),
                None,
            ),
            PrimitivePlan::Unsupported { .. }
        ));
    }

    #[test]
    fn action_directory_context_is_bounded_and_advises_action_call() {
        let actions = vec![
            crate::actions::ActionDef {
                name: "provider_task_create".to_string(),
                description: "Create provider tasks from a structured payload.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["title"],
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Task title"
                        }
                    }
                }),
                capabilities: vec!["tasks".to_string(), "external_provider".to_string()],
                ..crate::actions::ActionDef::default()
            },
            crate::actions::ActionDef {
                name: "unrelated_archive_read".to_string(),
                description: "Inspect archived records.".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                ..crate::actions::ActionDef::default()
            },
        ];

        let context = build_action_directory_context_message(
            &actions,
            "create a task in the provider with a title",
            1,
        )
        .expect("matching action directory context");

        assert!(context.contains("Runtime action directory"));
        assert!(context.contains("action_call"));
        assert!(context.contains("provider_task_create"));
        assert!(context.contains("title: string required"));
        assert!(context.contains("compact_action_names"));
        assert!(context.contains("unrelated_archive_read"));
        assert!(!context.contains("Inspect archived records"));
    }

    #[test]
    fn spine_prompt_prioritizes_user_provided_primary_sources() {
        let prompt = build_spine_system_prompt("", None, None);
        assert!(prompt.contains("user-provided URLs and documents as primary evidence"));
        assert!(prompt.contains("instead of fetching secondary sources"));
        assert!(prompt
            .contains("do not let older secondary material override current primary evidence"));
        assert!(prompt.contains("Do not send description-only file writes"));
        assert!(prompt.contains("Finish every requested deliverable before the final answer"));
        assert!(prompt.contains("lead with one natural confirmation sentence"));
        assert!(prompt.contains("follow with compact details"));
        assert!(prompt
            .to_ascii_lowercase()
            .contains("do not introduce a formal summary block"));
        assert!(prompt.contains("do not add generic filler follow-up questions"));
        assert!(prompt.contains("deliver it through app_deploy"));
        assert!(prompt.contains("call app_deploy with source_dir"));
        assert!(prompt.contains("accessible /apps/ URL"));
        assert!(prompt.contains("Do not return container paths"));
        assert!(prompt.contains("do not expose internal container filesystem paths"));
        assert!(prompt.contains("Documents surface"));
    }

    #[test]
    fn spine_prompt_requires_tool_evidence_before_durable_mutation_confirmation() {
        let prompt = build_spine_system_prompt("", None, None);

        assert!(prompt.contains(
            "Durable resource create, update, delete, lifecycle, schedule, and notification claims"
        ));
        assert!(
            prompt.contains("must be grounded in a successful tool result from the current turn")
        );
        assert!(
            prompt.contains("do not say it was created, updated, rescheduled, cancelled, or sent")
        );
    }

    #[test]
    fn spine_prompt_prefers_agentark_native_integrations_before_mcp() {
        let prompt = build_spine_system_prompt("", None, None);

        assert!(prompt.contains("Prefer AgentArk-native integration substrates"));
        assert!(prompt.contains("MCP server only when the user or provider source explicitly asks"));
        assert!(prompt.contains("Do not choose MCP merely because a community MCP package"));
    }

    #[test]
    fn spine_prompt_includes_ark_core_surface_glossary_for_direct_product_questions() {
        let prompt = build_spine_system_prompt("", None, None);

        assert!(prompt.contains("Ark Core product glossary"));
        assert!(prompt.contains("Pulse | Operational health"));
        assert!(prompt.contains("Sentinel | Supervision"));
        assert!(prompt.contains("Evolve | Learning lifecycle"));
        assert!(prompt.contains("Memory | Durable facts"));
        assert!(prompt.contains("Reflect | Retrospectives"));
    }

    #[test]
    fn spine_prompt_includes_active_prompt_bundle_primary_response_guidance() {
        let mut bundle = crate::core::self_evolve::PromptBundleProfile::default();
        bundle.primary_response.system_prompt =
            "Use the evolved primary response surface for the current spine turn.".to_string();
        bundle.primary_response.policy_block =
            "Preserve quality while reducing unnecessary prompt weight.".to_string();
        bundle.primary_response.instruction_template =
            "Prefer robust intent-level behavior over phrase-specific handling.".to_string();

        let prompt = build_spine_system_prompt("", Some(&bundle), None);

        assert!(prompt.contains("Use the evolved primary response surface"));
        assert!(prompt.contains("Preserve quality while reducing unnecessary prompt weight"));
        assert!(prompt.contains("Prefer robust intent-level behavior"));
    }

    #[test]
    fn spine_prompt_bundle_keeps_stable_fragments_before_evolvable_fragments() {
        let bundle =
            spine_prompt_bundle::build_spine_prompt_bundle("", None, None, &PRIMITIVE_NAMES);
        let fragments = bundle.ordered_fragments();
        let ids = fragments
            .iter()
            .map(|fragment| fragment.id)
            .collect::<Vec<_>>();
        let first_evolvable = fragments
            .iter()
            .position(|fragment| fragment.evolvable)
            .expect("spine bundle should have evolvable fragments");

        assert!(fragments[..first_evolvable]
            .iter()
            .all(|fragment| !fragment.evolvable));
        assert_eq!(
            fragments[first_evolvable..]
                .iter()
                .take(ALLOWED_EVOLVABLE_SPINE_FRAGMENT_IDS.len())
                .map(|fragment| fragment.id)
                .collect::<Vec<_>>(),
            ALLOWED_EVOLVABLE_SPINE_FRAGMENT_IDS
        );
        assert!(ids.contains(&"spine.non_evolvable_safety"));
        assert!(ids.contains(&"spine.primitive_schema_summary"));
        assert!(ids.contains(&"spine.tool_call_description_contract"));
    }

    #[test]
    fn spine_prompt_requires_visible_model_progress_separate_from_tool_metadata() {
        let prompt = build_spine_system_prompt("", None, None);

        assert!(prompt.contains("short normal assistant sentence"));
        assert!(prompt.contains("user-visible progress prose"));
        assert!(prompt.contains("separate from `_describe`"));
        assert!(prompt.contains("tool-call metadata only"));
    }

    #[test]
    fn spine_prompt_requires_clarification_for_location_dependent_discovery_without_location() {
        let prompt = build_spine_system_prompt("", None, None);

        assert!(prompt.contains("location-dependent"));
        assert!(prompt.contains("grounded location anchor"));
        assert!(prompt.contains("ask one concise clarification"));
        assert!(prompt.contains("Do not substitute timezone"));
        assert!(prompt.contains("stale memories"));
    }

    #[test]
    fn spine_prompt_bundle_uses_only_allowed_active_fragment_overrides() {
        let mut fragment_bundle =
            crate::core::model::prompt_fragments::default_prompt_fragment_bundle();
        fragment_bundle.version = "spine-fragments-test-v2".to_string();
        fragment_bundle
            .fragments
            .push(crate::core::model::prompt_fragments::PromptFragment {
                id: "spine.final_answer_policy".to_string(),
                surface: "spine".to_string(),
                body: "Use the evolved final answer surface without changing safety rules."
                    .to_string(),
                scope_tags: Vec::new(),
                always_on: true,
                priority: 0,
                est_tokens: 16,
                enabled: true,
            });
        fragment_bundle
            .fragments
            .push(crate::core::model::prompt_fragments::PromptFragment {
                id: "spine.non_evolvable_safety".to_string(),
                surface: "spine".to_string(),
                body: "This forbidden stable override must not be rendered.".to_string(),
                scope_tags: Vec::new(),
                always_on: true,
                priority: 0,
                est_tokens: 16,
                enabled: true,
            });

        let prompt = build_spine_system_prompt("", None, Some(&fragment_bundle));

        assert!(prompt.contains("Use the evolved final answer surface"));
        assert!(!prompt.contains("This forbidden stable override"));
        assert!(
            prompt.contains("Stable safety, authorization, credential, and tool-contract rules")
        );
    }

    #[test]
    fn parse_verification_verdict_accepts_complete() {
        assert_eq!(
            parse_verification_verdict(r#"{"complete": true, "evidence_gap": ""}"#),
            CompletionVerdict::Complete
        );
    }

    #[test]
    fn parse_verification_verdict_returns_gap_when_incomplete() {
        let verdict = parse_verification_verdict(
            r#"{"complete": false, "evidence_gap": "missing durable result"}"#,
        );

        assert_eq!(
            verdict,
            CompletionVerdict::Incomplete {
                evidence_gap: "missing durable result".to_string()
            }
        );
    }

    #[test]
    fn parse_verification_verdict_fails_open_on_uncertain_output() {
        assert_eq!(
            parse_verification_verdict("not json"),
            CompletionVerdict::Complete
        );
        assert_eq!(parse_verification_verdict(""), CompletionVerdict::Complete);
        assert_eq!(
            parse_verification_verdict(r#"{"complete": false}"#),
            CompletionVerdict::Complete
        );
    }

    #[test]
    fn terminal_audit_prompt_carries_recent_user_context_for_followups() {
        let messages = vec![
            SpineMessage::User {
                content: "install a public data API".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Installed.".to_string()),
                tool_calls: Vec::new(),
            },
            SpineMessage::User {
                content: "list examples with attributes".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Here are examples.".to_string()),
                tool_calls: Vec::new(),
            },
            SpineMessage::User {
                content: "same category subset".to_string(),
            },
        ];

        let context = recent_user_context_text(&messages);
        let prompt = build_verification_prompt(
            "same category subset",
            "I found the API id. I will retrieve the subset now.",
            &context,
            r#"{"status":"ok","data":{"id":"public-api"}}"#,
        );

        assert!(prompt.contains("RECENT USER CONTEXT"));
        assert!(prompt.contains("list examples with attributes"));
        assert!(prompt.contains("same category subset"));
    }

    #[test]
    fn parse_freshness_verdict_requests_live_integration_refresh_semantically() {
        let verdict = parse_freshness_verdict(
            r#"{"requires_refresh":true,"reason":"the answer depends on current integration credential state without live evidence","query":"Linear"}"#,
        );

        assert_eq!(
            verdict,
            FreshnessVerdict::RefreshRequired {
                reason: "the answer depends on current integration credential state without live evidence"
                    .to_string(),
                query: Some("Linear".to_string()),
            }
        );

        let call = freshness_refresh_tool_call(&verdict)
            .expect("refresh-required verdict should produce a primitive call");
        assert_eq!(call.name, "resource_rw");
        assert_eq!(call.arguments["kind"], "integration");
        assert_eq!(call.arguments["op"], "status");
        assert_eq!(call.arguments["query"], "Linear");
    }

    #[test]
    fn freshness_verdict_fails_open_without_structured_refresh_request() {
        assert_eq!(parse_freshness_verdict("not json"), FreshnessVerdict::Fresh);
        assert_eq!(
            parse_freshness_verdict(r#"{"requires_refresh":false}"#),
            FreshnessVerdict::Fresh
        );
    }

    #[test]
    fn freshness_prompt_requires_semantic_state_evidence_not_phrase_matching() {
        let prompt = build_freshness_prompt(
            "Use the connected work tracker to retrieve my assigned items.",
            "That provider is not ready yet.",
            "",
        )
        .to_ascii_lowercase();

        assert!(prompt.contains("meaning"));
        assert!(prompt.contains("current-turn"));
        assert!(prompt.contains("live"));
        assert!(!prompt.contains("exact phrase"));
        assert!(!prompt.contains("keyword"));
    }

    fn narration_tool_call_stub() -> SpineToolCall {
        SpineToolCall {
            id: "call-1".to_string(),
            name: "resource_rw".to_string(),
            arguments: serde_json::json!({"kind": "integration", "op": "list"}),
            activity_label: None,
        }
    }

    #[test]
    fn narrated_final_text_persists_only_the_terminal_answer() {
        let messages = vec![
            // Prior-history narration must be excluded by index.
            SpineMessage::User {
                content: "earlier request".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("old narration from a previous run".to_string()),
                tool_calls: vec![narration_tool_call_stub()],
            },
            // This run starts here.
            SpineMessage::User {
                content: "current request".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Checking the current integration registry.".to_string()),
                tool_calls: vec![narration_tool_call_stub()],
            },
            SpineMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: r#"{"status":"ok"}"#.to_string(),
            },
            // A turn with tool calls but no prose contributes nothing.
            SpineMessage::Assistant {
                content: None,
                tool_calls: vec![narration_tool_call_stub()],
            },
            SpineMessage::Assistant {
                content: Some("Registering the integration now.".to_string()),
                tool_calls: vec![narration_tool_call_stub()],
            },
            // The terminal assistant message has no tool calls and is excluded;
            // final_text is appended separately.
            SpineMessage::Assistant {
                content: Some("Integration saved and ready.".to_string()),
                tool_calls: Vec::new(),
            },
        ];

        let narrated = spine_narrated_final_text(3, &messages, "Integration saved and ready.");

        assert_eq!(narrated, "Integration saved and ready.");
    }

    #[test]
    fn narrated_final_text_without_progress_prose_is_the_final_answer_unchanged() {
        let messages = vec![
            SpineMessage::User {
                content: "current request".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Direct answer.".to_string()),
                tool_calls: Vec::new(),
            },
        ];

        assert_eq!(
            spine_narrated_final_text(1, &messages, "Direct answer."),
            "Direct answer."
        );
    }

    #[test]
    fn narrated_final_text_does_not_persist_progress_duplicates() {
        let messages = vec![
            SpineMessage::User {
                content: "current request".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Saving the document.".to_string()),
                tool_calls: vec![narration_tool_call_stub()],
            },
            SpineMessage::Assistant {
                content: Some("Saving the document.".to_string()),
                tool_calls: vec![narration_tool_call_stub()],
            },
            SpineMessage::Assistant {
                content: Some("Document saved.".to_string()),
                tool_calls: vec![narration_tool_call_stub()],
            },
        ];

        assert_eq!(
            spine_narrated_final_text(1, &messages, "Document saved."),
            "Document saved."
        );
    }

    #[test]
    fn partial_text_model_prose_stream_event_uses_the_model_text() {
        let model_text =
            "I'll check Linear's API docs and existing integrations before registering it.";

        let event = spine_model_prose_stream_event(3, model_text);

        match event {
            StreamEvent::ToolProgress {
                name,
                content,
                payload: Some(payload),
            } => {
                assert_eq!(name, "agent_model");
                assert_eq!(content, model_text);
                assert_eq!(payload["kind"], "model_prose");
                assert_eq!(payload["content"], model_text);
                assert_eq!(payload["content_snapshot"], model_text);
                assert_eq!(payload["stream_key"], "model-prose:3");
                assert_eq!(payload["done"], true);
            }
            other => panic!("expected model_prose ToolProgress event, got {other:?}"),
        }
    }

    #[test]
    fn visible_partial_text_trims_and_drops_empty_model_text() {
        assert_eq!(
            spine_visible_partial_text(Some("  Checking the registry now.  ")),
            Some("Checking the registry now.".to_string())
        );
        assert_eq!(spine_visible_partial_text(Some("   \n\t")), None);
        assert_eq!(spine_visible_partial_text(None), None);
    }

    #[test]
    fn parse_combined_verdicts_returns_both_judgments_from_one_response() {
        let (freshness, completion) = parse_combined_verdicts(
            r#"{"requires_refresh":true,"reason":"the answer asserts current credential state without live evidence","query":"work tracker","complete":false,"evidence_gap":"the requested items were never retrieved"}"#,
        );

        assert_eq!(
            freshness,
            FreshnessVerdict::RefreshRequired {
                reason: "the answer asserts current credential state without live evidence"
                    .to_string(),
                query: Some("work tracker".to_string()),
            }
        );
        assert_eq!(
            completion,
            CompletionVerdict::Incomplete {
                evidence_gap: "the requested items were never retrieved".to_string(),
            }
        );
    }

    #[test]
    fn parse_combined_verdicts_fails_open_on_malformed_or_partial_output() {
        assert_eq!(
            parse_combined_verdicts("not json"),
            (FreshnessVerdict::Fresh, CompletionVerdict::Complete)
        );
        // Missing fields default to the permissive judgment, mirroring the
        // single-verdict parsers: a verifier hiccup can never block a turn.
        assert_eq!(
            parse_combined_verdicts(r#"{"requires_refresh":false}"#),
            (FreshnessVerdict::Fresh, CompletionVerdict::Complete)
        );
        // complete:false without a usable evidence_gap is treated as Complete,
        // matching parse_verification_verdict.
        assert_eq!(
            parse_combined_verdicts(r#"{"requires_refresh":false,"complete":false}"#),
            (FreshnessVerdict::Fresh, CompletionVerdict::Complete)
        );
    }

    #[test]
    fn combined_verification_prompt_judges_meaning_not_phrasing() {
        let prompt = build_combined_verification_prompt(
            "Use the connected work tracker to retrieve my assigned items.",
            "That provider is not ready yet.",
            "",
            "",
            "",
        )
        .to_ascii_lowercase();

        assert!(prompt.contains("meaning"));
        assert!(prompt.contains("requires_refresh"));
        assert!(prompt.contains("complete"));
        assert!(prompt.contains("evidence_gap"));
        assert!(!prompt.contains("exact phrase"));
        assert!(!prompt.contains("keyword"));
    }

    #[test]
    fn current_turn_successful_tool_evidence_establishes_freshness_structurally() {
        let messages = vec![
            SpineMessage::User {
                content: "earlier turn".to_string(),
            },
            // Pre-window success must NOT count: only current-turn evidence
            // establishes current state.
            SpineMessage::Tool {
                tool_call_id: "stale".to_string(),
                content: r#"{"status":"ok","data":{}}"#.to_string(),
            },
            SpineMessage::User {
                content: "current turn".to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "failed".to_string(),
                content: r#"{"status":"error","error":"boom"}"#.to_string(),
            },
        ];
        let current_turn_start = 2;

        assert!(!current_turn_has_successful_tool_evidence(
            &messages,
            current_turn_start
        ));

        let mut grounded = messages.clone();
        grounded.push(SpineMessage::Tool {
            tool_call_id: "live".to_string(),
            content: r#"{"status":"ok","data":{"records":3}}"#.to_string(),
        });
        assert!(current_turn_has_successful_tool_evidence(
            &grounded,
            current_turn_start
        ));

        // needs_input is not success: the turn still lacks established state.
        let mut pending = messages.clone();
        pending.push(SpineMessage::Tool {
            tool_call_id: "pending".to_string(),
            content: r#"{"status":"needs_input","detail":"credentials required"}"#.to_string(),
        });
        assert!(!current_turn_has_successful_tool_evidence(
            &pending,
            current_turn_start
        ));
    }

    #[test]
    fn ordinary_no_tool_terminal_chat_skips_completion_audit_structurally() {
        let messages = vec![SpineMessage::User {
            content: "arbitrary user wording must not matter".to_string(),
        }];

        let decision = terminal_audit_decision(
            SpineTerminalTextKind::Completed,
            TerminalTextProvenance::ModelAuthored,
            CallerKind::Chat,
            &messages,
            0,
            0,
            0,
        );

        assert_eq!(decision, TerminalAuditDecision::Skip);
    }

    #[test]
    fn current_turn_tool_evidence_requires_completion_audit_structurally() {
        let messages = vec![
            SpineMessage::User {
                content: "same arbitrary wording".to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: r#"{"status":"ok","data":{"id":"work-1"}}"#.to_string(),
            },
        ];

        let decision = terminal_audit_decision(
            SpineTerminalTextKind::Completed,
            TerminalTextProvenance::ModelAuthored,
            CallerKind::Chat,
            &messages,
            1,
            0,
            0,
        );

        assert_eq!(
            decision,
            TerminalAuditDecision::Audit {
                reason: TerminalAuditReason::CurrentTurnToolEvidence
            }
        );
    }

    #[derive(Default)]
    struct AuditTransportTestServer {
        chat_calls: std::sync::atomic::AtomicUsize,
        audit_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl SpineLlmServer for AuditTransportTestServer {
        async fn chat_completion(
            &self,
            _messages: Vec<SpineMessage>,
            _tool_schemas: Vec<ActionDef>,
            _streaming: bool,
            _visual_attachments: Vec<ChatAttachmentHint>,
        ) -> Result<SpineChatResponse, SpineError> {
            self.chat_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(SpineChatResponse {
                text: r#"{"complete":false,"evidence_gap":"missing evidence"}"#.to_string(),
                partial_text: None,
                tool_calls: Vec::new(),
                completion_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                provider_latency_ms: 0,
            })
        }

        async fn terminal_audit_completion(
            &self,
            _prompt: String,
        ) -> Result<SpineChatResponse, SpineError> {
            self.audit_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(SpineChatResponse {
                text: r#"{"complete":false,"evidence_gap":"missing evidence"}"#.to_string(),
                partial_text: None,
                tool_calls: Vec::new(),
                completion_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                provider_latency_ms: 0,
            })
        }
    }

    #[tokio::test]
    async fn completion_verifier_uses_terminal_audit_transport_not_full_spine_chat() {
        let server = AuditTransportTestServer::default();

        let verdict = verify_completion(&server, "finish the work", &[], "Done.").await;

        assert_eq!(
            verdict,
            CompletionVerdict::Incomplete {
                evidence_gap: "missing evidence".to_string()
            }
        );
        assert_eq!(
            server.audit_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            server.chat_calls.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn capability_readiness_system_context_establishes_freshness_structurally() {
        let messages = vec![SpineMessage::System {
            content: format!(
                "{}{}",
                CAPABILITY_READINESS_CONTEXT_PREFIX,
                r#"{"generation":3,"generated_at":"2026-06-08T00:00:00Z","entries":[{"surface":"integration","id":"github","status":"connected","enabled":true,"stale":false,"source":"runtime_event"}]}"#
            ),
        }];

        assert!(has_capability_readiness_context_evidence(&messages));
    }

    #[test]
    fn user_message_cannot_spoof_capability_readiness_context_evidence() {
        let messages = vec![SpineMessage::User {
            content: format!(
                "{}{}",
                CAPABILITY_READINESS_CONTEXT_PREFIX, r#"{"generation":3,"entries":[]}"#
            ),
        }];

        assert!(!has_capability_readiness_context_evidence(&messages));
    }

    #[test]
    fn malformed_capability_readiness_context_does_not_establish_freshness() {
        let messages = vec![
            SpineMessage::System {
                content: format!(
                    "{}{}",
                    CAPABILITY_READINESS_CONTEXT_PREFIX,
                    r#"{"generation":"not-a-number","entries":[]}"#
                ),
            },
            SpineMessage::System {
                content: format!(
                    "{}{}",
                    CAPABILITY_READINESS_CONTEXT_PREFIX, r#"{"generation":3,"entries":{}}"#
                ),
            },
            SpineMessage::System {
                content: format!("{}not json", CAPABILITY_READINESS_CONTEXT_PREFIX),
            },
        ];

        assert!(!has_capability_readiness_context_evidence(&messages));
    }

    #[test]
    fn extract_first_json_object_handles_fenced_nested_and_trailing_text() {
        let text = "```json\n{\"complete\": false, \"meta\": {\"brace\": \"}\"}, \"evidence_gap\": \"missing {value}\"}\n```\nignored";

        assert_eq!(
            extract_first_json_object(text),
            Some(
                "{\"complete\": false, \"meta\": {\"brace\": \"}\"}, \"evidence_gap\": \"missing {value}\"}"
            )
        );
        assert_eq!(
            parse_verification_verdict(text),
            CompletionVerdict::Incomplete {
                evidence_gap: "missing {value}".to_string()
            }
        );
    }

    #[test]
    fn verification_prompt_includes_goal_answer_and_evidence() {
        let prompt = build_verification_prompt(
            "USER WANTS A TRACKER",
            "Here is your result",
            "recent user context",
            "operation result: ok",
        );

        assert!(prompt.contains("USER WANTS A TRACKER"));
        assert!(prompt.contains("recent user context"));
        assert!(prompt.contains("Here is your result"));
        assert!(prompt.contains("operation result: ok"));
        assert!(prompt.to_ascii_lowercase().contains("complete"));
        assert!(prompt.contains("evidence_gap"));
    }

    #[test]
    fn verification_prompt_stays_flow_agnostic() {
        let prompt =
            build_verification_prompt("goal", "answer", "context", "evidence").to_ascii_lowercase();
        for banned in [
            "app_deploy",
            "app_restart",
            "app_stop",
            "app_delete",
            "watcher",
            "work_type",
            "deliverable",
            "automation",
            "artifact",
            "scheduled_task",
            "terminal_observation",
            "line",
            "colon",
            "240",
            "char",
        ] {
            assert!(
                !prompt.contains(banned),
                "verifier prompt leaked flow-specific token: {banned}"
            );
        }
    }

    #[test]
    fn verifier_prompt_requires_user_visible_gaps_not_internal_procedure() {
        let prompt =
            build_verification_prompt("goal", "answer", "context", "evidence").to_ascii_lowercase();

        assert!(prompt.contains("user-visible outcome"));
        assert!(prompt.contains("visible proof"));
        assert!(prompt.contains("not internal procedure"));
    }

    #[test]
    fn recent_tool_evidence_is_bounded_by_character_budget() {
        let old = "o".repeat(COMPLETION_VERIFIER_EVIDENCE_CHAR_BUDGET);
        let new = "new-evidence".to_string();
        let messages = vec![
            SpineMessage::Tool {
                tool_call_id: "old".to_string(),
                content: old,
            },
            SpineMessage::Tool {
                tool_call_id: "new".to_string(),
                content: new.clone(),
            },
        ];

        let evidence = recent_tool_evidence_text(&messages);

        assert!(evidence.chars().count() <= COMPLETION_VERIFIER_EVIDENCE_CHAR_BUDGET + 20);
        assert!(evidence.contains(&new));
        assert!(evidence.contains("[truncated]"));
    }

    #[test]
    fn completion_continue_and_incomplete_messages_are_generic() {
        let gap =
            public_completion_gap("No app_deploy call was executed; file_write only staged files.");
        let prompt = completion_continue_prompt(&gap);
        let blocker = completion_incomplete_message(&gap);

        assert!(prompt.contains("staged files"));
        assert!(blocker.contains("staged files"));
        for banned in [
            "work_type",
            "app_deploy",
            "file_write",
            "watcher",
            "terminal_observation",
        ] {
            assert!(!prompt.contains(banned));
            assert!(!blocker.contains(banned));
        }
    }

    #[test]
    fn needs_arguments_result_does_not_reopen_completion_reprompt_budget() {
        let result = ToolResult::from_value(
            true,
            serde_json::json!({
                "tool": "arbitrary_integration",
                "status": "needs_arguments",
                "detail": "The operation requires more runtime arguments.",
                "data": {
                    "recoverable_by_model": true,
                    "expected_contract": {
                        "required_arguments": ["body"]
                    }
                }
            }),
        );

        assert!(
            !tool_result_progresses_completion_reprompt_budget(&result),
            "structured missing-argument output is repairable, but it is not successful progress"
        );
    }

    #[test]
    fn successful_result_reopens_completion_reprompt_budget() {
        let result = ToolResult::from_value(
            true,
            serde_json::json!({
                "tool": "arbitrary_integration",
                "status": "completed",
                "data": {
                    "records": [1, 2, 3]
                }
            }),
        );

        assert!(tool_result_progresses_completion_reprompt_budget(&result));
    }

    #[test]
    fn incomplete_model_authored_terminal_replaces_unsupported_prose() {
        let answer = incomplete_terminal_acceptance_text(
            &completion_incomplete_message("No records were retrieved or displayed."),
            "The integration is connected. Let me pull the records now.",
            TerminalTextProvenance::ModelAuthored,
        );

        assert!(answer.contains("No records were retrieved or displayed."));
        assert!(!answer.contains("Let me pull"));
        assert!(!answer.contains("connected"));
        assert!(!answer.contains("I could not verify"));
    }

    #[test]
    fn incomplete_system_authored_terminal_can_keep_existing_blocker_context() {
        let answer = incomplete_terminal_acceptance_text(
            &completion_incomplete_message("No provider result was returned."),
            "The provider is unavailable until credentials are configured.",
            TerminalTextProvenance::SystemAuthored,
        );

        assert!(answer.contains("No provider result was returned."));
        assert!(answer.contains("credentials are configured"));
    }

    #[test]
    fn public_completion_gap_redacts_internal_identifier_shapes() {
        let gap = public_completion_gap(
            "No app_deploy call was executed; tool_call_id abc and file_write staged source_paths only.",
        );

        assert!(!gap.contains("app_deploy"));
        assert!(!gap.contains("tool_call_id"));
        assert!(!gap.contains("file_write"));
        assert!(!gap.contains("source_paths"));
        assert!(gap.contains("staged"));
        assert!(gap.contains("internal operation"));
    }

    #[test]
    fn completion_guard_extends_budget_for_verify_nudge_and_finish() {
        assert_eq!(
            completion_guarded_max_turns(30),
            30 + COMPLETION_REPROMPT_CONTINUATION_TURNS
        );
        assert_eq!(
            completion_guarded_max_turns(0),
            1 + COMPLETION_REPROMPT_CONTINUATION_TURNS
        );
    }

    #[test]
    fn user_facing_callers_run_completion_verifier() {
        assert!(completion_verification_required_for_caller(
            CallerKind::Chat
        ));
        assert!(completion_verification_required_for_caller(
            CallerKind::Gateway
        ));
        assert!(completion_verification_required_for_caller(
            CallerKind::Companion
        ));
    }

    #[test]
    fn automation_callers_keep_completion_verification() {
        assert!(completion_verification_required_for_caller(
            CallerKind::Task
        ));
        assert!(completion_verification_required_for_caller(
            CallerKind::Watcher
        ));
        assert!(completion_verification_required_for_caller(
            CallerKind::Cron
        ));
    }

    #[test]
    fn blocked_tool_terminals_require_completion_verification() {
        assert!(terminal_kind_requires_completion_verification(
            SpineTerminalTextKind::Blocked,
            TerminalTextProvenance::SystemAuthored,
            CallerKind::Chat,
        ));
    }

    #[test]
    fn structured_user_questions_skip_completion_verification() {
        assert!(!terminal_kind_requires_completion_verification(
            SpineTerminalTextKind::NeedsInput,
            TerminalTextProvenance::StructuredUserQuestion,
            CallerKind::Chat,
        ));
        assert!(!terminal_kind_requires_completion_verification(
            SpineTerminalTextKind::Blocked,
            TerminalTextProvenance::StructuredUserQuestion,
            CallerKind::Chat,
        ));
    }

    #[test]
    fn interstitial_stop_gets_nudged_then_later_complete_can_accept() {
        let first = next_completion_step(
            CompletionVerdict::Incomplete {
                evidence_gap: "the requested result is not evidenced yet".to_string(),
            },
            0,
            0,
            0,
            3,
        );
        assert!(matches!(first, CompletionStep::Reprompt { .. }));

        let second = next_completion_step(CompletionVerdict::Complete, 1, 1, 1, 3);
        assert_eq!(second, CompletionStep::Accept);
    }

    #[test]
    fn new_tool_evidence_reopens_one_generic_completion_reprompt() {
        let used_reprompts = completion_reprompts_after_tool_evidence(MAX_COMPLETION_REPROMPTS);
        let step = next_completion_step(
            CompletionVerdict::Incomplete {
                evidence_gap: "the visible result is still missing after the latest attempt"
                    .to_string(),
            },
            used_reprompts,
            1,
            2,
            5,
        );

        assert!(
            matches!(step, CompletionStep::Reprompt { .. }),
            "fresh tool evidence should allow one more semantic recovery turn"
        );
    }

    #[test]
    fn run_wide_reprompt_ceiling_stops_reloop_despite_fresh_evidence() {
        // The per-evidence budget is refilled to 0 by fresh tool evidence and
        // there are plenty of turns left, but the run-wide total has hit the
        // ceiling — a permanently-failing tool call must stop reprompting
        // instead of looping to max_turns.
        let step = next_completion_step(
            CompletionVerdict::Incomplete {
                evidence_gap: "the query still returns no usable result".to_string(),
            },
            completion_reprompts_after_tool_evidence(MAX_COMPLETION_REPROMPTS),
            MAX_TOTAL_COMPLETION_REPROMPTS,
            2,
            50,
        );

        assert!(
            matches!(step, CompletionStep::AcceptWithCaveat { .. }),
            "run-wide reprompt ceiling must terminate the loop even with fresh evidence and turns remaining"
        );
    }

    #[test]
    fn persistent_gap_accepts_with_generic_incomplete_status() {
        let step = next_completion_step(
            CompletionVerdict::Incomplete {
                evidence_gap: "the report was never generated by app_deploy".to_string(),
            },
            MAX_COMPLETION_REPROMPTS,
            1,
            1,
            3,
        );

        match step {
            CompletionStep::AcceptWithCaveat { message } => {
                // The status is user-facing, but sanitized and generic: it
                // reports the missing outcome without leaking internal tool
                // identifiers.
                assert!(message.contains("the report was never generated"));
                for banned in ["work_type", "app_deploy", "watcher", "terminal_observation"] {
                    assert!(
                        !message.contains(banned),
                        "caveat leaked flow-specific token: {banned}"
                    );
                }
            }
            other => panic!("expected generic caveat after persistent gap, got {other:?}"),
        }
    }

    #[test]
    fn incomplete_verdict_accepts_with_caveat_when_no_turn_remains() {
        let step = next_completion_step(
            CompletionVerdict::Incomplete {
                evidence_gap: "the final artifact is missing".to_string(),
            },
            0,
            0,
            1,
            2,
        );

        assert!(matches!(step, CompletionStep::AcceptWithCaveat { .. }));
    }

    fn test_tool_call(id: &str, name: &str) -> SpineToolCall {
        SpineToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: serde_json::json!({}),
            activity_label: None,
        }
    }

    #[test]
    fn spine_turn_records_parse_structured_outcomes_without_substring_matching() {
        let result = SpineResult::Completed {
            messages: vec![
                SpineMessage::Assistant {
                    content: None,
                    tool_calls: vec![
                        test_tool_call("call-failed", "resource_rw"),
                        test_tool_call("call-ok", "resource_rw"),
                    ],
                },
                SpineMessage::Tool {
                    tool_call_id: "call-failed".to_string(),
                    content: serde_json::json!({
                        "ok": false,
                        "message": "quoted payload: \"ok\":true"
                    })
                    .to_string(),
                },
                SpineMessage::Tool {
                    tool_call_id: "call-ok".to_string(),
                    content: serde_json::json!({
                        "result": {
                            "status": "ok"
                        }
                    })
                    .to_string(),
                },
            ],
            final_text: "Done.".to_string(),
            turns_used: 1,
        };

        let records = spine_turn_records(&result);

        assert_eq!(records[0].outcome, AgentTurnOutcomeKind::Abandoned);
        assert_eq!(records[1].outcome, AgentTurnOutcomeKind::Succeeded);
    }

    #[test]
    fn spine_tool_start_stream_payload_exposes_sanitized_model_tool_inputs() {
        let call = SpineToolCall {
            id: "call-research-1".to_string(),
            name: "research".to_string(),
            arguments: serde_json::json!({
                "query": "agent memory systems comparison",
                "headers": {
                    "authorization": "Bearer secret"
                },
                "nested": {
                    "password": "secret",
                    "url": "https://example.com/report"
                },
                "_internal": "hidden"
            }),
            activity_label: None,
        };

        let payload = spine_tool_start_stream_payload(&call);

        assert_eq!(payload["kind"], "model_tool_call");
        assert_eq!(payload["tool_call_id"], "call-research-1");
        assert_eq!(payload["tool_name"], "research");
        assert_eq!(
            payload["arguments"]["query"],
            "agent memory systems comparison"
        );
        assert!(payload["arguments"].get("_internal").is_none());
        assert_eq!(payload["arguments"]["headers"], "[redacted]");
        assert_eq!(payload["arguments"]["nested"]["password"], "[redacted]");
        assert_eq!(
            payload["arguments"]["nested"]["url"],
            "https://example.com/report"
        );
        let intent_summary = payload["intent_summary"]
            .as_str()
            .expect("tool start payload should include visible intent summary");
        assert!(intent_summary.contains("research"));
        assert!(intent_summary.contains("agent memory systems comparison"));
    }

    #[test]
    fn spine_tool_start_stream_payload_uses_model_activity_label() {
        let call = SpineToolCall {
            id: "call-read-1".to_string(),
            name: "resource_rw".to_string(),
            arguments: serde_json::json!({
                "kind": "file",
                "op": "read",
                "id": "assets/docs/arkmemory.md"
            }),
            activity_label: Some("Read arkmemory docs".to_string()),
        };

        let payload = spine_tool_start_stream_payload(&call);

        assert_eq!(payload["activity_label"], "Read arkmemory docs");
        assert_eq!(payload["display_label"], "Read arkmemory docs");
        assert_eq!(payload["intent_summary"], "Read arkmemory docs");
        assert!(payload["arguments"].get("activity_label").is_none());
    }

    #[test]
    fn spine_tool_result_stream_content_exposes_sanitized_result_preview() {
        let call = SpineToolCall {
            id: "call-fetch-1".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({ "url": "https://example.com" }),
            activity_label: None,
        };
        let result = ToolResult::from_value(
            true,
            serde_json::json!({
                "ok": true,
                "token": "secret",
                "results": [{
                    "title": "Agent memory report",
                    "body": "x".repeat(2_000)
                }]
            }),
        );

        let content = spine_tool_result_stream_content(&call, &result);
        let payload = serde_json::from_str::<serde_json::Value>(&content)
            .expect("tool result stream content should be structured JSON");

        assert_eq!(payload["kind"], "model_tool_result");
        assert_eq!(payload["tool_call_id"], "call-fetch-1");
        assert_eq!(payload["tool_name"], "fetch");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["result_preview"]["token"], "[redacted]");
        assert_eq!(
            payload["result_preview"]["results"][0]["title"],
            "Agent memory report"
        );
        assert!(
            payload["result_preview"]["results"][0]["body"]
                .as_str()
                .expect("body should remain a string")
                .len()
                < 1_200
        );
        assert!(payload["summary"]
            .as_str()
            .expect("summary should be present")
            .contains("Agent memory report"));
    }

    #[test]
    fn tool_result_model_json_adds_generic_evidence_envelope_without_dropping_payload() {
        let call = SpineToolCall {
            id: "call-app-1".to_string(),
            name: "app_deploy".to_string(),
            arguments: serde_json::json!({}),
            activity_label: None,
        };
        let result = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "error",
                "tool": "app_deploy",
                "domain": "app",
                "reason": "failed",
                "message": "{\"phase\":\"build\",\"command\":\"npm run build\",\"exit_code\":1,\"log_tail\":\"vite missing\",\"retryable\":false,\"hint\":\"patch dependencies\"}",
                "remediation": {"type": "patch"}
            }),
        );

        let payload: serde_json::Value = serde_json::from_str(&result.to_json_for_tool(&call))
            .expect("tool result should serialize as JSON");

        assert_eq!(payload["ok"], false);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["operation"]["primitive"], "app_deploy");
        assert_eq!(payload["capability_tags"][0], "app_deploy");
        assert_eq!(payload["error_class"], "runtime");
        assert_eq!(payload["retryable"], false);
        assert_eq!(payload["diagnostics"]["phase"], "build");
        assert_eq!(payload["diagnostics"]["exit_code"], 1);
        assert_eq!(payload["logs"]["log_tail"], "vite missing");
        assert_eq!(payload["remediation"]["type"], "patch");
    }

    #[test]
    fn structured_chat_request_context_carries_attachments_and_arkorbit() {
        let hints = RequestExecutionHints {
            attachments_present: true,
            attachments: vec![ChatAttachmentHint {
                upload_id: "upload-1".to_string(),
                kind: "image".to_string(),
                content_type: Some("image/png".to_string()),
                document_id: None,
            }],
            arkorbit_context: Some(serde_json::json!({
                "active_orbit_id": "orbit-1",
                "widgets": [{"id": "w1", "kind": "note"}]
            })),
            browser_profile_context: Some(serde_json::json!({
                "profile_id": "profile-1",
                "profile_name": "alex",
                "browser": "chromium"
            })),
            execution_profile: Some(serde_json::json!({
                "capability_tags": ["automation"],
                "deliverables": ["automation"]
            })),
            ..Default::default()
        };

        let context = structured_chat_request_context_system_message(&hints, None)
            .expect("structured request context should be present");

        assert!(context.contains("upload-1"));
        assert!(context.contains("active_orbit_id"));
        assert!(context.contains("orbit-1"));
        assert!(context.contains("browser_profile_context"));
        assert!(context.contains("profile-1"));
        assert!(context.contains("alex"));
        assert!(context.contains("execution_profile"));
        assert!(context.contains("automation"));
    }

    #[test]
    fn structured_chat_request_context_carries_recent_actionable_artifacts() {
        let hints = RequestExecutionHints {
            recent_actionable_artifacts: vec![serde_json::json!({
                "artifact_type": "app",
                "artifact_id": "app-123",
                "title": "GPU pricing comparison",
                "url": "/apps/app-123/",
                "related_actions": ["service_manage"]
            })],
            ..Default::default()
        };

        let context = structured_chat_request_context_system_message(&hints, None)
            .expect("artifact context should be present");

        assert!(context.contains("recent_actionable_artifacts"));
        assert!(context.contains("app-123"));
        assert!(context.contains("service_manage"));
    }

    #[test]
    fn structured_chat_request_context_shrinks_large_runtime_payloads() {
        let large_text = "x".repeat(12_000);
        let hints = RequestExecutionHints {
            attachments_present: true,
            attachments: (0..200)
                .map(|index| ChatAttachmentHint {
                    upload_id: format!("upload-{}-{}", index, "a".repeat(80)),
                    kind: "document".to_string(),
                    content_type: Some("application/json".to_string()),
                    document_id: Some(format!("doc-{}-{}", index, "b".repeat(80))),
                })
                .collect(),
            execution_profile: Some(serde_json::json!({
                "capability_tags": ["automation"],
                "runtime_payload": large_text
            })),
            arkorbit_context: Some(serde_json::json!({
                "active_orbit_id": "orbit-large",
                "runtime_payload": large_text
            })),
            browser_profile_context: Some(serde_json::json!({
                "profile_id": "profile-large",
                "runtime_payload": large_text
            })),
            recent_actionable_artifacts: vec![serde_json::json!({
                "artifact_id": "artifact-large",
                "runtime_payload": large_text
            })],
            ..Default::default()
        };

        let context = structured_chat_request_context_system_message(
            &hints,
            Some(SpineRequestContextBudget { max_chars: 15_000 }),
        )
        .expect("large runtime context should still be represented");

        assert!(
            context.len() < 15_000,
            "runtime request context should be capped, got {} chars",
            context.len()
        );
        assert!(context.contains("\"truncated\":true"));
        assert!(context.contains("\"original_chars\""));
        assert!(
            !context.contains("\n  \"attachments\""),
            "runtime request context should use compact JSON"
        );
    }

    #[test]
    fn structured_chat_request_context_keeps_full_payload_when_budget_allows() {
        let large_text = "useful-context-".repeat(300);
        let hints = RequestExecutionHints {
            arkorbit_context: Some(serde_json::json!({
                "active_orbit_id": "orbit-rich",
                "runtime_payload": large_text
            })),
            ..Default::default()
        };

        let context = structured_chat_request_context_system_message(
            &hints,
            Some(SpineRequestContextBudget { max_chars: 100_000 }),
        )
        .expect("runtime context should be present");

        assert!(!context.contains("\"truncated\":true"));
        assert!(context.contains("useful-context-useful-context"));
    }

    #[test]
    fn client_temporal_context_does_not_override_durable_schedule_contract() {
        let hints = RequestExecutionHints {
            client_timezone: Some("Asia/Kolkata".to_string()),
            client_timezone_offset_minutes: Some(330),
            ..Default::default()
        };

        let context = structured_chat_request_context_system_message(&hints, None)
            .expect("temporal context should be present");

        assert!(context.contains("client_temporal_context"));
        assert!(context.contains("Use cron for recurring scheduled_task cadences"));
        assert!(context.contains("local_time plus timezone for wall-clock"));
        assert!(context.contains("Do not infer that local_time replaces cron"));
    }

    #[test]
    fn browser_profile_context_injects_selected_profile_into_browser_start() {
        let invocation = PrimitiveActionInvocation {
            action_name: "browser_auto".to_string(),
            arguments: serde_json::json!({
                "action": "start_session",
                "task": "Open the account page and read the visible state."
            }),
        };

        let repaired = browser_auto_invocation_with_request_profile(
            &invocation,
            Some(&serde_json::json!({
                "profile_id": "profile-1",
                "profile_name": "Primary login"
            })),
        );

        assert_eq!(repaired.action_name, "browser_auto");
        assert_eq!(repaired.arguments["profile_id"], "profile-1");
    }

    #[test]
    fn browser_profile_context_does_not_override_explicit_browser_profile() {
        let invocation = PrimitiveActionInvocation {
            action_name: "browser_auto".to_string(),
            arguments: serde_json::json!({
                "action": "start_session",
                "task": "Open the account page.",
                "profile": "client account"
            }),
        };

        let repaired = browser_auto_invocation_with_request_profile(
            &invocation,
            Some(&serde_json::json!({
                "profile_id": "profile-1",
                "profile_name": "Primary login"
            })),
        );

        assert_eq!(repaired.arguments["profile"], "client account");
        assert!(repaired.arguments.get("profile_id").is_none());
    }

    #[test]
    fn browser_profile_context_can_inject_profile_name_when_id_is_absent() {
        let invocation = PrimitiveActionInvocation {
            action_name: "browser_auto".to_string(),
            arguments: serde_json::json!({
                "action": "start_session",
                "task": "open the signed-in inbox"
            }),
        };

        let repaired = browser_auto_invocation_with_request_profile(
            &invocation,
            Some(&serde_json::json!({
                "profile_name": "debanka"
            })),
        );

        assert_eq!(repaired.arguments["profile"], "debanka");
        assert!(repaired.arguments.get("profile_id").is_none());
    }

    #[test]
    fn spine_trace_steps_preserve_prompt_telemetry_payload() {
        let steps = spine_trace_steps(&[SpineTraceEvent::PromptTelemetry {
            data: serde_json::json!({
                "trace_kind": "prompt_telemetry",
                "request_mode": "chat",
                "prompt_version": "system_prompt_v2+prompt-bundle-test",
                "final_system_prompt_chars": 1234,
                "tool_schema_chars": 5678,
                "estimated_total_request_chars": 6912,
                "tool_count": 7,
                "sections": {
                    "runtime_access_summary": 900,
                    "action_catalog": 5678
                }
            }),
        }]);

        let telemetry_step = steps
            .iter()
            .find(|step| step.title == "Prompt Telemetry")
            .expect("prompt telemetry step should be present");
        let data = telemetry_step
            .data
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .expect("prompt telemetry step data should be JSON");

        assert_eq!(data["trace_kind"], "prompt_telemetry");
        assert_eq!(data["final_system_prompt_chars"], 1234);
        assert_eq!(data["sections"]["action_catalog"], 5678);
    }

    #[test]
    fn spine_trace_steps_preserve_arkdistill_telemetry_payload() {
        let steps = spine_trace_steps(&[SpineTraceEvent::ArkDistillTelemetry {
            data: serde_json::json!({
                "trace_kind": "arkdistill_telemetry",
                "primitive": "fetch",
                "action": "page_fetch",
                "original_chars": 12000,
                "distilled_chars": 3400,
                "saved_chars": 8600,
                "estimated_saved_tokens": 2150
            }),
        }]);

        let telemetry_step = steps
            .iter()
            .find(|step| step.title == "ArkDistill Context Savings")
            .expect("arkdistill telemetry step should be present");
        let data = telemetry_step
            .data
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .expect("arkdistill telemetry step data should be JSON");

        assert_eq!(data["trace_kind"], "arkdistill_telemetry");
        assert_eq!(data["primitive"], "fetch");
        assert_eq!(data["saved_chars"], 8600);
        assert_eq!(data["estimated_saved_tokens"], 2150);
    }

    #[test]
    fn spine_trace_steps_surface_completion_verification_progress() {
        let steps = spine_trace_steps(&[
            SpineTraceEvent::CompletionVerificationStarted {
                turn: 2,
                proposed_answer_chars: 512,
            },
            SpineTraceEvent::CompletionVerificationCompleted {
                turn: 2,
                complete: false,
            },
        ]);

        assert_eq!(steps[0].title, "Checking Final Answer");
        assert_eq!(steps[0].step_type, "info");
        assert_eq!(steps[1].title, "Final Answer Needs More Work");
        assert_eq!(steps[1].step_type, "warning");
    }

    #[test]
    fn spine_prompt_telemetry_includes_token_breakdown() {
        let prepared = PreparedSpineMessages {
            system_prompt: "saved context".to_string(),
            history: vec![ConversationMessage {
                role: "assistant".to_string(),
                content: "prior answer".to_string(),
                _timestamp: chrono::Utc::now(),
            }],
            user_message: "current request".to_string(),
        };
        let bundle = crate::core::self_evolve::PromptBundleProfile::default();
        let fragment_bundle =
            crate::core::model::prompt_fragments::default_prompt_fragment_bundle();
        let telemetry = build_spine_prompt_telemetry(
            CallerKind::Chat,
            &prepared,
            &bundle,
            &fragment_bundle,
            &ToolRegistry::new().schemas(),
        );

        assert!(
            telemetry["final_system_prompt_tokens"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(telemetry["history_prompt_tokens"].as_u64().unwrap_or(0) > 0);
        assert!(telemetry["user_prompt_tokens"].as_u64().unwrap_or(0) > 0);
        assert!(telemetry["tool_schema_tokens"].as_u64().unwrap_or(0) > 0);
        assert_eq!(
            telemetry["estimated_total_request_tokens"]
                .as_u64()
                .unwrap_or(0),
            telemetry["final_system_prompt_tokens"]
                .as_u64()
                .unwrap_or(0)
                + telemetry["history_prompt_tokens"].as_u64().unwrap_or(0)
                + telemetry["user_prompt_tokens"].as_u64().unwrap_or(0)
                + telemetry["tool_schema_tokens"].as_u64().unwrap_or(0)
        );
        assert_eq!(
            telemetry["spine_prompt_bundle_version"],
            SPINE_PROMPT_BUNDLE_VERSION
        );
        assert!(telemetry["prompt_fragment_version"]
            .as_str()
            .unwrap_or_default()
            .contains("prompt_fragments_v1"));
        assert!(
            telemetry["sections"]["spine.source_grounding_policy"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(telemetry["spine_prompt_fragments"]
            .as_array()
            .map(|fragments| fragments
                .iter()
                .any(|fragment| fragment["id"] == "spine.final_answer_policy"))
            .unwrap_or(false));
    }

    #[test]
    fn file_write_tool_result_is_sanitized_for_model() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "file_write",
                "status": "completed",
                "detail": "Written 12 bytes to /app/gpu-pricing/runbook.md.",
                "data": {
                    "payload": {
                        "kind": "resource",
                        "resource": {
                            "id": "file:abc",
                            "path": "/app/gpu-pricing/runbook.md",
                            "mime": "text/markdown",
                            "bytes": 12,
                            "created_at": "2026-05-20T00:00:00Z",
                            "source_action": "file_write"
                        }
                    },
                    "document": {
                        "id": "generated-file:abc:123",
                        "filename": "runbook.md",
                        "content_type": "text/markdown",
                        "chunk_count": 1,
                        "file_size": 12,
                        "url": "/ui/documents"
                    },
                    "write": {
                        "path": "/app/gpu-pricing/runbook.md",
                        "bytes": 12,
                        "content_type": "text/markdown"
                    }
                }
            })
        );
        let sanitized = spine_tool_result_value_for_model("file_write", "file_write", raw);
        let rendered = sanitized.to_string();

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["artifact"]["label"], "runbook.md");
        assert_eq!(sanitized["artifact"]["document"]["url"], "/ui/documents");
        assert!(rendered.contains("Documents"));
        assert!(!rendered.contains("/app/"));
        assert!(!rendered.contains("gpu-pricing/runbook.md"));
    }

    #[test]
    fn pdf_generate_tool_result_is_sanitized_for_model() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "pdf_generate",
                "status": "completed",
                "detail": "Saved managed file report.pdf.",
                "data": {
                    "payload": {
                        "kind": "resource",
                        "resource": {
                            "id": "file:abc",
                            "path": "/app/data/outputs/report.pdf",
                            "mime": "application/pdf",
                            "bytes": 1200,
                            "created_at": "2026-05-20T00:00:00Z",
                            "source_action": "pdf_generate"
                        }
                    },
                    "artifact": {
                        "kind": "managed_file",
                        "label": "report.pdf",
                        "bytes": 1200,
                        "content_type": "application/pdf",
                        "download_url": "/api/outputs/0185f5e8-9694-454f-b0d3-42c83fbba585/report.pdf/download"
                    },
                    "document": {
                        "id": "generated-file:abc:123",
                        "filename": "report.pdf",
                        "content_type": "application/pdf",
                        "chunk_count": 1,
                        "file_size": 1200,
                        "url": "/ui/documents"
                    }
                }
            })
        );
        let sanitized = spine_tool_result_value_for_model("pdf_generate", "pdf_generate", raw);
        let rendered = sanitized.to_string();

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["tool"], "pdf_generate");
        assert_eq!(sanitized["artifact"]["label"], "report.pdf");
        assert_eq!(
            sanitized["artifact"]["download_url"],
            "/api/outputs/0185f5e8-9694-454f-b0d3-42c83fbba585/report.pdf/download"
        );
        assert_eq!(sanitized["artifact"]["document"]["url"], "/ui/documents");
        assert!(rendered.contains("Documents"));
        assert!(!rendered.contains("/app/data/outputs"));
    }

    #[test]
    fn service_manage_tool_result_hides_access_secrets() {
        let raw = serde_json::json!({
            "status": "deployed",
            "type": "static",
            "app_id": "abc12345",
            "url": "/apps/abc12345/",
            "access_url": "/apps/abc12345/?grant=secret",
            "title": "GPU Pricing",
            "access_key": "top-secret",
            "access_password": "top-secret"
        })
        .to_string();
        let sanitized = spine_tool_result_value_for_model("resource_rw", "service_manage", raw);
        let rendered = sanitized.to_string();

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["app"]["url"], "/apps/abc12345/?grant=secret");
        assert!(!rendered.contains("top-secret"));
        assert!(!rendered.contains("access_password"));
        assert!(!rendered.contains("access_key"));
    }

    #[test]
    fn app_deploy_tool_result_preserves_quality_check_for_model() {
        let raw = serde_json::json!({
            "status": "deployed",
            "type": "static",
            "app_id": "abc12345",
            "url": "/apps/abc12345/",
            "title": "Generated App",
            "quality_check": {
                "status": "concerns",
                "needs_repair": true,
                "judge_passed": false,
                "judge_summary": "Requested persistence was not evidenced.",
                "concerns": ["Requested persistence was not evidenced."]
            }
        })
        .to_string();
        let sanitized = spine_tool_result_value_for_model("app_deploy", "app_deploy", raw);

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["tool"], "app_deploy");
        assert_eq!(sanitized["quality_check"]["needs_repair"], true);
        assert_eq!(sanitized["quality_check"]["judge_passed"], false);
    }

    #[test]
    fn service_manage_list_result_preserves_empty_registry_for_model() {
        let raw = serde_json::json!({
            "status": "ok",
            "tool": "service_manage",
            "services": []
        })
        .to_string();
        let sanitized = spine_tool_result_value_for_model("resource_rw", "service_manage", raw);

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["status"], "empty");
        assert_eq!(sanitized["service_count"], 0);
        assert_eq!(sanitized["services"].as_array().unwrap().len(), 0);
        assert_eq!(sanitized["terminal_observation"], true);
        assert!(sanitized["assistant_instruction"]
            .as_str()
            .unwrap_or_default()
            .contains("answer from this result"));
    }

    #[test]
    fn service_manage_completion_marker_preserves_lifecycle_result_for_model() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "service_manage",
                "status": "completed",
                "detail": "Listed services.",
                "data": {
                    "status": "ok",
                    "services": []
                }
            })
        );
        let sanitized = spine_tool_result_value_for_model("resource_rw", "service_manage", raw);

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["status"], "empty");
        assert_eq!(sanitized["terminal_observation"], true);
        assert_eq!(sanitized["services"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn service_manage_status_result_preserves_not_found_for_model() {
        let raw = serde_json::json!({
            "status": "not_found",
            "tool": "service_manage",
            "service_id": null,
            "query": "GPU pricing comparison",
            "services": []
        })
        .to_string();
        let sanitized = spine_tool_result_value_for_model("resource_rw", "service_manage", raw);

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["status"], "not_found");
        assert_eq!(sanitized["service_count"], 0);
        assert_eq!(sanitized["query"], "GPU pricing comparison");
        assert_eq!(sanitized["terminal_observation"], true);
        assert!(sanitized["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not found"));
    }

    #[test]
    fn service_manage_status_result_preserves_current_service_without_secrets() {
        let raw = serde_json::json!({
            "status": "ok",
            "tool": "service_manage",
            "service_id": "abc12345",
            "service": {
                "id": "abc12345",
                "title": "GPU Pricing",
                "url": "/apps/abc12345/",
                "access_url": "/apps/abc12345/?grant=secret",
                "enabled": true,
                "running": true,
                "access_key": "top-secret",
                "access_password": "top-secret"
            }
        })
        .to_string();
        let sanitized = spine_tool_result_value_for_model("resource_rw", "service_manage", raw);
        let rendered = sanitized.to_string();

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["status"], "ok");
        assert_eq!(sanitized["service"]["id"], "abc12345");
        assert_eq!(sanitized["service"]["title"], "GPU Pricing");
        assert_eq!(sanitized["service"]["url"], "/apps/abc12345/?grant=secret");
        assert!(!rendered.contains("top-secret"));
        assert!(!rendered.contains("access_password"));
        assert!(!rendered.contains("access_key"));
    }

    #[test]
    fn structured_fetch_result_is_not_double_wrapped_for_model() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "page_fetch",
                "status": "completed",
                "detail": "Fetched readable content.",
                "data": {
                    "url": "https://example.com",
                    "content": "Readable page text"
                }
            })
        );
        let sanitized = spine_tool_result_value_for_model("fetch", "page_fetch", raw);
        let rendered = sanitized.to_string();

        assert_eq!(sanitized["ok"], true);
        assert_eq!(sanitized["primitive"], "fetch");
        assert_eq!(sanitized["data"]["content"], "Readable page text");
        assert!(!rendered.contains(crate::runtime::TOOL_COMPLETION_MARKER));
    }

    #[test]
    fn arkdistill_compacts_noisy_tool_output_for_model_context() {
        let noisy = (0..500)
            .map(|_| "same noisy browser line with repeated navigation chrome")
            .collect::<Vec<_>>()
            .join("\n");
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "page_fetch",
                "status": "completed",
                "data": {
                    "url": "https://example.com",
                    "content": noisy,
                    "status": "ok"
                }
            })
        );

        let output = spine_tool_result_output_for_model("fetch", "page_fetch", raw);
        let rendered = output.value.to_string();

        assert!(output.stats.saved_chars > 0);
        assert!(output.stats.estimated_saved_tokens > 0);
        assert!(rendered.len() < 7_500);
        assert_eq!(output.value["data"]["url"], "https://example.com");
        assert_eq!(output.value["data"]["status"], "ok");
        assert!(rendered.contains("omitted"));
    }

    #[test]
    fn arkdistill_omits_blob_payloads_with_size_metadata() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "browser_snapshot",
                "status": "completed",
                "data": {
                    "screenshot_base64": "A".repeat(12_000),
                    "body_text": "Loaded page"
                }
            })
        );

        let output = spine_tool_result_output_for_model("browse", "browser_snapshot", raw);
        let rendered = output.value.to_string();

        assert!(output.stats.saved_chars > 5_000);
        assert!(!rendered.contains(&"A".repeat(256)));
        assert!(rendered.contains("base64 omitted"));
    }

    #[test]
    fn prepared_messages_preserve_redacted_internal_tool_context_without_raw_json() {
        let large_content = "x".repeat(1200);
        let messages = vec![
            SpineMessage::System {
                content: "system".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Let me save that.".to_string()),
                tool_calls: vec![SpineToolCall {
                    id: "call_1".to_string(),
                    name: "file_write".to_string(),
                    arguments: serde_json::json!({
                        "path": "reports/gpu-pricing/index.html",
                        "content": large_content,
                        "headers": {"authorization": "Bearer secret-token"}
                    }),
                    activity_label: None,
                }],
            },
            SpineMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: "{\"ok\":true}".to_string(),
            },
        ];
        let prepared = prepare_spine_messages_for_llm(&messages);
        let joined = prepared
            .history
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let marker_sample =
            crate::core::model::llm_context_sanitizer::wrap_internal_tool_context("sample");
        let marker = marker_sample.lines().next().unwrap_or_default();
        assert!(joined.contains("Let me save that."));
        assert!(joined.contains(marker));
        assert!(joined.contains("file_write"));
        assert!(joined.contains("reports/gpu-pricing/index.html"));
        assert!(joined.contains("chars: 1200"));
        assert!(prepared.user_message.contains(marker));
        assert!(!joined.contains(&"x".repeat(300)));
        assert!(!joined.contains("`call_1` called"));
        assert!(!joined.contains("secret-token"));
        assert!(!joined.contains("\"name\":\"file_write\""));
        assert!(!prepared.user_message.contains("\"name\":\"file_write\""));
    }

    #[test]
    fn resource_file_create_with_content_requires_direct_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {"path": "notes.txt", "content": "hello"}
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_write"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_write");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn search_primitive_preserves_deep_research_depth() {
        let plan = plan_search(&serde_json::json!({
            "query": "India AI compute policy",
            "depth": "deep",
            "limit": 18
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "research");
                assert_eq!(actions[0].arguments["query"], "India AI compute policy");
                assert_eq!(actions[0].arguments["depth"], "deep");
                assert_eq!(actions[0].arguments["max_sources"], 18);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_file_delete_routes_to_file_delete() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "delete",
            "kind": "file",
            "id": "reports/old.md"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "file_delete");
                assert_eq!(actions[0].arguments["path"], "reports/old.md");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_activity_read_routes_to_ark_inspect_activity_surface() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "read",
            "kind": "activity",
            "limit": 8
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "ark_inspect");
                assert_eq!(actions[0].arguments["operation"], "surface");
                assert_eq!(actions[0].arguments["surface"], "activity");
                assert_eq!(actions[0].arguments["limit"], 8);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_schema_exposes_activity_as_readable_resource_kind() {
        let schema = ToolRegistry::new()
            .schemas()
            .into_iter()
            .find(|schema| schema.name == "resource_rw")
            .expect("resource_rw schema should exist");

        let kinds = schema.input_schema["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind enum should be present");

        assert!(kinds.iter().any(|kind| kind.as_str() == Some("activity")));
        assert!(kinds
            .iter()
            .any(|kind| kind.as_str() == Some("notification")));

        let kind_description = schema.input_schema["properties"]["kind"]["description"]
            .as_str()
            .expect("kind description should be present");
        assert!(kind_description.contains("backend resource adapter table"));
        assert!(kind_description.contains("app_deploy"));

        let ops = schema.input_schema["properties"]["op"]["enum"]
            .as_array()
            .expect("op enum should be present");
        assert!(ops.iter().any(|op| op.as_str() == Some("create")));
        assert!(ops.iter().any(|op| op.as_str() == Some("delete")));
        assert!(ops.iter().any(|op| op.as_str() == Some("connect")));
    }

    #[test]
    fn primitive_schema_exposes_direct_artifact_tools() {
        let names = ToolRegistry::new()
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect::<std::collections::BTreeSet<_>>();
        for expected in [
            "app_deploy",
            "file_read",
            "file_search",
            "file_write",
            "file_patch",
            "file_delete",
            "skill_manage",
        ] {
            assert!(names.contains(expected), "missing primitive {expected}");
        }
    }

    #[test]
    fn primitive_app_deploy_schema_exposes_acceptance_contract() {
        let schema = ToolRegistry::new()
            .schemas()
            .into_iter()
            .find(|schema| schema.name == "app_deploy")
            .expect("app_deploy schema should exist");
        let properties = schema.input_schema["properties"]
            .as_object()
            .expect("app_deploy properties should be an object");

        assert!(properties.contains_key("request_context"));
        assert!(properties.contains_key("acceptance_criteria"));
    }

    #[test]
    fn resource_file_create_without_path_still_requires_direct_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "id": "reports/comparison.html",
            "content": {"content": "<!doctype html><title>Report</title>"}
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_write"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_write");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_file_create_without_body_still_requires_direct_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {
                "path": "reports/comparison.html",
                "description": "HTML report to save"
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_write"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_write");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_html_file_create_requires_direct_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {
                "path": "reports/comparison.html",
                "content_type": "text/html",
                "content": "<!doctype html><html><body>Report</body></html>",
                "title": "Comparison Report"
            },
            "metadata": {
                "source_url": "https://example.com/pricing"
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_write"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_write");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_html_path_without_html_document_requires_direct_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {
                "path": "reports/notes.html",
                "content_type": "text/html",
                "content": "# Notes\n\nThis is not a browser document."
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_write"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_write");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_markdown_file_create_requires_direct_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {
                "path": "reports/runbook.md",
                "content": "# Runbook\n\nSteps."
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_write"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_write");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn needs_input_tool_result_has_terminal_user_message() {
        let result = ToolResult::from_value(
            true,
            serde_json::json!({
                "ok": true,
                "primitive": "browse",
                "tool": "browser_auto",
                "status": "needs_input",
                "detail": "Should I inspect the History section or the Products section?",
                "data": {
                    "session": {
                        "status": "waiting_for_operator",
                        "question": "Should I inspect the History section or the Products section?"
                    }
                }
            }),
        );

        let message = needs_input_message_from_tool_results(&[result])
            .expect("needs-input tool result should stop the spine turn");

        assert_eq!(
            message,
            "Should I inspect the History section or the Products section?"
        );
    }

    #[test]
    fn needs_input_detail_without_structured_question_does_not_terminalize() {
        let result = ToolResult::from_value(
            true,
            serde_json::json!({
                "ok": true,
                "primitive": "resource_rw",
                "tool": "watch",
                "status": "needs_input",
                "detail": "Watcher requires poll_action/poll_arguments or a script so it knows what to poll.",
                "data": {
                    "reason": "missing_poll_source"
                }
            }),
        );

        assert!(needs_input_message_from_tool_results(&[result]).is_none());
    }

    #[test]
    fn credential_handoff_tool_result_terminalizes_without_literal_question() {
        let result = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "needs_credentials",
                "message": "Custom API integration saved. Credentials are still required through the secure credential form or Settings > Integrations > Custom APIs > Acme.",
                "credential_request": {
                    "kind": "custom_api_auth",
                    "api_id": "acme",
                    "api_name": "Acme",
                    "settings_path": "Settings > Integrations > Custom APIs > Acme",
                    "secure_input_required": true
                },
                "secure_credential_prompt_pending": true
            }),
        );

        let message = needs_input_message_from_tool_results(&[result])
            .expect("credential handoff should stop the spine turn");

        assert!(message.contains("secure credential form"));
        assert!(message.contains("Settings > Integrations > Custom APIs > Acme"));
    }

    #[test]
    fn credential_handoff_terminalization_is_not_tied_to_known_provider_or_kind() {
        let result = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "needs_credentials",
                "credential_request": {
                    "kind": "future_secure_handoff",
                    "settings_path": "Settings > External Capabilities > Example",
                    "secure_input_required": true
                }
            }),
        );

        let message = needs_input_message_from_tool_results(&[result.clone()])
            .expect("any secure credential handoff should stop the spine turn");

        assert!(message.contains("secure credential form"));
        assert!(message.contains("Settings > External Capabilities > Example"));
        assert!(!tool_result_requires_model_repair(&result));
    }

    #[test]
    fn final_user_text_strips_internal_markers_and_identifier_shapes() {
        let finalized = finalize_user_text(
            SpineTerminalTextKind::Blocked,
            concat!(
                "__TOOL_COMPLETION__:{\"tool\":\"watch\",\"status\":\"failed\"}\n",
                "Failed inside durable_orchestration_action_result with poll_action_missing."
            ),
            TerminalTextProvenance::ModelAuthored,
        );

        assert!(!finalized.contains("__TOOL_COMPLETION__"));
        assert!(!finalized.contains("durable_orchestration_action_result"));
        assert!(!finalized.contains("poll_action_missing"));
        assert!(!finalized.trim().is_empty());
    }

    #[test]
    fn resource_rw_schema_teaches_durable_argument_contracts_upfront() {
        // The model must see the full durable-action contract in the tool
        // schema itself, so the first call can be correct for any wording.
        let schemas = build_primitive_schemas();
        let resource_rw = schemas
            .iter()
            .find(|schema| schema.name == "resource_rw")
            .expect("resource_rw primitive schema");
        let content_description = resource_rw.input_schema["properties"]["content"]["description"]
            .as_str()
            .expect("content description");

        for taught in ["cron", "local_time", "items", "condition", "on_trigger"] {
            assert!(
                content_description.contains(taught),
                "resource_rw content schema must teach `{taught}`"
            );
        }
        assert!(resource_rw
            .description
            .contains("condition/change monitoring"));
        assert!(resource_rw
            .description
            .contains("pure time-based reminders"));
        assert!(content_description.contains("ownership"));
        assert!(content_description.contains("action/script"));
    }

    #[test]
    fn spine_prompt_teaches_compact_durable_contracts() {
        let prompt = build_spine_system_prompt("", None, None);

        assert!(
            prompt.contains("field schemas and validation errors are the full contract source"),
            "spine prompt must point full field shape to schema and validation"
        );
        assert!(
            prompt.contains("Use cron for recurring scheduled_task cadences"),
            "spine prompt must not imply local_time replaces cron for recurrence"
        );
        assert!(
            prompt.contains("Use watcher for autonomous condition/change monitoring"),
            "spine prompt must teach durable-kind ownership by semantic contract"
        );
        assert!(
            prompt.contains("Use scheduled_task for pure time-based reminders"),
            "spine prompt must teach the scheduled_task ownership boundary"
        );
        assert!(
            !prompt.contains("\"batch_example\""),
            "spine prompt must not embed the full scheduled_task JSON contract"
        );
    }

    #[test]
    fn resource_rw_content_schema_exposes_watcher_condition_shape() {
        let schemas = build_primitive_schemas();
        let resource_rw = schemas
            .iter()
            .find(|schema| schema.name == "resource_rw")
            .expect("resource_rw primitive schema");
        let condition =
            &resource_rw.input_schema["properties"]["content"]["properties"]["condition"];

        assert_eq!(condition["type"], "object");
        assert_eq!(
            condition["required"],
            serde_json::json!(["description", "evaluation_mode", "type"])
        );
        assert!(condition["properties"]["evaluation_mode"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("change"));
        assert!(condition["properties"]["type"]["enum"]
            .as_array()
            .expect("condition type enum")
            .iter()
            .any(|value| value == "llm"));
    }

    #[test]
    fn resource_rw_content_schema_exposes_registered_resource_contract_fields() {
        let schemas = build_primitive_schemas();
        let resource_rw = schemas
            .iter()
            .find(|schema| schema.name == "resource_rw")
            .expect("resource_rw primitive schema");
        let content_properties = resource_rw.input_schema["properties"]["content"]["properties"]
            .as_object()
            .expect("resource_rw content properties");

        let expected_fields = [
            "message",
            "title",
            "delivery_channel",
            "channel",
            "name",
            "id",
            "base_url",
            "path",
            "openapi_url",
            "openapi_text",
            "docs_url",
            "docs_text",
            "operation",
            "operations",
            "auth",
            "auth_type",
            "auth_mode",
            "auth_profile_id",
            "send",
            "pack_id",
            "source_url",
            "source_path",
            "manifest_text",
            "manifest",
            "url",
            "command",
            "transport",
        ];

        for field in expected_fields {
            let field_schema = content_properties
                .get(field)
                .unwrap_or_else(|| panic!("resource_rw content schema must expose `{field}`"));
            assert!(
                field_schema
                    .get("description")
                    .and_then(|value| value.as_str())
                    .is_some_and(|description| !description.trim().is_empty()),
                "resource_rw content field `{field}` must include a description"
            );
        }
    }

    #[test]
    fn memory_rw_content_schema_exposes_mutation_contract_fields() {
        let schemas = build_primitive_schemas();
        let memory_rw = schemas
            .iter()
            .find(|schema| schema.name == "memory_rw")
            .expect("memory_rw primitive schema");
        let content_properties = memory_rw.input_schema["properties"]["content"]["properties"]
            .as_object()
            .expect("memory_rw content properties");

        for field in ["key", "value", "text", "kind"] {
            let field_schema = content_properties
                .get(field)
                .unwrap_or_else(|| panic!("memory_rw content schema must expose `{field}`"));
            assert!(
                field_schema
                    .get("description")
                    .and_then(|value| value.as_str())
                    .is_some_and(|description| !description.trim().is_empty()),
                "memory_rw content field `{field}` must include a description"
            );
        }
    }

    #[test]
    fn next_repair_hint_promotes_durable_contract_and_instruction() {
        let value = serde_json::json!({
            "tool": "schedule_task",
            "status": "failed",
            "data": {
                "recoverable_by_model": true,
                "violations": ["`task` is required."],
                "expected_contract": { "required": "`task` plus one schedule source" },
                "assistant_instruction": "Satisfy the full contract in one corrected call."
            }
        });

        let hint = next_repair_hint_from_tool_value(&value);

        assert_eq!(
            hint["hint"],
            "Satisfy the full contract in one corrected call."
        );
        assert!(hint["remediation"]["required"]
            .as_str()
            .unwrap_or_default()
            .contains("task"));
    }

    #[test]
    fn final_user_text_preserves_markdown_structure_in_completed_answers() {
        let answer = concat!(
            "# Plan\n\n",
            "First paragraph of the answer.\n\n",
            "- bullet one\n",
            "- bullet two\n\n",
            "```rust\nfn main() {}\n```\n\n",
            "Done."
        );

        let finalized = finalize_user_text(
            SpineTerminalTextKind::Completed,
            answer,
            TerminalTextProvenance::ModelAuthored,
        );

        assert_eq!(finalized, answer);
    }

    #[test]
    fn final_user_text_collapses_blank_runs_but_keeps_line_breaks() {
        let finalized = finalize_user_text(
            SpineTerminalTextKind::Completed,
            "Para one.\n\n\n\nPara two.   \nLine three.",
            TerminalTextProvenance::ModelAuthored,
        );

        assert_eq!(finalized, "Para one.\n\nPara two.\nLine three.");
    }

    #[test]
    fn tool_repair_gate_structured_failure_requires_recoverable_flag() {
        let recoverable = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "failed",
                "detail": "Watcher setup payload was incomplete.",
                "recoverable_by_model": true
            }),
        );
        let retryable = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "failed",
                "detail": "Transient provider failure.",
                "retryable": true
            }),
        );
        let hard_failure = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "failed",
                "detail": "The action is not permitted."
            }),
        );

        assert!(tool_result_requires_model_repair(&recoverable));
        assert!(tool_result_requires_model_repair(&retryable));
        assert!(!tool_result_requires_model_repair(&hard_failure));
    }

    #[test]
    fn tool_repair_gate_finds_recoverable_flag_in_nested_data() {
        let result = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "failed",
                "detail": "Watcher setup payload was incomplete.",
                "data": {
                    "reason": "missing_poll_target",
                    "recoverable_by_model": true
                }
            }),
        );

        assert!(tool_result_requires_model_repair(&result));
    }

    #[test]
    fn tool_repair_gate_needs_input_depends_on_structured_question() {
        let with_question = ToolResult::from_value(
            true,
            serde_json::json!({
                "status": "needs_input",
                "data": { "session": { "question": "Which account should I monitor?" } }
            }),
        );
        let without_question = ToolResult::from_value(
            true,
            serde_json::json!({
                "status": "needs_input",
                "detail": "Watcher requires poll_action/poll_arguments or a script."
            }),
        );

        assert!(!tool_result_requires_model_repair(&with_question));
        assert!(tool_result_requires_model_repair(&without_question));
    }

    #[test]
    fn tool_repair_gate_structured_success_never_repairs() {
        let result = ToolResult::from_value(false, serde_json::json!({ "success": true }));

        assert!(!tool_result_requires_model_repair(&result));
    }

    #[test]
    fn tool_repair_gate_unstructured_value_falls_back_to_transport_ok() {
        let failed = ToolResult::from_value(false, serde_json::json!({ "detail": "boom" }));
        let succeeded = ToolResult::from_value(true, serde_json::json!({ "detail": "all good" }));

        assert!(tool_result_requires_model_repair(&failed));
        assert!(!tool_result_requires_model_repair(&succeeded));
    }

    #[test]
    fn tool_repair_budget_fires_own_voice_exhaustion_after_budget_spent() {
        let mut attempts: HashMap<String, usize> = HashMap::new();

        for attempt in 1..=MAX_TOOL_REPAIR_ATTEMPTS {
            assert!(
                note_tool_repair_attempt(&mut attempts, "resource_rw").is_none(),
                "attempt {attempt} is within budget and stays silent"
            );
        }
        let message = note_tool_repair_attempt(&mut attempts, "resource_rw")
            .expect("the failure after the budget injects the own-voice instruction");

        assert!(message.contains("reached its repair limit"));
        assert!(message.contains("in your own words"));
        assert!(!message.contains(crate::runtime::TOOL_COMPLETION_MARKER));

        // The budget is tracked per tool name; another tool starts fresh.
        assert!(note_tool_repair_attempt(&mut attempts, "communicate").is_none());
    }

    #[test]
    fn final_user_text_rejects_tool_origin_prose_without_question_provenance() {
        let finalized = finalize_user_text(
            SpineTerminalTextKind::NeedsInput,
            "Watcher requires poll_action/poll_arguments or a script so it knows what to poll.",
            TerminalTextProvenance::ToolOrigin,
        );

        assert_ne!(
            finalized,
            "Watcher requires poll_action/poll_arguments or a script so it knows what to poll."
        );
        assert!(!finalized.contains("poll_action"));
        assert!(!finalized.trim().is_empty());
    }

    #[test]
    fn final_user_text_preserves_structured_user_questions() {
        let finalized = finalize_user_text(
            SpineTerminalTextKind::NeedsInput,
            "Which account should I monitor?",
            TerminalTextProvenance::StructuredUserQuestion,
        );

        assert_eq!(finalized, "Which account should I monitor?");
    }

    #[test]
    fn failed_search_tool_result_surfaces_backend_unavailable_message() {
        let results = vec![ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "error",
                "tool": "search",
                "domain": crate::actions::ActionErrorDomain::Search.as_key(),
                "reason": "failed",
                "message": "DuckDuckGo backend failed: challenge page",
            }),
        )];

        let message = failed_search_message_from_tool_results(&results)
            .expect("failed search evidence should stop the turn");

        assert!(message.contains("No search backend is currently available in AgentArk right now"));
        assert!(message.contains("free anonymous DuckDuckGo/browser search"));
        assert!(message.contains("best-effort and not always reliable"));
        assert!(message.contains("SearXNG"));
        assert!(message.contains("Serper"));
        assert!(message.contains("The free built-in search fallback failed"));
        assert!(message.contains("Configure an API-backed search provider or SearXNG"));
        assert!(!message.contains("Internal provider details were withheld"));
        assert!(!message.contains("DuckDuckGo backend failed: challenge page"));
    }

    #[test]
    fn failed_search_tool_result_uses_structured_domain_not_message_text() {
        let search_result = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "error",
                "domain": crate::actions::ActionErrorDomain::Search.as_key(),
                "reason": "failed",
                "message": "temporary provider failure",
            }),
        );
        let non_search_result = ToolResult::from_value(
            false,
            serde_json::json!({
                "status": "error",
                "domain": crate::actions::ActionErrorDomain::Action.as_key(),
                "reason": "failed",
                "message": crate::actions::search::SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE,
            }),
        );

        assert!(failed_search_message_from_tool_results(&[search_result]).is_some());
        assert!(failed_search_message_from_tool_results(&[non_search_result]).is_none());
    }

    #[test]
    fn structured_failed_browser_tool_result_is_not_marked_ok_for_model() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "browser_auto",
                "status": "failed",
                "detail": "The browser task failed.",
                "data": {
                    "success": false,
                    "reason": "browser_loading_error",
                    "session": {
                        "status": "failed",
                        "reason": "The page did not load."
                    }
                }
            })
        );

        let sanitized = spine_tool_result_value_for_model("browse", "browser_auto", raw);

        assert_eq!(sanitized["ok"], false);
        assert_eq!(sanitized["status"], "failed");
    }

    #[test]
    fn action_invocation_success_uses_structured_completion_state() {
        let raw = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "watch",
                "status": "failed",
                "detail": "Watcher setup payload was incomplete.",
                "data": {
                    "success": false,
                    "durable_commit": false,
                    "reason": "missing_poll_target",
                    "recoverable_by_model": true
                }
            })
        );
        let value = spine_raw_tool_result_value_for_model("resource_rw", "watch", raw);

        assert!(!action_invocation_value_reports_success(&value));
    }

    #[test]
    fn browse_snapshot_maps_to_browser_auto_without_requiring_new_task() {
        let plan = plan_browse(&serde_json::json!({
            "action": "snapshot",
            "session_id": "session-1"
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "browser_auto");
                assert_eq!(actions[0].arguments["action"], "snapshot");
                assert_eq!(actions[0].arguments["session_id"], "session-1");
                assert!(actions[0].arguments.get("task").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn browse_resume_handoff_maps_user_note_to_browser_auto() {
        let plan = plan_browse(&serde_json::json!({
            "action": "resume_handoff",
            "session_id": "session-1",
            "note": "Use the History section"
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "browser_auto");
                assert_eq!(actions[0].arguments["action"], "resume_handoff");
                assert_eq!(actions[0].arguments["session_id"], "session-1");
                assert_eq!(actions[0].arguments["note"], "Use the History section");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_browser_profile_list_maps_to_profile_manager() {
        let plan = plan_resource_rw(&serde_json::json!({
            "kind": "browser_profile",
            "op": "list"
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "browser_profile_manage");
                assert_eq!(actions[0].arguments["operation"], "list");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_browser_profile_update_preserves_typed_resource_fields() {
        let plan = plan_resource_rw(&serde_json::json!({
            "kind": "browser_profile",
            "op": "update",
            "profile_id": "profile-1",
            "name": "Debanka",
            "enabled": true,
            "metadata": {
                "browser": "chrome",
                "managed": false
            }
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "browser_profile_manage");
                assert_eq!(actions[0].arguments["operation"], "update");
                assert_eq!(actions[0].arguments["profile_id"], "profile-1");
                assert_eq!(actions[0].arguments["name"], "Debanka");
                assert_eq!(actions[0].arguments["enabled"], true);
                assert_eq!(actions[0].arguments["metadata"]["browser"], "chrome");
                assert_eq!(actions[0].arguments["metadata"]["managed"], false);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn code_exec_requires_language_for_inline_code() {
        let plan = plan_code_exec(&serde_json::json!({
            "code": "print('hello')"
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("language"));
                assert_eq!(extra.unwrap()["field"], "language");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn pdf_generate_maps_to_managed_pdf_action() {
        let plan = plan_pdf_generate(&serde_json::json!({
            "title": "Market research",
            "filename": "market-research.pdf",
            "style": "report",
            "content": "Executive summary\n\nFindings."
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "pdf_generate");
                assert_eq!(actions[0].arguments["title"], "Market research");
                assert_eq!(actions[0].arguments["filename"], "market-research.pdf");
                assert_eq!(actions[0].arguments["style"], "report");
                assert_eq!(actions[0].arguments["document_visible"], true);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn direct_app_deploy_maps_to_app_deploy_action() {
        let plan = plan_direct_action(
            "app_deploy",
            &serde_json::json!({
                "title": "Demo",
                "files": {"index.html": "<!doctype html><html><body>Demo</body></html>"}
            }),
        );
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "app_deploy");
                assert_eq!(actions[0].arguments["title"], "Demo");
                assert_eq!(
                    actions[0].arguments["files"]["index.html"],
                    "<!doctype html><html><body>Demo</body></html>"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn direct_file_write_preserves_root_path_and_content() {
        let plan = plan_direct_action(
            "file_write",
            &serde_json::json!({
                "path": "reports/notes.md",
                "content": "# Notes\n\nCurrent findings.",
                "document_visible": true
            }),
        );
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "file_write");
                assert_eq!(actions[0].arguments["path"], "reports/notes.md");
                assert_eq!(
                    actions[0].arguments["content"],
                    "# Notes\n\nCurrent findings."
                );
                assert_eq!(actions[0].arguments["document_visible"], true);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_file_batch_patch_requires_direct_file_patch() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "update",
            "kind": "file",
            "content": {
                "patches": [
                    {"path": "reports/comparison.html", "patch": "@@\n-old\n+new\n"}
                ]
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("file_patch"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "file_patch");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_create_with_source_is_rejected() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "GPU pricing comparison",
                "files": {"index.html": "<!doctype html><title>GPU pricing</title>"}
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("app_deploy"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "app_deploy");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_api_create_maps_to_capability_acquire() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "custom_api",
            "content": {
                "name": "provider leads",
                "description": "Read leads from a provider API",
                "base_url": "https://api.example.com",
                "path": "/leads",
                "auth_type": "bearer"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "capability_acquire");
                assert_eq!(actions[0].arguments["base_url"], "https://api.example.com");
                assert_eq!(actions[0].arguments["auth_type"], "bearer");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_api_install_and_connect_map_to_capability_acquire() {
        for op in ["install", "connect"] {
            let plan = plan_resource_rw(&serde_json::json!({
                "op": op,
                "kind": "custom_api",
                "content": {
                    "name": "provider work items",
                    "description": "Read work items from a provider API",
                    "base_url": "https://api.example.com",
                    "path": "/graphql",
                    "auth_type": "bearer"
                }
            }));
            match plan {
                PrimitivePlan::Actions(actions) => {
                    assert_eq!(actions[0].action_name, "capability_acquire");
                    assert_eq!(actions[0].arguments["base_url"], "https://api.example.com");
                    assert_eq!(actions[0].arguments["path"], "/graphql");
                    assert_eq!(actions[0].arguments["auth_type"], "bearer");
                    assert!(actions[0].arguments.get("kind").is_none());
                }
                other => panic!("unexpected plan for {op}: {other:?}"),
            }
        }
    }

    #[test]
    fn resource_custom_api_install_without_spec_routes_to_capability_resolution() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "install",
            "kind": "custom_api",
            "query": "project management provider"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "capability_resolve");
                assert_eq!(
                    actions[0].arguments["selected_action"],
                    "capability_acquire"
                );
                assert_eq!(
                    actions[0].arguments["requested_capability"],
                    "project management provider"
                );
                assert!(actions[0].arguments["failure_output"]
                    .as_str()
                    .unwrap()
                    .contains("missing structured resource fields"));
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_api_update_preserves_target_id() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "update",
            "kind": "custom_api",
            "id": "provider-api",
            "content": {
                "method": "post",
                "path": "/graphql",
                "default_headers": {
                    "content-type": "application/json"
                }
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "capability_acquire");
                assert_eq!(actions[0].arguments["id"], "provider-api");
                assert_eq!(actions[0].arguments["method"], "post");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn custom_api_update_progress_identity_includes_payload_shape() {
        let get_call = SpineToolCall {
            id: "call_1".to_string(),
            name: "resource_rw".to_string(),
            arguments: serde_json::json!({
                "kind": "custom_api",
                "op": "update",
                "id": "provider-api",
                "content": {
                    "method": "get",
                    "path": "/graphql"
                }
            }),
            activity_label: None,
        };
        let post_call = SpineToolCall {
            id: "call_2".to_string(),
            name: "resource_rw".to_string(),
            arguments: serde_json::json!({
                "kind": "custom_api",
                "op": "update",
                "id": "provider-api",
                "content": {
                    "method": "post",
                    "path": "/graphql"
                }
            }),
            activity_label: None,
        };

        assert_ne!(
            tool_call_progress_identity(&get_call),
            tool_call_progress_identity(&post_call)
        );
    }

    #[test]
    fn fetch_custom_api_maps_to_saved_custom_api_request() {
        let plan = plan_fetch(&serde_json::json!({
            "integration": "custom_api",
            "id": "provider-api",
            "operation": "post-graphql",
            "content": {
                "body": {
                    "query": "query Probe { __typename }"
                }
            }
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "custom_api_request");
                assert_eq!(actions[0].arguments["id"], "provider-api");
                assert_eq!(actions[0].arguments["operation"], "post-graphql");
                assert_eq!(
                    actions[0].arguments["body"]["query"],
                    "query Probe { __typename }"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn fetch_post_forwards_body_headers_and_persistence_contract() {
        let plan = plan_fetch(&serde_json::json!({
            "url": "https://api.example.com/register",
            "method": "POST",
            "headers": { "Content-Type": "application/json" },
            "body": { "model": "agentark" },
            "persist_response": [{
                "response_path": "token",
                "secret_key": "provider:token",
                "sensitive": true
            }],
            "timeout_secs": 20
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "http_request");
                assert_eq!(
                    actions[0].arguments["url"],
                    "https://api.example.com/register"
                );
                assert_eq!(actions[0].arguments["method"], "POST");
                assert_eq!(
                    actions[0].arguments["headers"]["Content-Type"],
                    "application/json"
                );
                assert_eq!(actions[0].arguments["body"]["model"], "agentark");
                assert_eq!(
                    actions[0].arguments["persist_response"][0]["secret_key"],
                    "provider:token"
                );
                assert_eq!(actions[0].arguments["timeout_secs"], 20);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn fetch_resource_request_routes_to_http_request_without_requiring_user_path() {
        let plan = plan_fetch(&serde_json::json!({
            "url": "https://example.test/files/guide.md",
            "as_resource": true,
            "suggested_name": "guide.md"
        }));

        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "http_request");
                assert_eq!(actions[0].arguments["as_resource"], true);
                assert_eq!(actions[0].arguments["suggested_name"], "guide.md");
                assert!(actions[0].arguments.get("save_to").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_api_test_maps_to_inspect_integration_with_run_check() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "test",
            "kind": "custom_api",
            "id": "provider-api"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "inspect_integration");
                assert_eq!(actions[0].arguments["surface"], "custom_apis");
                assert_eq!(actions[0].arguments["id"], "provider-api");
                assert_eq!(actions[0].arguments["run_check"], true);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_generic_integration_test_maps_to_inspect_with_run_check() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "test",
            "kind": "integration",
            "query": "provider api"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "inspect_integration");
                assert_eq!(actions[0].arguments["query"], "provider api");
                assert_eq!(actions[0].arguments["run_check"], true);
                assert!(actions[0].arguments.get("surface").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_generic_integration_install_without_structured_spec_routes_to_capability_resolution(
    ) {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "install",
            "kind": "integration",
            "query": "provider integration"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "capability_resolve");
                assert!(actions[0].arguments.get("selected_action").is_none());
                assert_eq!(
                    actions[0].arguments["requested_capability"],
                    "provider integration"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_mcp_server_create_maps_to_manage_action() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "mcp_server",
            "content": {
                "name": "Voice API MCP",
                "url": "https://mcp.example.com/mcp",
                "auth_type": "bearer"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "mcp_server_manage");
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(actions[0].arguments["url"], "https://mcp.example.com/mcp");
                assert!(actions[0].arguments.get("kind").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_mcp_server_install_and_connect_map_to_manage_action() {
        for op in ["install", "connect"] {
            let plan = plan_resource_rw(&serde_json::json!({
                "op": op,
                "kind": "mcp_server",
                "content": {
                    "name": "Voice API MCP",
                    "url": "https://mcp.example.com/mcp",
                    "auth_type": "bearer"
                }
            }));
            match plan {
                PrimitivePlan::Actions(actions) => {
                    assert_eq!(actions[0].action_name, "mcp_server_manage");
                    assert_eq!(actions[0].arguments["operation"], "create");
                    assert_eq!(actions[0].arguments["url"], "https://mcp.example.com/mcp");
                    assert!(actions[0].arguments.get("kind").is_none());
                }
                other => panic!("unexpected plan for {op}: {other:?}"),
            }
        }
    }

    #[test]
    fn resource_extension_pack_connect_without_pack_id_routes_to_capability_resolution() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "connect",
            "kind": "extension_pack",
            "query": "project management provider"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "capability_resolve");
                assert_eq!(
                    actions[0].arguments["selected_action"],
                    "extension_pack_connect"
                );
                assert_eq!(
                    actions[0].arguments["requested_capability"],
                    "project management provider"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_generic_integration_with_mcp_like_fields_resolves_before_mutation() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "install",
            "kind": "integration",
            "content": {
                "name": "Voice API MCP",
                "url": "https://mcp.example.com/mcp",
                "auth_type": "bearer",
                "resources_enabled": true
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "capability_resolve");
                assert!(actions[0].arguments.get("selected_action").is_none());
                let failure_output = actions[0].arguments["failure_output"]
                    .as_str()
                    .expect("capability_resolve should include structured fallback output");
                assert!(failure_output.contains("\"kind\":\"integration\""));
                assert!(
                    failure_output.contains("\"policy\":\"discover_or_inspect_first_then_mutate\"")
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_mcp_server_create_without_config_routes_to_capability_resolution() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "mcp_server",
            "query": "provider MCP"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "capability_resolve");
                assert_eq!(actions[0].arguments["selected_action"], "mcp_server_manage");
                assert_eq!(actions[0].arguments["requested_capability"], "provider MCP");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_messaging_channel_create_without_send_routes_to_capability_resolution() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "custom_messaging_channel",
            "query": "provider alerts"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].action_name, "capability_resolve");
                assert_eq!(
                    actions[0].arguments["selected_action"],
                    "custom_messaging_channel_upsert"
                );
                assert_eq!(
                    actions[0].arguments["requested_capability"],
                    "provider alerts"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_messaging_channel_create_maps_to_upsert() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "custom_messaging_channel",
            "content": {
                "name": "Provider alerts",
                "send": {
                    "method": "post",
                    "url_template": "https://hooks.example.com/send",
                    "body_template": "{\"text\":\"{{text}}\"}"
                }
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "custom_messaging_channel_upsert");
                assert_eq!(actions[0].arguments["name"], "Provider alerts");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_custom_messaging_channel_lifecycle_maps_to_manage() {
        for (op, expected_operation) in [
            ("delete", "delete"),
            ("enable", "enable"),
            ("disable", "disable"),
            ("test", "test"),
        ] {
            let plan = plan_resource_rw(&serde_json::json!({
                "op": op,
                "kind": "custom_messaging_channel",
                "id": "provider-alerts"
            }));
            match plan {
                PrimitivePlan::Actions(actions) => {
                    assert_eq!(actions[0].action_name, "custom_messaging_channel_manage");
                    assert_eq!(actions[0].arguments["id"], "provider-alerts");
                    assert_eq!(actions[0].arguments["operation"], expected_operation);
                }
                other => panic!("unexpected plan for {op}: {other:?}"),
            }
        }
    }

    #[test]
    fn resource_custom_messaging_channel_create_with_notification_shape_routes_to_notification() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "custom_messaging_channel",
            "content": {
                "message": "Meeting with Mark",
                "channel": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "notify_user");
                assert_eq!(actions[0].arguments["message"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["delivery_channel"], "telegram");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_unsupported_surface_with_notification_shape_returns_recovery_hint() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "conversation",
            "content": {
                "message": "Meeting with Mark",
                "channel": "slack"
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("does not yet have a substrate adapter"));
                let extra =
                    extra.expect("notification-shaped unsupported resource should include hint");
                assert_eq!(extra["suggested_kind"], "notification");
                assert_eq!(extra["suggested_op"], "create");
                assert!(extra["hint"]
                    .as_str()
                    .unwrap()
                    .contains("kind=notification"));
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_extension_pack_install_maps_to_lifecycle_action() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "install",
            "kind": "extension_pack",
            "content": {
                "pack_id": "linear"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "extension_pack_install");
                assert_eq!(actions[0].arguments["pack_id"], "linear");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_extension_pack_delete_maps_to_delete_action() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "delete",
            "kind": "extension_pack",
            "id": "linear"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "extension_pack_delete");
                assert_eq!(actions[0].arguments["pack_id"], "linear");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_skill_import_requires_direct_skill_manage() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "install",
            "kind": "skill",
            "content": {
                "url": "https://example.com/skills/SKILL.md",
                "name": "source-checker"
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("skill_manage"));
                assert_eq!(extra.unwrap()["suggested_primitive"], "skill_manage");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn direct_skill_manage_maps_to_generic_skill_management() {
        let plan = plan_skill_manage(&serde_json::json!({
            "operation": "import",
            "url": "https://example.com/skills/SKILL.md",
            "name": "source-checker"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "manage_actions");
                assert_eq!(actions[0].arguments["resource"], "skill");
                assert_eq!(actions[0].arguments["operation"], "import");
                assert_eq!(
                    actions[0].arguments["url"],
                    "https://example.com/skills/SKILL.md"
                );
                assert_eq!(actions[0].arguments["name"], "source-checker");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn direct_skill_manage_preserves_authored_skill_content() {
        let plan = plan_skill_manage(&serde_json::json!({
            "operation": "create",
            "name": "source-checker",
            "markdown": "---\nname: source-checker\n---\n\n# Source Checker"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "manage_actions");
                assert_eq!(actions[0].arguments["resource"], "skill");
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(
                    actions[0].arguments["content"],
                    "---\nname: source-checker\n---\n\n# Source Checker"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_skill_enable_maps_to_generic_skill_management() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "enable",
            "kind": "skill",
            "id": "source-checker"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "manage_actions");
                assert_eq!(actions[0].arguments["resource"], "skill");
                assert_eq!(actions[0].arguments["operation"], "enable");
                assert_eq!(actions[0].arguments["name"], "source-checker");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_skill_marketplace_create_maps_to_generic_skill_management() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "skill_marketplace",
            "content": {
                "name": "Team Skills",
                "url": "https://example.com/marketplace.json"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "manage_actions");
                assert_eq!(actions[0].arguments["resource"], "skill_marketplace");
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(actions[0].arguments["name"], "Team Skills");
                assert_eq!(
                    actions[0].arguments["url"],
                    "https://example.com/marketplace.json"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_scheduled_task_preserves_script_state_store_fields() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "scheduled_task",
            "content": {
                "task": "Run recurring outreach automation with a local state database",
                "cron": "*/10 * * * *",
                "script_language": "python",
                "script": "import sqlite3\nsqlite3.connect('lead-state.db').close()",
                "workdir": "automations/lead-outreach"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["cron"], "*/10 * * * *");
                assert_eq!(actions[0].arguments["script_language"], "python");
                assert_eq!(actions[0].arguments["workdir"], "automations/lead-outreach");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_scheduled_task_preserves_scheduler_fields_from_tool_payload() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "scheduled_task",
            "task": "Send meeting reminder",
            "at": "2026-05-26T21:51:00+05:30",
            "action": "notify_user",
            "action_arguments": {
                "message": "Meeting with Mark"
            },
            "report_to": "telegram"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task"], "Send meeting reminder");
                assert_eq!(actions[0].arguments["at"], "2026-05-26T21:51:00+05:30");
                assert_eq!(actions[0].arguments["action"], "notify_user");
                assert_eq!(
                    actions[0].arguments["action_arguments"]["message"],
                    "Meeting with Mark"
                );
                assert_eq!(actions[0].arguments["report_to"], "telegram");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_notification_with_schedule_maps_to_notify_user_scheduled_task() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "notification",
            "content": {
                "message": "Meeting with Mark",
                "local_time": "12:10 PM",
                "timezone": "Asia/Kolkata",
                "delivery_channel": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["local_time"], "12:10 PM");
                assert_eq!(actions[0].arguments["timezone"], "Asia/Kolkata");
                assert_eq!(actions[0].arguments["report_to"], "telegram");
                assert_eq!(actions[0].arguments["action"], "notify_user");
                assert_eq!(
                    actions[0].arguments["action_arguments"]["message"],
                    "Meeting with Mark"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn scheduled_notification_ignores_invented_delivery_action_names() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "notification",
            "content": {
                "message": "Meeting with Mark",
                "local_time": "12:10 PM",
                "timezone": "Asia/Kolkata",
                "delivery_channel": "ext.ops-alerts",
                "action": "send_external_delivery_channel"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["action"], "notify_user");
                assert_eq!(actions[0].arguments["report_to"], "ext.ops-alerts");
                assert_eq!(
                    actions[0].arguments["action_arguments"]["message"],
                    "Meeting with Mark"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_notification_without_schedule_maps_to_direct_notify_user() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "notification",
            "content": {
                "title": "Meeting",
                "message": "Meeting with Mark",
                "delivery_channel": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "notify_user");
                assert_eq!(actions[0].arguments["title"], "Meeting");
                assert_eq!(actions[0].arguments["message"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["delivery_channel"], "telegram");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_notification_schedule_accepts_structured_channel_route_and_title_body() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "notification",
            "content": {
                "title": "Meeting with Mark",
                "local_time": "12:10 PM",
                "timezone": "Asia/Kolkata",
                "channel": "slack"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["report_to"], "slack");
                assert_eq!(actions[0].arguments["action"], "notify_user");
                assert_eq!(
                    actions[0].arguments["action_arguments"]["message"],
                    "Meeting with Mark"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_notification_direct_accepts_structured_channel_route_and_title_body() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "notification",
            "content": {
                "title": "Meeting with Mark",
                "channel": "whatsapp"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "notify_user");
                assert_eq!(actions[0].arguments["message"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["delivery_channel"], "whatsapp");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_notification_text_without_structured_schedule_stays_direct() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "notification",
            "content": {
                "message": "Meeting with Mark at 1:30 PM",
                "delivery_channel": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "notify_user");
                assert_eq!(
                    actions[0].arguments["message"],
                    "Meeting with Mark at 1:30 PM"
                );
                assert_eq!(actions[0].arguments["delivery_channel"], "telegram");
                assert!(actions[0].arguments.get("local_time").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }
    #[test]
    fn resource_scheduled_task_message_payload_becomes_notify_user_action() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "scheduled_task",
            "content": {
                "message": "Meeting with Mark",
                "local_time": "12:10 PM",
                "timezone": "Asia/Kolkata",
                "delivery_channel": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["action"], "notify_user");
                assert_eq!(
                    actions[0].arguments["action_arguments"]["message"],
                    "Meeting with Mark"
                );
                assert_eq!(actions[0].arguments["report_to"], "telegram");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_scheduled_task_notification_payload_accepts_structured_channel_route() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "scheduled_task",
            "content": {
                "message": "Meeting with Mark",
                "local_time": "12:10 PM",
                "timezone": "Asia/Kolkata",
                "channel": "pagerduty"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task"], "Meeting with Mark");
                assert_eq!(actions[0].arguments["action"], "notify_user");
                assert_eq!(actions[0].arguments["report_to"], "pagerduty");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_scheduled_task_update_maps_id_to_task_id() {
        let task_id = "11111111-1111-4111-8111-111111111111";
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "update",
            "kind": "scheduled_task",
            "id": task_id,
            "content": {
                "task": "Send the existing reminder",
                "at": "2026-05-22T13:06:00+05:30",
                "report_to": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task_id"], task_id);
                assert_eq!(actions[0].arguments["at"], "2026-05-22T13:06:00+05:30");
                assert!(actions[0].arguments.get("id").is_none());
                assert!(actions[0].arguments.get("op").is_none());
                assert!(actions[0].arguments.get("kind").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_scheduled_task_update_preserves_persisted_schedule_fields() {
        let task_id = "11111111-1111-4111-8111-111111111111";
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "update",
            "kind": "scheduled_task",
            "id": task_id,
            "content": {
                "scheduled_for": "2026-05-22T13:06:00+05:30"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task_id"], task_id);
                assert_eq!(
                    actions[0].arguments["scheduled_for"],
                    "2026-05-22T13:06:00+05:30"
                );
                assert!(actions[0].arguments.get("id").is_none());
                assert!(actions[0].arguments.get("op").is_none());
                assert!(actions[0].arguments.get("kind").is_none());
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_scheduled_task_update_preserves_structured_local_time() {
        let task_id = "11111111-1111-4111-8111-111111111111";
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "update",
            "kind": "scheduled_task",
            "id": task_id,
            "content": {
                "local_time": "00:22",
                "timezone": "Asia/Kolkata"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "schedule_task");
                assert_eq!(actions[0].arguments["task_id"], task_id);
                assert_eq!(actions[0].arguments["local_time"], "00:22");
                assert_eq!(actions[0].arguments["timezone"], "Asia/Kolkata");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_durable_work_lifecycle_maps_to_work_manage() {
        let cases = [
            ("scheduled_task", "delete", "task_id"),
            ("scheduled_task", "status", "task_id"),
            ("scheduled_task", "list", "task_id"),
            ("watcher", "status", "watcher_id"),
            ("watcher", "pause", "watcher_id"),
            ("background_session", "resume", "background_session_id"),
        ];

        for (kind, op, expected_id_field) in cases {
            let mut args = serde_json::json!({
                "op": op,
                "kind": kind,
                "id": "11111111-1111-4111-8111-111111111111"
            });
            if op == "list" {
                args.as_object_mut().unwrap().remove("id");
            }
            let plan = plan_resource_rw(&args);
            match plan {
                PrimitivePlan::Actions(actions) => {
                    assert_eq!(actions[0].action_name, "background_work_manage");
                    assert_eq!(actions[0].arguments["operation"], op);
                    if op != "list" {
                        assert_eq!(
                            actions[0].arguments[expected_id_field],
                            "11111111-1111-4111-8111-111111111111"
                        );
                    }
                    assert_eq!(actions[0].arguments["kind"], kind);
                    assert!(actions[0].arguments.get("op").is_none());
                    assert!(actions[0].arguments.get("id").is_none());
                }
                other => panic!("unexpected plan for {kind}/{op}: {other:?}"),
            }
        }
    }

    #[test]
    fn resource_background_session_create_is_not_advertised_as_lifecycle_op() {
        let ops = supported_resource_ops("background_session");

        assert!(!ops.contains(&"create"));
        assert!(!ops.contains(&"update"));
        assert!(ops.contains(&"status"));
        assert!(ops.contains(&"update_delivery"));
    }

    #[test]
    fn resource_background_session_create_does_not_route_to_lifecycle_manage() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "background_session",
            "query": "monitor external service state on a cadence and notify only on material changes",
            "content": {
                "cadence": "twice daily",
                "notify_channel": "in_app"
            }
        }));

        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("background_session"));
                let extra = extra.expect("create recovery hint should be present");
                assert_eq!(extra["suggested_kinds"][0], "watcher");
                assert_eq!(extra["suggested_kinds"][1], "scheduled_task");
            }
            PrimitivePlan::Actions(actions) => {
                panic!(
                    "background_session create must not map to {}; lifecycle management cannot create durable work",
                    actions[0].action_name
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_source_payloads_are_not_deploy_material() {
        for content in [
            serde_json::json!({"name": "Pricing report", "content": "<!doctype html><html><body>Report</body></html>"}),
            serde_json::json!({"name": "Generated app", "source": "<!doctype html><html><body>App</body></html>"}),
            serde_json::json!({"name": "Nested files", "files": {"index.html": {"content": "<!doctype html><title>Nested</title>"}}}),
            serde_json::json!({"name": "Empty app"}),
        ] {
            let plan = plan_resource_rw(&serde_json::json!({
                "op": "create",
                "kind": "app_service",
                "content": content
            }));
            match plan {
                PrimitivePlan::Unsupported { reason, extra } => {
                    assert!(reason.contains("app_deploy"));
                    assert_eq!(extra.unwrap()["suggested_primitive"], "app_deploy");
                }
                other => panic!("unexpected plan: {other:?}"),
            }
        }
    }

    #[test]
    fn resource_app_service_status_remains_lifecycle_resource() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "status",
            "kind": "app_service",
            "id": "app-123"
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "service_manage");
                assert_eq!(actions[0].arguments["operation"], "status");
                assert_eq!(actions[0].arguments["service_id"], "app-123");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn repeated_resource_mutation_signature_uses_stable_target() {
        let first = SpineToolCall {
            id: "a".to_string(),
            name: "resource_rw".to_string(),
            arguments: serde_json::json!({
                "op": "create",
                "kind": "app_service",
                "content": {
                    "name": "Pricing comparison",
                    "files": {"index.html": "<!doctype html><html><body>v1</body></html>"}
                }
            }),
            activity_label: None,
        };
        let second = SpineToolCall {
            id: "b".to_string(),
            name: "resource_rw".to_string(),
            arguments: serde_json::json!({
                "op": "create",
                "kind": "app_service",
                "content": {
                    "name": "Pricing comparison",
                    "files": {"index.html": "<!doctype html><html><body>v2</body></html>"}
                }
            }),
            activity_label: None,
        };

        let first = tool_call_progress_signature(&first).expect("signature");
        let second = tool_call_progress_signature(&second).expect("signature");

        assert_eq!(first.class, ToolProgressClass::Mutation);
        assert_eq!(first.key, second.key);
    }

    #[test]
    fn file_write_progress_signature_is_content_aware_for_rewrites() {
        let first = SpineToolCall {
            id: "a".to_string(),
            name: "file_write".to_string(),
            arguments: serde_json::json!({
                "path": "expense-tracker/server.py",
                "content": "print('old')"
            }),
            activity_label: None,
        };
        let second = SpineToolCall {
            id: "b".to_string(),
            name: "file_write".to_string(),
            arguments: serde_json::json!({
                "path": "expense-tracker/server.py",
                "content": "print('fixed')"
            }),
            activity_label: None,
        };
        let repeated = SpineToolCall {
            id: "c".to_string(),
            name: "file_write".to_string(),
            arguments: first.arguments.clone(),
            activity_label: None,
        };

        let first = tool_call_progress_signature(&first).expect("signature");
        let second = tool_call_progress_signature(&second).expect("signature");
        let repeated = tool_call_progress_signature(&repeated).expect("signature");

        assert_eq!(first.class, ToolProgressClass::Mutation);
        assert_ne!(first.key, second.key);
        assert_eq!(first.key, repeated.key);
    }

    #[test]
    fn file_write_progress_signature_includes_non_text_body_sources() {
        let from_source_path = SpineToolCall {
            id: "a".to_string(),
            name: "file_write".to_string(),
            arguments: serde_json::json!({
                "path": "report.html",
                "source_path": "tmp/report.html"
            }),
            activity_label: None,
        };
        let from_resource = SpineToolCall {
            id: "b".to_string(),
            name: "file_write".to_string(),
            arguments: serde_json::json!({
                "path": "report.html",
                "source_resource": {"id": "resource-1", "kind": "managed_file"}
            }),
            activity_label: None,
        };

        let source_path = tool_call_progress_signature(&from_source_path).expect("signature");
        let source_resource = tool_call_progress_signature(&from_resource).expect("signature");

        assert_ne!(source_path.key, source_resource.key);
    }

    #[test]
    fn file_patch_progress_signature_is_patch_aware() {
        let first = SpineToolCall {
            id: "a".to_string(),
            name: "file_patch".to_string(),
            arguments: serde_json::json!({
                "path": "expense-tracker/server.py",
                "patch": "@@\n-print('old')\n+print('fixed')\n"
            }),
            activity_label: None,
        };
        let second = SpineToolCall {
            id: "b".to_string(),
            name: "file_patch".to_string(),
            arguments: serde_json::json!({
                "path": "expense-tracker/server.py",
                "patch": "@@\n-print('old')\n+print('newer')\n"
            }),
            activity_label: None,
        };

        let first = tool_call_progress_signature(&first).expect("signature");
        let second = tool_call_progress_signature(&second).expect("signature");

        assert_ne!(first.key, second.key);
    }

    #[test]
    fn dependency_batches_serialize_file_write_before_source_dir_deploy() {
        let calls = vec![
            (
                SpineToolCall {
                    id: "write".to_string(),
                    name: "file_write".to_string(),
                    arguments: serde_json::json!({
                        "path": "apps/demo/index.html",
                        "content": "<!doctype html><html><body>Demo</body></html>"
                    }),
                    activity_label: None,
                },
                None,
                false,
            ),
            (
                SpineToolCall {
                    id: "deploy".to_string(),
                    name: "app_deploy".to_string(),
                    arguments: serde_json::json!({
                        "source_dir": "apps/demo",
                        "source_paths": ["index.html"]
                    }),
                    activity_label: None,
                },
                None,
                false,
            ),
        ];

        let batches = dependency_batches_for_tool_calls(&calls, Some("conversation-1"));
        assert_eq!(batches, vec![vec![0], vec![1]]);
    }

    #[test]
    fn dependency_batches_keep_independent_reads_parallel() {
        let calls = vec![
            (
                SpineToolCall {
                    id: "search".to_string(),
                    name: "search".to_string(),
                    arguments: serde_json::json!({"query": "papers"}),
                    activity_label: None,
                },
                None,
                false,
            ),
            (
                SpineToolCall {
                    id: "read".to_string(),
                    name: "file_read".to_string(),
                    arguments: serde_json::json!({"path": "notes.md"}),
                    activity_label: None,
                },
                None,
                false,
            ),
        ];

        let batches = dependency_batches_for_tool_calls(&calls, Some("conversation-1"));
        assert_eq!(batches, vec![vec![0, 1]]);
    }

    #[test]
    fn final_response_uses_exact_local_app_link_from_tool_result() {
        let messages = vec![SpineMessage::Tool {
            tool_call_id: "call-app".to_string(),
            content: serde_json::json!({
                "ok": true,
                "app": {
                    "id": "abc12345",
                    "title": "Demo App",
                    "url": "/apps/abc12345/"
                }
            })
            .to_string(),
        }];
        let normalized = normalize_final_response_artifact_links(
            "Live: https://source.example/apps/abc12345/",
            &messages,
        );
        assert!(normalized.contains("[Demo App](/apps/abc12345/)"));
        assert!(!normalized.contains("source.example/apps/abc12345"));
    }

    #[test]
    fn memory_write_requires_structured_content() {
        let plan = plan_memory_rw_for_caller(
            &serde_json::json!({
                "op": "write",
                "explicit_user_request": true,
                "content": {"key": "preferred_model", "value": "fast"}
            }),
            CallerKind::Task,
        );
        assert!(matches!(
            plan,
            PrimitivePlan::Memory(MemoryPrimitiveOp::Write { .. })
        ));
    }

    #[test]
    fn memory_write_requires_active_memory_management_intent() {
        let plan = plan_memory_rw_for_caller(
            &serde_json::json!({
                "op": "write",
                "content": {"key": "personal_fact", "value": "durable user-provided fact"}
            }),
            CallerKind::Task,
        );
        assert!(matches!(plan, PrimitivePlan::Unsupported { .. }));
    }

    #[test]
    fn chat_memory_mutation_plan_is_deferred_for_arkmemory() {
        let plan = plan_memory_rw_for_caller(
            &serde_json::json!({
                "op": "update",
                "explicit_user_request": true,
                "content": {
                    "key": "profile.location",
                    "value": "Kolkata, India"
                },
                "intent_summary": "The user corrected a saved profile fact."
            }),
            CallerKind::Chat,
        );

        assert!(matches!(
            plan,
            PrimitivePlan::Memory(MemoryPrimitiveOp::DeferredMutation { .. })
        ));
    }

    #[test]
    fn chat_memory_schema_exposes_read_only_memory_ops() {
        let schema = ToolRegistry::new()
            .schemas_for_caller(CallerKind::Chat)
            .into_iter()
            .find(|schema| schema.name == "memory_rw")
            .expect("memory_rw schema should be present");
        let ops = schema.input_schema["properties"]["op"]["enum"]
            .as_array()
            .expect("op enum should be an array")
            .iter()
            .map(|value| value.as_str().unwrap_or_default())
            .collect::<Vec<_>>();

        assert_eq!(ops, vec!["search", "read"]);
    }
}
