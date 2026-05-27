use super::spine_prompt_bundle::{
    self, ALLOWED_EVOLVABLE_SPINE_FRAGMENT_IDS, SPINE_PROMPT_BUNDLE_VERSION,
};
use super::spine_request::*;
use super::*;
use crate::actions::ActionAuthorization;
use async_trait::async_trait;
use chrono::{TimeZone, Timelike};
use futures::future::join_all;
use std::sync::LazyLock;

const PRIMITIVE_NAMES: [&str; 8] = [
    "search",
    "fetch",
    "browse",
    "code_exec",
    "pdf_generate",
    "resource_rw",
    "memory_rw",
    "delegate",
];

const RESOURCE_RW_VALID_KINDS: [&str; 17] = [
    "file",
    "app_service",
    "watcher",
    "scheduled_task",
    "notification",
    "background_session",
    "goal",
    "dashboard",
    "conversation",
    "activity",
    "integration",
    "custom_api",
    "custom_messaging_channel",
    "extension_pack",
    "mcp_server",
    "skill",
    "skill_marketplace",
];
const LLM_NATIVE_IMAGE_ATTACHMENT_LIMIT: usize = 4;
const LLM_NATIVE_IMAGE_ATTACHMENT_MAX_BYTES: u64 = 8 * 1024 * 1024;
static NOTIFICATION_CLOCK_TIME_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(?:^|[^[:alnum:]_])(?:(?P<hour12>0?[1-9]|1[0-2])(?:[:.](?P<minute12>[0-5]\d))?\s*(?P<meridiem>a\.?m\.?|p\.?m\.?)|(?P<hour24>[01]?\d|2[0-3]):(?P<minute24>[0-5]\d)(?::(?P<second24>[0-5]\d))?)(?:[^[:alnum:]_]|$)",
    )
    .expect("valid notification clock time regex")
});

#[derive(Debug, Clone)]
pub struct SpineChatResponse {
    pub text: String,
    pub partial_text: Option<String>,
    pub tool_calls: Vec<SpineToolCall>,
    pub completion_tokens: usize,
    pub cache_read_tokens: usize,
    pub cache_creation_tokens: usize,
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
                        Ok(value) => outputs.push(value),
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
        let mut result =
            spine_tool_result_value_for_model(call.name.as_str(), &invocation.action_name, content);
        remember_pending_credential_prompt_from_tool_result(cx, &mut result).await;
        Ok(result)
    }

    async fn repair_invocation_for_request_context(
        &self,
        invocation: &PrimitiveActionInvocation,
        call: &SpineToolCall,
        cx: &SpineContext,
    ) -> PrimitiveActionInvocation {
        if call.name == "resource_rw" && invocation.action_name.eq_ignore_ascii_case("notify_user")
        {
            if let Some(request_text) = latest_user_message_text(&cx.request.messages) {
                let default_timezone = {
                    let profile = cx.agent.user_profile.read().await;
                    profile
                        .timezone
                        .as_deref()
                        .and_then(|timezone| timezone.parse::<chrono_tz::Tz>().ok())
                };
                if let Some(repaired) = scheduled_notification_invocation_from_direct_notification(
                    invocation,
                    request_text,
                    chrono::Utc::now(),
                    default_timezone,
                ) {
                    return repaired;
                }
            }
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
            MemoryPrimitiveOp::Write {
                key,
                value,
                kind,
                scope,
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

    async fn plan_call(&self, call: &SpineToolCall, _cx: &SpineContext) -> PrimitivePlan {
        match call.name.as_str() {
            "search" => plan_search(&call.arguments),
            "fetch" => plan_fetch(&call.arguments),
            "browse" => plan_browse(&call.arguments),
            "code_exec" => plan_code_exec(&call.arguments),
            "pdf_generate" => plan_pdf_generate(&call.arguments),
            "resource_rw" => plan_resource_rw(&call.arguments),
            "memory_rw" => plan_memory_rw(&call.arguments),
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
struct NotificationScheduleHint {
    local_date: String,
    local_time: String,
    timezone_name: String,
    timezone_abbrev: String,
    date_label: String,
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
    Write {
        key: String,
        value: String,
        kind: Option<String>,
        scope: Option<String>,
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

impl ToolResult {
    fn from_value(ok: bool, value: serde_json::Value) -> Self {
        Self { ok, value }
    }

    fn to_json(&self) -> String {
        if self.value.get("ok").is_some() || self.value.get("status").is_some() {
            self.value.to_string()
        } else {
            serde_json::json!({
                "ok": self.ok,
                "result": self.value,
            })
            .to_string()
        }
    }

    fn summary(&self) -> String {
        let raw = self.to_json();
        safe_truncate(&raw, 240)
    }
}

const SPINE_TOOL_STREAM_MAX_STRING_CHARS: usize = 900;
const SPINE_TOOL_STREAM_MAX_ARRAY_ITEMS: usize = 12;
const SPINE_TOOL_STREAM_MAX_OBJECT_KEYS: usize = 24;
const SPINE_TOOL_STREAM_MAX_DEPTH: usize = 5;
const SPINE_TOOL_STREAM_PREVIEW_ITEMS: usize = 5;

fn spine_tool_start_stream_payload(call: &SpineToolCall) -> serde_json::Value {
    let arguments = sanitize_spine_tool_stream_value(&call.arguments, 0);
    let intent_summary = spine_tool_start_intent_summary(&call.name, &arguments);
    serde_json::json!({
        "kind": "model_tool_call",
        "tool_call_id": call.id,
        "tool_name": call.name,
        "arguments": arguments,
        "intent_summary": intent_summary,
    })
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

fn sanitize_spine_tool_stream_value(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= SPINE_TOOL_STREAM_MAX_DEPTH {
        return match value {
            serde_json::Value::Null => serde_json::Value::Null,
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => value.clone(),
            serde_json::Value::String(text) => {
                serde_json::Value::String(safe_truncate(text.trim(), 160))
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
            text.trim(),
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
        .split(|ch: char| ch == '_' || ch == '-')
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
        .split(|ch: char| ch == '_' || ch == '-')
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
    let max_turns = request.max_turns.max(1);
    let mut completed_tool_signatures: HashMap<String, ToolProgressClass> = HashMap::new();

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
                tools.schemas(),
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
        })
        .await;

        if response.tool_calls.is_empty() {
            let final_text = normalize_final_response_artifact_links(&response.text, &messages);
            if let Some(blocker) = no_progress_final_response(&messages, &final_text) {
                messages.push(SpineMessage::Assistant {
                    content: Some(blocker.clone()),
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
                    final_text: blocker,
                    turns_used: turn + 1,
                };
            }
            if incomplete_no_tool_final_response(&messages, &final_text) {
                if turn + 1 < max_turns {
                    messages.push(SpineMessage::User {
                        content: incomplete_no_tool_final_response_repair_prompt(),
                    });
                    continue;
                }
                let blocker = incomplete_no_tool_final_response_blocker(&final_text);
                messages.push(SpineMessage::Assistant {
                    content: Some(blocker.clone()),
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
                    final_text: blocker,
                    turns_used: turn + 1,
                };
            }
            if final_response_repeats_tool_call_content(&messages, &final_text)
                && turn + 1 < max_turns
            {
                messages.push(SpineMessage::User {
                    content: stale_final_response_repair_prompt(),
                });
                continue;
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

        messages.push(SpineMessage::Assistant {
            content: response.partial_text.clone(),
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

        let futures = prepared_calls
            .iter()
            .map(|(tool_call, _signature, blocked)| {
                let tool_call = tool_call.clone();
                let blocked = *blocked;
                let tools = tools.clone();
                let cx = cx.clone();
                async move {
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
            })
            .collect::<Vec<_>>();
        let results = join_all(futures).await;

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
                content: result.to_json(),
            });
        }
        if let Some(final_text) = needs_input_message_from_tool_results(&results) {
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
        _ => return None,
    };
    let signature_args =
        tool_call_progress_identity(call).unwrap_or_else(|| call.arguments.clone());
    Some(ToolProgressSignature {
        key: format!("{}:{}", call.name, canonical_json_string(&signature_args)),
        class,
    })
}

fn tool_call_progress_identity(call: &SpineToolCall) -> Option<serde_json::Value> {
    if call.name == "pdf_generate" {
        return Some(serde_json::json!({
            "title": json_text(&call.arguments, "title"),
            "filename": json_text(&call.arguments, "filename"),
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
}

impl AgentSpineLlmServer {
    pub fn new(
        agent: Agent,
        channel: impl Into<String>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        trace: Arc<SpineTraceRecorder>,
        caller_kind: CallerKind,
    ) -> Self {
        Self {
            agent,
            channel: channel.into(),
            stream_tx,
            trace,
            caller_kind,
        }
    }

    async fn load_llm_image_attachments(
        &self,
        hints: &[ChatAttachmentHint],
    ) -> Vec<crate::core::llm::LlmImageAttachment> {
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
            attachments.push(crate::core::llm::LlmImageAttachment {
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
        image_attachments: &[crate::core::llm::LlmImageAttachment],
    ) -> anyhow::Result<crate::core::llm::LlmResponse> {
        if streaming {
            if let Some(tx) = self.stream_tx.clone() {
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
        let mut candidates = self.agent.llm_candidates_for_role(&ModelRole::Primary);
        if candidates.is_empty() {
            candidates.push(self.agent.primary_llm_candidate());
        }
        let candidates = self
            .agent
            .reorder_candidates_with_failover(candidates, None)
            .await;
        let active_prompt_bundle = self
            .agent
            .active_prompt_bundle_for_message(&prepared.user_message)
            .await;
        let active_prompt_fragment_bundle = self
            .agent
            .active_prompt_fragment_bundle_for_message(&prepared.user_message)
            .await;
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
                    self.agent
                        .record_llm_usage(&self.channel, "spine_model_turn", &resp)
                        .await;
                    let mut attempted = Vec::new();
                    let mut attempt_records = Vec::new();
                    self.agent
                        .record_model_attempt(
                            &mut attempted,
                            &mut attempt_records,
                            candidate,
                            true,
                            None,
                            idx > 0,
                            started.elapsed().as_millis() as u64,
                            None,
                        )
                        .await;
                    let usage = resp.usage.clone();
                    return Ok(SpineChatResponse {
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
                    last_error = Some(error.to_string());
                    let mut attempted = Vec::new();
                    let mut attempt_records = Vec::new();
                    self.agent
                        .record_model_attempt(
                            &mut attempted,
                            &mut attempt_records,
                            candidate,
                            false,
                            last_error.as_deref(),
                            idx > 0,
                            started.elapsed().as_millis() as u64,
                            None,
                        )
                        .await;
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
}

struct PreparedSpineMessages {
    system_prompt: String,
    history: Vec<ConversationMessage>,
    user_message: String,
}

fn prepare_spine_messages_for_llm(messages: &[SpineMessage]) -> PreparedSpineMessages {
    let mut system_parts = Vec::new();
    let mut conversational = Vec::new();
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
            }
            SpineMessage::Tool {
                tool_call_id,
                content,
            } => conversational.push(ConversationMessage {
                role: "user".to_string(),
                content: format!("Tool result for `{}`:\n{}", tool_call_id, content),
                _timestamp: chrono::Utc::now(),
            }),
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

fn structured_chat_request_context_system_message(
    request_hints: &RequestExecutionHints,
) -> Option<String> {
    let mut context = serde_json::Map::new();
    if request_hints.attachments_present || !request_hints.attachments.is_empty() {
        context.insert(
            "attachments_present".to_string(),
            serde_json::json!(
                request_hints.attachments_present || !request_hints.attachments.is_empty()
            ),
        );
        context.insert(
            "attachments".to_string(),
            serde_json::to_value(&request_hints.attachments)
                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new())),
        );
    }
    if let Some(arkorbit_context) = request_hints.arkorbit_context.as_ref() {
        context.insert(
            "arkorbit_context".to_string(),
            bounded_json_for_spine_context(arkorbit_context, 8_000),
        );
    }
    if let Some(browser_profile_context) = request_hints.browser_profile_context.as_ref() {
        context.insert(
            "browser_profile_context".to_string(),
            bounded_json_for_spine_context(browser_profile_context, 8_000),
        );
    }
    if !request_hints.recent_actionable_artifacts.is_empty() {
        context.insert(
            "recent_actionable_artifacts".to_string(),
            bounded_json_for_spine_context(
                &serde_json::Value::Array(request_hints.recent_actionable_artifacts.clone()),
                8_000,
            ),
        );
    }
    if context.is_empty() {
        return None;
    }

    Some(format!(
        "Structured chat request context:\n{}",
        serde_json::to_string_pretty(&serde_json::Value::Object(context))
            .unwrap_or_else(|_| "{}".to_string())
    ))
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
    browser_sessions: &crate::core::browser_session::BrowserSessionManager,
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
        if let Some(request_context) = structured_chat_request_context_system_message(request_hints)
        {
            spine_messages.push(SpineMessage::System {
                content: request_context,
            });
        }
        spine_messages.push(SpineMessage::User {
            content: message.to_string(),
        });

        let mut request = SpineRequest::new(CallerKind::Chat, spine_messages, channel);
        request.conversation_id = conversation_id.map(str::to_string);
        request.project_id = project_id.map(str::to_string);
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
                turns_used: _,
                ..
            } => Ok(ProcessedMessage {
                response: final_text,
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
            SpineResult::Blocked { final_text, .. } => Ok(spine_blocked_processed_message(
                conversation_id,
                &final_text,
                crate::core::ExecutionRunStatus::Blocked.as_str(),
                trace_steps,
                turn_records,
                cached_prompt_tokens,
                cache_creation_prompt_tokens,
            )),
            SpineResult::NeedsInput { final_text, .. } => Ok(spine_blocked_processed_message(
                conversation_id,
                &final_text,
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
                    response: approval_required_response(&approval),
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
                &format!(
                    "The spine hit a platform failure before it could complete: {}.",
                    error.message
                ),
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

        let mut request = SpineRequest::new(CallerKind::Task, spine_messages, channel);
        request.conversation_id = conversation_id.map(str::to_string);
        request.project_id = project_id.map(str::to_string);
        request.authorization = automation_runtime_authorization_context(
            &task.arguments,
            ActionExecutionSurface::Automation,
        );

        let trace = Arc::new(SpineTraceRecorder::default());
        let request = Arc::new(request);
        let cx = SpineContext::new(self.clone(), request.clone(), trace.clone(), None);
        let server = AgentSpineLlmServer::new(
            self.clone(),
            channel,
            None,
            trace.clone(),
            request.caller_kind,
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
                "instruction": "Run exactly one watcher poll through the model-routed spine. Return the poll result as final text; do not evaluate the watcher condition in the final answer."
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

        let trace = Arc::new(SpineTraceRecorder::default());
        let request = Arc::new(request);
        let cx = SpineContext::new(self.clone(), request.clone(), trace.clone(), None);
        let server = AgentSpineLlmServer::new(
            self.clone(),
            channel,
            None,
            trace.clone(),
            request.caller_kind,
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
    ) -> Vec<crate::core::task_router::SubAgentSpec> {
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
            crate::core::task_router::SubAgentSpec {
                agent_type: "researcher".to_string(),
                task: format!(
                    "Gather the evidence and constraints needed to complete this delegated objective:\n{}",
                    context
                ),
                preferred_model_role: Some("research".to_string()),
                depends_on: Vec::new(),
                plan_step_id: None,
            },
            crate::core::task_router::SubAgentSpec {
                agent_type: "coder".to_string(),
                task: format!(
                    "Implement or execute the concrete work needed for this delegated objective:\n{}",
                    context
                ),
                preferred_model_role: Some("code".to_string()),
                depends_on: Vec::new(),
                plan_step_id: None,
            },
            crate::core::task_router::SubAgentSpec {
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
            content: (!message.content.trim().is_empty()).then_some(message.content),
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

fn spine_blocked_processed_message(
    conversation_id: Option<&str>,
    response: &str,
    status: &str,
    trace_steps: Vec<ExecutionStep>,
    turn_records: Vec<AgentTurnRecord>,
    cached_prompt_tokens: i64,
    cache_creation_prompt_tokens: i64,
) -> ProcessedMessage {
    ProcessedMessage {
        response: response.to_string(),
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
    prompt_fragment_bundle: &crate::core::prompt_fragments::PromptFragmentBundleProfile,
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
    let prompt_fragment_version = crate::core::prompt_fragments::compose_prompt_fragment_version(
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
                SpineTraceEvent::ToolStarted { name, .. } => (
                    "[tool]",
                    name.as_str(),
                    "info",
                    serde_json::to_string(event).ok(),
                ),
                SpineTraceEvent::ToolCompleted { ok, name, .. } => (
                    "[tool]",
                    name.as_str(),
                    if *ok { "success" } else { "warning" },
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
            ExecutionStep {
                icon: icon.to_string(),
                title: title.to_string(),
                detail: data.clone().unwrap_or_default(),
                step_type: step_type.to_string(),
                data,
                timestamp: chrono::Utc::now(),
                duration_ms: Some(0),
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
                outcome: if content.contains("\"ok\":true") || content.contains("\"status\":\"ok\"")
                {
                    AgentTurnOutcomeKind::Succeeded
                } else {
                    AgentTurnOutcomeKind::Abandoned
                },
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

fn build_spine_system_prompt(
    extra_system: &str,
    prompt_bundle: Option<&crate::core::self_evolve::PromptBundleProfile>,
    prompt_fragment_bundle: Option<&crate::core::prompt_fragments::PromptFragmentBundleProfile>,
) -> String {
    spine_prompt_bundle::build_spine_prompt_bundle(
        extra_system,
        prompt_bundle,
        prompt_fragment_bundle,
        &PRIMITIVE_NAMES,
    )
    .render()
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
                    "integration": { "type": "string", "enum": ["gmail", "calendar", "google_drive", "connector", "custom_api"], "description": "Canonical integration surface for connected reads. Use custom_api with id plus operation/body to call a saved read-only custom API action." },
                    "id": { "type": "string", "description": "Saved integration/custom API id when integration=custom_api." },
                    "operation": { "type": "string", "description": "Saved custom API operation id, action name, or operation label when integration=custom_api." },
                    "op": { "type": "string", "description": "Read operation such as read, list, query, today, free_busy." },
                    "query": { "type": "string" },
                    "content": { "type": "object" },
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
                    "expectation": { "type": "string", "description": "User-facing browser completion expectation, checkpoint, stop condition, requested question, or follow-up choice that must be preserved for the browser loop." },
                    "metadata": { "type": "object" }
                },
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "code_exec",
            "Run sandboxed computation, shell commands, tests, builds, parsers, or local analysis.",
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
            "Create a managed PDF document artifact from complete supplied content. Use this for PDF deliverables so AgentArk returns a Documents-visible managed file directly instead of requiring code execution or filesystem handoff.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Human-readable document title." },
                    "filename": { "type": "string", "description": "Suggested PDF filename. AgentArk normalizes it safely." },
                    "style": { "type": "string", "enum": ["plain", "report", "letter", "invoice"], "default": "report" },
                    "content": { "type": "string", "description": "Complete final document body to render into the PDF." },
                    "metadata": { "type": "object", "description": "Optional provenance or artifact identity metadata." }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        ),
        primitive_schema(
            "resource_rw",
            "Create, read, update, delete, list, pause, resume, connect, install, refresh, test, or inspect backed durable AgentArk resources, user-visible artifacts, notifications, local activity evidence, and external integration surfaces. Use notification for user-facing notification delivery; include schedule fields when it should happen later. Use activity for recent work, recent conversations, Reflect/Sentinel signals, work patterns, attention, avoidance, recurring themes, and retrospective personal activity questions. Use managed files for saved documents, reports, runbooks, reusable workflow instructions, and source assets; they are surfaced through Documents rather than container paths. Use app_service or dashboard for browser-runnable pages, dashboards, apps, and tools. Use scheduled_task or watcher only when the durable outcome must run independently later, monitor a condition, notify outside the artifact, or follow a concrete cadence. Use integration/custom_api/custom_messaging_channel/extension_pack/mcp_server when setting up or inspecting external capabilities for AgentArk itself. If a requested kind/op is unsupported, the tool result is terminal evidence; do not loop.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["create", "read", "update", "delete", "list", "status", "pause", "resume", "stop", "cancel", "update_delivery", "install", "connect", "enable", "disable", "refresh", "test"] },
                    "kind": {
                        "type": "string",
                        "enum": ["file", "app_service", "watcher", "scheduled_task", "notification", "background_session", "goal", "dashboard", "conversation", "activity", "integration", "custom_api", "custom_messaging_channel", "extension_pack", "mcp_server", "skill", "skill_marketplace"],
                        "description": "Resource substrate selected by meaning: notification for user-facing notification delivery, immediate or scheduled; activity for recent work, conversations, Reflect/Sentinel evidence, personal activity patterns, attention, avoidance, recurring themes, and retrospective local-state insight; app_service/dashboard for browser-viewable pages, dashboards, apps, games, tools, and HTML UI artifacts; file for raw documents, runbooks, source assets, and reusable instructions; scheduled_task/watcher only for independent future execution or monitoring; integration/custom_api/custom_messaging_channel/extension_pack/mcp_server for external capabilities installed into AgentArk; skill for reusable AgentArk procedures/capabilities; skill_marketplace for sources that list installable skills."
                    },
                    "id": {
                        "type": "string",
                        "description": "Identifier for an existing resource. For new file creation, do not use id as the destination path; use content.path."
                    },
                    "query": { "type": "string", "description": "Semantic lookup, listing, or matching text for the target resource." },
                    "content": {
                        "type": "object",
                        "description": "Payload for the resource. For kind=file create/update, include path as a workspace/data-relative string and one body source in the same call: content (complete file body string), content_base64, source_path, or source_resource. Do not create files with only title/description/metadata. Single-file patch updates need path plus patch; batch patch updates need patches entries with path and patch. For kind=app_service/dashboard, provide browser-runnable files with an HTML entrypoint such as index.html, a complete HTML document in content.content, or staged source_dir/source_paths so AgentArk returns a browser-accessible /apps/ URL. For kind=notification create, include message/title plus optional delivery route as delivery_channel, report_to, or channel; include cron, at, scheduled_for, or local_time/timezone when delivery should happen later. For kind=scheduled_task create/update, include task or task_id plus cron, at, scheduled_for, or local_time/timezone, optional report_to/channel, and optional action/action_arguments; for notification reminders, action_arguments.message is the reminder body. scheduled_for is the persisted one-time timestamp returned by task status/list and is equivalent to at. Use local_time/timezone when the user gave a wall-clock time without a full date. For custom_api create/update/install/connect, provide name, description, and base_url/path or openapi_url/openapi_text plus auth fields if known. For custom_messaging_channel configure a reusable non-bundled outbound channel by providing the channel name and declarative HTTP send specification; do not use it to send a one-off notification. For extension_pack provide pack_id/source_url/source_path/manifest_text/manifest for installs or pack_id plus connection fields for connect. For mcp_server provide name plus HTTP url or stdio command/args and auth type/credential metadata only when the requested substrate is explicitly MCP. For skill provide name plus complete SKILL.md markdown for create/update, url/source_url/install_url for import/install, or arguments for test. For skill_marketplace provide name/url/enabled for create/update and id/name for read/delete/refresh/enable/disable. Durable facts, preferences, and reusable knowledge that are not procedures should use memory_rw rather than skill."
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional provenance, title, source URLs, refresh notes, workflow steps, or non-sensitive operational metadata. For source-grounded apps/documents, include artifact_identity with source URLs and a compact source-data fingerprint or fact set so duplicate detection can reuse existing artifacts without relying on user wording. Do not use metadata to request scheduling unless the user intent requires later autonomous execution."
                    },
                    "duplicate_policy": {
                        "type": "string",
                        "enum": ["reuse_existing", "create_new"],
                        "default": "reuse_existing",
                        "description": "For app_service/dashboard and document-visible file writes, reuse/skip identical existing artifacts by default. Use create_new only when the user explicitly wants another duplicate copy."
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
                    "content": { "type": "object" },
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
    if depth == "deep" {
        PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "research".to_string(),
            arguments: serde_json::json!({
                "query": query,
                "depth": "standard",
            }),
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
        if method.eq_ignore_ascii_case("GET") {
            return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "page_fetch".to_string(),
                arguments: serde_json::json!({ "url": url }),
            }]);
        }
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "http_request".to_string(),
            arguments: merge_objects(
                serde_json::json!({ "url": url, "method": method }),
                merge_content_metadata(arguments),
            ),
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

fn plan_resource_rw(arguments: &serde_json::Value) -> PrimitivePlan {
    let op = json_text(arguments, "op")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let kind = json_text(arguments, "kind")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
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
            let mut payload = merge_content_metadata(arguments);
            let has_patch = payload.get("patch").is_some() || payload.get("patches").is_some();
            if let Some(plan) = validate_file_mutation_payload(&payload, op.as_str(), has_patch) {
                return plan;
            }
            if !has_patch && op == "create" && file_payload_should_deploy_as_app(&payload) {
                return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "service_manage".to_string(),
                    arguments: app_service_payload_from_html_file_payload(&payload),
                }]);
            }
            if !has_patch
                && payload.get("document_visible").is_none()
                && payload.get("index_document").is_none()
            {
                if let Some(object) = payload.as_object_mut() {
                    object.insert(
                        "document_visible".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: if has_patch {
                    "file_patch"
                } else {
                    "file_write"
                }
                .to_string(),
                arguments: payload,
            }])
        }
        ("app_service", _) | ("dashboard", _) => {
            let payload = service_manage_payload_from_resource(arguments);
            if matches!(op.as_str(), "create" | "update")
                && !service_manage_payload_has_deploy_material(&payload)
            {
                return unsupported_with_extra(
                    "resource_rw app_service/dashboard create/update requires deployable app content.",
                    serde_json::json!({
                        "kind": kind,
                        "op": op,
                        "field": "content.files",
                        "hint": "Provide content.files with app-relative file names and complete file bodies, or content.content with a complete HTML document, or source_dir/source_paths for staged files."
                    }),
                );
            }
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
        ("custom_api", "create")
        | ("custom_api", "update")
        | ("custom_api", "install")
        | ("custom_api", "connect") => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                object.remove("op");
                object.remove("kind");
                if op != "update" {
                    object.remove("id");
                }
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "capability_acquire".to_string(),
                arguments: payload,
            }])
        }
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
        ("custom_api", "delete") | ("custom_api", "enable") | ("custom_api", "disable") => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "operation".to_string(),
                    serde_json::Value::String(op.clone()),
                );
                object.remove("op");
                object.remove("kind");
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "custom_api_manage".to_string(),
                arguments: payload,
            }])
        }
        ("custom_api", _) => unsupported_resource_operation(
            "custom_api",
            op.as_str(),
            &[
                "create", "update", "install", "connect", "list", "read", "status", "test",
                "delete", "enable", "disable",
            ],
        ),
        ("custom_messaging_channel", "create") | ("custom_messaging_channel", "update") => {
            match custom_messaging_channel_payload_from_resource(arguments, op.as_str()) {
                Ok(payload) => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                    action_name: "custom_messaging_channel_upsert".to_string(),
                    arguments: payload,
                }]),
                Err(plan) => plan,
            }
        }
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
        ("custom_messaging_channel", _) => unsupported_resource_operation(
            "custom_messaging_channel",
            op.as_str(),
            &["create", "update", "list", "read", "status"],
        ),
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
        ("extension_pack", "create")
        | ("extension_pack", "update")
        | ("extension_pack", "install") => {
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "extension_pack_install".to_string(),
                arguments: merge_content_metadata(arguments),
            }])
        }
        ("extension_pack", "connect") => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "extension_pack_connect".to_string(),
            arguments: merge_content_metadata(arguments),
        }]),
        ("extension_pack", "enable") | ("extension_pack", "disable") => {
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "enabled".to_string(),
                    serde_json::Value::Bool(op == "enable"),
                );
            }
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "extension_pack_set_enabled".to_string(),
                arguments: payload,
            }])
        }
        ("extension_pack", "test") => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "extension_pack_test_connection".to_string(),
            arguments: merge_content_metadata(arguments),
        }]),
        ("extension_pack", _) => unsupported_resource_operation(
            "extension_pack",
            op.as_str(),
            &[
                "create", "update", "install", "connect", "list", "read", "status", "enable",
                "disable", "test",
            ],
        ),
        ("skill", "create")
        | ("skill", "update")
        | ("skill", "delete")
        | ("skill", "list")
        | ("skill", "read")
        | ("skill", "status")
        | ("skill", "install")
        | ("skill", "enable")
        | ("skill", "disable")
        | ("skill", "test") => PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
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
            let mut payload = merge_content_metadata(arguments);
            if let Some(object) = payload.as_object_mut() {
                let operation = match op.as_str() {
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
            PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
                action_name: "mcp_server_manage".to_string(),
                arguments: payload,
            }])
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
        "valid_kinds": RESOURCE_RW_VALID_KINDS,
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

fn custom_messaging_channel_payload_from_resource(
    arguments: &serde_json::Value,
    op: &str,
) -> Result<serde_json::Value, PrimitivePlan> {
    let mut payload = merge_content_metadata(arguments);
    merge_top_level_resource_fields(
        arguments,
        &mut payload,
        &[
            "name",
            "description",
            "enabled",
            "docs_url",
            "send",
            "auth_manifest",
            "auth_profile_id",
            "credential_fields",
            "clear_secrets",
        ],
    );
    if json_text(&payload, "name").is_none() || !payload.get("send").is_some_and(|v| v.is_object())
    {
        let mut extra = serde_json::json!({
            "kind": "custom_messaging_channel",
            "op": op,
            "field": "content.send",
            "hint": "Use kind=notification for delivering a user-facing notification. Use custom_messaging_channel only to configure a reusable non-bundled outbound channel."
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
            }
        }
        return Err(unsupported_with_extra(
            "resource_rw custom_messaging_channel create requires a reusable channel name and declarative send specification.",
            extra,
        ));
    }
    if let Some(object) = payload.as_object_mut() {
        object.remove("op");
        object.remove("kind");
    }
    Ok(payload)
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
        object
            .entry("task".to_string())
            .or_insert_with(|| serde_json::Value::String(message.clone()));
        object
            .entry("action".to_string())
            .or_insert_with(|| serde_json::Value::String("notify_user".to_string()));
        let action_is_notify_user = object
            .get("action")
            .and_then(|value| value.as_str())
            .is_none_or(|value| value.trim().eq_ignore_ascii_case("notify_user"));
        if action_is_notify_user {
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

fn scheduled_notification_invocation_from_direct_notification(
    invocation: &PrimitiveActionInvocation,
    request_text: &str,
    now_utc: chrono::DateTime<chrono::Utc>,
    default_timezone: Option<chrono_tz::Tz>,
) -> Option<PrimitiveActionInvocation> {
    if !invocation.action_name.eq_ignore_ascii_case("notify_user")
        || notification_payload_has_schedule(&invocation.arguments)
    {
        return None;
    }

    let hint = notification_schedule_hint_from_request(request_text, now_utc, default_timezone)?;
    let message = direct_notification_body_from_arguments(&invocation.arguments)?;
    let reminder_message = format!(
        "⏰ Reminder: {} scheduled for {} at {} {}",
        message, hint.date_label, hint.local_time, hint.timezone_abbrev
    );
    let mut arguments = serde_json::Map::new();
    arguments.insert("task".to_string(), serde_json::Value::String(message));
    arguments.insert(
        "local_date".to_string(),
        serde_json::Value::String(hint.local_date),
    );
    arguments.insert(
        "local_time".to_string(),
        serde_json::Value::String(hint.local_time),
    );
    arguments.insert(
        "timezone".to_string(),
        serde_json::Value::String(hint.timezone_name),
    );
    if let Some(route) = direct_notification_route_from_arguments(&invocation.arguments) {
        arguments.insert("report_to".to_string(), serde_json::Value::String(route));
    }
    arguments.insert(
        "action".to_string(),
        serde_json::Value::String("notify_user".to_string()),
    );
    arguments.insert(
        "action_arguments".to_string(),
        serde_json::json!({
            "message": reminder_message,
            "source": "reminder",
            "in_app_title": "Reminder"
        }),
    );
    if let Some(allow_duplicate) = invocation.arguments.get("allow_duplicate").cloned() {
        arguments.insert("allow_duplicate".to_string(), allow_duplicate);
    }

    Some(PrimitiveActionInvocation {
        action_name: "schedule_task".to_string(),
        arguments: serde_json::Value::Object(arguments),
    })
}

fn notification_schedule_hint_from_request(
    request_text: &str,
    now_utc: chrono::DateTime<chrono::Utc>,
    default_timezone: Option<chrono_tz::Tz>,
) -> Option<NotificationScheduleHint> {
    let local_time = NOTIFICATION_CLOCK_TIME_RE
        .captures_iter(request_text)
        .find_map(|captures| notification_clock_time_from_captures(&captures))?;
    let timezone = default_timezone.unwrap_or(chrono_tz::UTC);
    let now_local = now_utc.with_timezone(&timezone);
    let now_local_date = now_local.date_naive();
    let mut local_date = now_local_date;
    let mut scheduled_local =
        notification_resolve_local_datetime(timezone, local_date, local_time)?;
    if scheduled_local.with_timezone(&chrono::Utc) <= now_utc {
        local_date = now_local_date.succ_opt()?;
        scheduled_local = notification_resolve_local_datetime(timezone, local_date, local_time)?;
    }

    Some(NotificationScheduleHint {
        local_date: scheduled_local.date_naive().to_string(),
        local_time: format_notification_local_time(local_time),
        timezone_name: timezone.to_string(),
        timezone_abbrev: scheduled_local.format("%Z").to_string(),
        date_label: notification_schedule_date_label(now_local_date, scheduled_local.date_naive()),
    })
}

fn notification_clock_time_from_captures(
    captures: &regex::Captures<'_>,
) -> Option<chrono::NaiveTime> {
    if let (Some(hour), Some(meridiem)) = (captures.name("hour12"), captures.name("meridiem")) {
        let mut hour = hour.as_str().parse::<u32>().ok()?;
        let minute = captures
            .name("minute12")
            .map(|value| value.as_str().parse::<u32>().ok())
            .unwrap_or(Some(0))?;
        let meridiem = meridiem.as_str().to_ascii_lowercase();
        if meridiem.starts_with('p') && hour != 12 {
            hour += 12;
        } else if meridiem.starts_with('a') && hour == 12 {
            hour = 0;
        }
        return chrono::NaiveTime::from_hms_opt(hour, minute, 0);
    }

    let hour = captures.name("hour24")?.as_str().parse::<u32>().ok()?;
    let minute = captures.name("minute24")?.as_str().parse::<u32>().ok()?;
    let second = captures
        .name("second24")
        .map(|value| value.as_str().parse::<u32>().ok())
        .unwrap_or(Some(0))?;
    chrono::NaiveTime::from_hms_opt(hour, minute, second)
}

fn notification_resolve_local_datetime(
    timezone: chrono_tz::Tz,
    local_date: chrono::NaiveDate,
    local_time: chrono::NaiveTime,
) -> Option<chrono::DateTime<chrono_tz::Tz>> {
    match timezone.from_local_datetime(&local_date.and_time(local_time)) {
        chrono::LocalResult::Single(value) => Some(value),
        chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest),
        chrono::LocalResult::None => None,
    }
}

fn format_notification_local_time(time: chrono::NaiveTime) -> String {
    let hour24 = time.hour();
    let hour12 = match hour24 % 12 {
        0 => 12,
        value => value,
    };
    let meridiem = if hour24 < 12 { "AM" } else { "PM" };
    format!("{}:{:02} {}", hour12, time.minute(), meridiem)
}

fn notification_schedule_date_label(
    now_local_date: chrono::NaiveDate,
    scheduled_local_date: chrono::NaiveDate,
) -> String {
    if scheduled_local_date == now_local_date {
        "today".to_string()
    } else if now_local_date
        .succ_opt()
        .is_some_and(|tomorrow| scheduled_local_date == tomorrow)
    {
        "tomorrow".to_string()
    } else {
        scheduled_local_date.to_string()
    }
}

fn direct_notification_body_from_arguments(arguments: &serde_json::Value) -> Option<String> {
    json_text(arguments, "message")
        .or_else(|| json_text(arguments, "title"))
        .or_else(|| json_text(arguments, "task"))
}

fn direct_notification_route_from_arguments(arguments: &serde_json::Value) -> Option<String> {
    ["delivery_channel", "report_to", "channel", "recipient"]
        .iter()
        .find_map(|key| json_text(arguments, key))
}

fn latest_user_message_text(messages: &[SpineMessage]) -> Option<&str> {
    messages.iter().rev().find_map(|message| match message {
        SpineMessage::User { content } => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        _ => None,
    })
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
    {
        return PrimitivePlan::Actions(vec![PrimitiveActionInvocation {
            action_name: "capability_acquire".to_string(),
            arguments: payload,
        }]);
    }

    unsupported_with_extra(
        "resource_rw integration create/update/install/connect requires structured integration content.",
        serde_json::json!({
            "kind": "integration",
            "op": op,
            "hint": "Use a specific resource kind when the substrate is explicit: mcp_server for MCP transports, custom_api for HTTP/API operations, custom_messaging_channel for send specs, or extension_pack for pack manifests/catalog installs. For custom APIs provide base_url/path or openapi_url/openapi_text; do not infer an MCP server from a generic integration request."
        }),
    )
}

fn validate_file_mutation_payload(
    payload: &serde_json::Value,
    op: &str,
    has_patch: bool,
) -> Option<PrimitivePlan> {
    let has_path = json_text(payload, "path").is_some();
    let has_batch_patches = payload.get("patches").is_some();
    let has_complete_body = file_mutation_has_complete_body(payload);

    if has_batch_patches {
        let patches = payload.get("patches").and_then(|value| value.as_array());
        let every_patch_has_path = patches
            .map(|items| {
                !items.is_empty() && items.iter().all(|item| json_text(item, "path").is_some())
            })
            .unwrap_or(false);
        if every_patch_has_path {
            return None;
        }
        return Some(unsupported_with_extra(
            "resource_rw file patch updates with `content.patches` require each patch entry to include `path`.",
            serde_json::json!({
                "kind": "file",
                "op": op,
                "field": "content.patches[].path",
                "hint": "Provide a workspace/data-relative path on each patch entry."
            }),
        ));
    }

    if has_path && (has_patch || has_complete_body) {
        return None;
    }

    if has_path && !has_complete_body {
        return Some(unsupported_with_extra(
            "resource_rw file create/update requires a file body source.",
            serde_json::json!({
                "kind": "file",
                "op": op,
                "field": "content.content",
                "hint": "Include one body source in the same call: content.content with the complete file body string, content.content_base64, content.source_path, or content.source_resource."
            }),
        ));
    }

    let field = "content.path";
    let reason = if has_patch {
        "resource_rw file patch updates require `content.path` for the file being patched."
    } else {
        "resource_rw file create/update requires `content.path` for the destination file."
    };
    Some(unsupported_with_extra(
        reason,
        serde_json::json!({
            "kind": "file",
            "op": op,
            "field": field,
            "hint": "Use a workspace/data-relative path such as reports/comparison.html."
        }),
    ))
}

fn file_mutation_has_complete_body(payload: &serde_json::Value) -> bool {
    [
        "content",
        "content_base64",
        "source_path",
        "source_resource",
    ]
    .iter()
    .any(|key| match payload.get(*key) {
        Some(serde_json::Value::String(value)) => !value.trim().is_empty(),
        Some(serde_json::Value::Object(object)) => !object.is_empty(),
        Some(serde_json::Value::Array(items)) => !items.is_empty(),
        Some(serde_json::Value::Number(_)) | Some(serde_json::Value::Bool(_)) => true,
        _ => false,
    })
}

fn file_payload_should_deploy_as_app(payload: &serde_json::Value) -> bool {
    if json_bool(payload, "file_only")
        .or_else(|| json_bool(payload, "raw_file"))
        .unwrap_or(false)
    {
        return false;
    }
    let Some(body) = json_text(payload, "content") else {
        return false;
    };
    if body.trim().is_empty() {
        return false;
    }
    let media_type = json_text(payload, "content_type")
        .or_else(|| json_text(payload, "mime"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = json_text(payload, "path")
        .unwrap_or_default()
        .to_ascii_lowercase();
    text_looks_like_html_document(&body)
        && (media_type.contains("html") || file_path_is_html_entry(&path))
}

fn app_service_payload_from_html_file_payload(payload: &serde_json::Value) -> serde_json::Value {
    let body = json_text(payload, "content").unwrap_or_default();
    let path = json_text(payload, "path").unwrap_or_else(|| "index.html".to_string());
    let title = json_text(payload, "title")
        .or_else(|| json_text(payload, "name"))
        .or_else(|| {
            path.rsplit('/')
                .next()
                .map(|name| name.trim_end_matches(".html").trim_end_matches(".htm"))
                .filter(|name| !name.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "Generated HTML page".to_string());
    let mut files = serde_json::Map::new();
    files.insert("index.html".to_string(), serde_json::Value::String(body));

    let mut out = serde_json::Map::new();
    out.insert(
        "operation".to_string(),
        serde_json::Value::String("create".to_string()),
    );
    out.insert(
        "kind".to_string(),
        serde_json::Value::String("static".to_string()),
    );
    out.insert("name".to_string(), serde_json::Value::String(title));
    out.insert("files".to_string(), serde_json::Value::Object(files));

    let mut metadata = serde_json::Map::new();
    metadata.insert("original_path".to_string(), serde_json::Value::String(path));
    for key in [
        "source_url",
        "source_fingerprint",
        "artifact_identity",
        "title",
        "description",
        "tags",
        "category",
        "stale_at",
    ] {
        if let Some(value) = payload.get(key).cloned() {
            metadata.insert(key.to_string(), value);
        }
    }
    out.insert("metadata".to_string(), serde_json::Value::Object(metadata));
    serde_json::Value::Object(out)
}

fn plan_memory_rw(arguments: &serde_json::Value) -> PrimitivePlan {
    let op = json_text(arguments, "op")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
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
        "operation",
        "operation_id",
        "action_name",
        "body",
        "integration",
        "duplicate_policy",
        "allow_duplicate",
    ] {
        if let Some(value) = arguments.get(key) {
            out.entry(key.to_string()).or_insert_with(|| value.clone());
        }
    }
    serde_json::Value::Object(out)
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

fn service_manage_payload_has_deploy_material(payload: &serde_json::Value) -> bool {
    let Some(object) = payload.as_object() else {
        return false;
    };
    object
        .get("files")
        .and_then(|value| value.as_object())
        .is_some_and(|files| {
            service_files_have_browser_entry(files)
                || service_payload_has_runtime_entrypoint(object)
        })
        || (object
            .get("source_dir")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            && object
                .get("source_paths")
                .and_then(|value| value.as_array())
                .is_some_and(|paths| !paths.is_empty()))
        || object
            .get("repo_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || object
            .get("file_patches")
            .and_then(|value| value.as_array())
            .is_some_and(|patches| !patches.is_empty())
        || object
            .get("delete_paths")
            .and_then(|value| value.as_array())
            .is_some_and(|paths| !paths.is_empty())
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

fn service_payload_has_runtime_entrypoint(
    object: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    ["entry_command", "start_command"].iter().any(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    })
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

fn spine_tool_result_value_for_model(
    primitive: &str,
    action_name: &str,
    content: String,
) -> serde_json::Value {
    if action_name == "service_manage" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) {
            return sanitize_service_manage_result_for_model(primitive, &value);
        }
    }

    if let Some(value) = parse_structured_tool_completion_for_model(&content) {
        if action_name == "service_manage" {
            let data = value.get("data").unwrap_or(&value);
            return sanitize_service_manage_result_for_model(primitive, data);
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

fn sanitize_structured_tool_result_for_model(
    primitive: &str,
    action_name: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    let mut out = value.clone();
    compact_model_visible_tool_value(&mut out);
    if let Some(object) = out.as_object_mut() {
        object
            .entry("ok".to_string())
            .or_insert_with(|| serde_json::Value::Bool(true));
        object
            .entry("primitive".to_string())
            .or_insert_with(|| serde_json::Value::String(primitive.to_string()));
        object
            .entry("tool".to_string())
            .or_insert_with(|| serde_json::Value::String(action_name.to_string()));
    }
    out
}

fn compact_model_visible_tool_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if matches!(
                    key.as_str(),
                    "content" | "body_text" | "text" | "markdown" | "html"
                ) {
                    if let Some(text) = value.as_str() {
                        if text.chars().count() > 8_000 {
                            *value =
                                serde_json::Value::String(head_tail_excerpt(text, 6_500, 1_500));
                        }
                    }
                } else if key == "image_base64" {
                    if let Some(text) = value.as_str() {
                        *value = serde_json::Value::String(format!(
                            "[base64 omitted: {} chars]",
                            text.chars().count()
                        ));
                    }
                } else {
                    compact_model_visible_tool_value(value);
                }
            }
            if let Some(results) = object
                .get_mut("results")
                .and_then(|value| value.as_array_mut())
            {
                if results.len() > 5 {
                    results.truncate(5);
                }
                for result in results {
                    compact_model_visible_tool_value(result);
                }
            }
            if let Some(attempts) = object
                .get_mut("attempts")
                .and_then(|value| value.as_array_mut())
            {
                if attempts.len() > 3 {
                    attempts.truncate(3);
                }
                for attempt in attempts {
                    compact_model_visible_tool_value(attempt);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                compact_model_visible_tool_value(item);
            }
        }
        _ => {}
    }
}

fn head_tail_excerpt(text: &str, head_chars: usize, tail_chars: usize) -> String {
    let total = text.chars().count();
    if total <= head_chars + tail_chars {
        return text.to_string();
    }
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!(
        "{}\n...[omitted {} chars]...\n{}",
        head,
        total.saturating_sub(head_chars + tail_chars),
        tail
    )
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
        "message": message,
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

fn stale_final_response_repair_prompt() -> String {
    "The previous assistant message repeated content that was attached to an earlier tool-call turn instead of answering from the latest tool results. Provide the final answer now, grounded in the latest tool results. If the result is inconclusive or failed, state that clearly and include the blocker."
        .to_string()
}

fn no_progress_final_response(messages: &[SpineMessage], final_text: &str) -> Option<String> {
    if final_text.trim().is_empty() {
        return None;
    }
    let last_tool_content = messages.iter().rev().find_map(|message| {
        if let SpineMessage::Tool { content, .. } = message {
            Some(content)
        } else {
            None
        }
    })?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(last_tool_content) else {
        return None;
    };
    let error = value.get("error").and_then(|item| item.as_str());
    if error != Some("repeated_no_progress_tool_call") {
        return None;
    }
    let message = value
        .get("message")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("The last tool call repeated work that already produced no new progress.");
    Some(format!(
        "I could not complete this because the latest tool step made no new progress. {} Use the previous tool result, inspect the saved state, or call a different operation with different arguments before retrying.",
        message
    ))
}

fn needs_input_message_from_tool_results(results: &[ToolResult]) -> Option<String> {
    results.iter().find_map(|result| {
        let outcome = super::tool_responses::structured_tool_value_outcome(&result.value)?;
        if outcome.state != super::tool_responses::StructuredToolOutcomeState::NeedsInput {
            return None;
        }
        needs_input_message_from_tool_value(&result.value).or(outcome.message)
    })
}

fn needs_input_message_from_tool_value(value: &serde_json::Value) -> Option<String> {
    json_text_path(value, &["data", "session", "question"])
        .or_else(|| json_text_path(value, &["data", "question"]))
        .or_else(|| json_text(value, "question"))
        .or_else(|| json_text(value, "detail"))
        .or_else(|| json_text(value, "message"))
        .or_else(|| Some("I need your input before I can continue this browser task.".to_string()))
}

fn incomplete_no_tool_final_response_repair_prompt() -> String {
    "The previous assistant message was an unfinished lead-in and did not call a tool or provide a final answer. Continue from the current state: call the required tool now, or give a concrete blocker grounded in the available evidence."
        .to_string()
}

fn incomplete_no_tool_final_response_blocker(final_text: &str) -> String {
    let lead_in = final_text.trim();
    if lead_in.is_empty() {
        return "I could not complete this because the model stopped without calling a tool or giving a final answer."
            .to_string();
    }
    format!(
        "I could not complete this because the model stopped after an unfinished lead-in without calling a tool or giving a final answer: {}",
        lead_in
    )
}

fn incomplete_no_tool_final_response(messages: &[SpineMessage], final_text: &str) -> bool {
    let trimmed = final_text.trim();
    if trimmed.is_empty() || trimmed.lines().count() != 1 {
        return false;
    }
    if trimmed.chars().count() > 240 {
        return false;
    }
    let follows_live_context = messages.iter().rev().any(|message| {
        matches!(
            message,
            SpineMessage::User { .. } | SpineMessage::Tool { .. }
        )
    });
    if !follows_live_context {
        return false;
    }
    if trimmed.ends_with(':') {
        return true;
    }
    substantial_tool_run_before_final(messages) && !final_text_has_delivery_reference(trimmed)
}

fn substantial_tool_run_before_final(messages: &[SpineMessage]) -> bool {
    let mut assistant_tool_turns = 0usize;
    let mut requested_tool_calls = 0usize;
    let mut completed_tool_results = 0usize;

    for message in messages {
        match message {
            SpineMessage::Assistant { tool_calls, .. } if !tool_calls.is_empty() => {
                assistant_tool_turns += 1;
                requested_tool_calls += tool_calls.len();
            }
            SpineMessage::Tool { .. } => {
                completed_tool_results += 1;
            }
            _ => {}
        }
    }

    assistant_tool_turns >= 2 || requested_tool_calls >= 4 || completed_tool_results >= 4
}

fn final_text_has_delivery_reference(text: &str) -> bool {
    text.contains("](")
        || text.contains("://")
        || text.contains("/apps/")
        || text.contains("/api/outputs/")
        || text.contains("/ui/")
}

fn final_response_repeats_tool_call_content(messages: &[SpineMessage], final_text: &str) -> bool {
    let final_normalized = normalize_text_for_repetition_check(final_text);
    if final_normalized.len() < 12 {
        return false;
    }

    messages.iter().any(|message| {
        let SpineMessage::Assistant {
            content,
            tool_calls,
        } = message
        else {
            return false;
        };
        if tool_calls.is_empty() {
            return false;
        }
        let Some(content) = content.as_deref() else {
            return false;
        };
        let prior_normalized = normalize_text_for_repetition_check(content);
        texts_are_near_duplicates(&final_normalized, &prior_normalized)
    })
}

fn normalize_text_for_repetition_check(text: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_space = true;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            normalized.push(ch);
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn texts_are_near_duplicates(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }

    let left_len = left.chars().count();
    let right_len = right.chars().count();
    let shorter = left_len.min(right_len) as f64;
    let longer = left_len.max(right_len) as f64;
    if shorter / longer < 0.8 {
        return false;
    }

    let left_tokens = left.split_whitespace().collect::<Vec<_>>();
    let right_tokens = right.split_whitespace().collect::<Vec<_>>();
    if left_tokens.len().min(right_tokens.len()) < 4 {
        return false;
    }

    let left_set = left_tokens
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let right_set = right_tokens
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let intersection = left_set.intersection(&right_set).count() as f64;
    let union = left_set.union(&right_set).count() as f64;
    union > 0.0 && intersection / union >= 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_registry_has_exactly_seven_tools() {
        let registry = ToolRegistry::new();
        let names = registry
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert_eq!(names, PRIMITIVE_NAMES);
    }

    #[test]
    fn spine_prompt_prioritizes_user_provided_primary_sources() {
        let prompt = build_spine_system_prompt("", None, None);
        assert!(prompt.contains("user-provided URLs and documents as primary evidence"));
        assert!(prompt.contains("instead of fetching secondary sources"));
        assert!(
            prompt
                .contains("do not let older secondary material override current primary evidence")
        );
        assert!(prompt.contains("Do not send description-only file creates"));
        assert!(prompt.contains("Finish every requested deliverable before the final answer"));
        assert!(prompt.contains("lead with one natural confirmation sentence"));
        assert!(prompt.contains("follow with compact details"));
        assert!(
            prompt
                .to_ascii_lowercase()
                .contains("do not introduce a formal summary block")
        );
        assert!(prompt.contains("do not add generic filler follow-up questions"));
        assert!(prompt.contains("accessible /apps/ URL"));
        assert!(prompt.contains("Do not return container paths"));
        assert!(prompt.contains("do not expose internal container filesystem paths"));
        assert!(prompt.contains("Documents surface"));
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

        assert!(
            fragments[..first_evolvable]
                .iter()
                .all(|fragment| !fragment.evolvable)
        );
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
    }

    #[test]
    fn spine_prompt_bundle_uses_only_allowed_active_fragment_overrides() {
        let mut fragment_bundle = crate::core::prompt_fragments::default_prompt_fragment_bundle();
        fragment_bundle.version = "spine-fragments-test-v2".to_string();
        fragment_bundle
            .fragments
            .push(crate::core::prompt_fragments::PromptFragment {
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
            .push(crate::core::prompt_fragments::PromptFragment {
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
    fn incomplete_no_tool_final_response_flags_short_multistep_tool_run() {
        let messages = vec![
            SpineMessage::User {
                content: "Create a researched PDF report comparing agent platforms.".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Gathering source evidence.".to_string()),
                tool_calls: vec![
                    test_tool_call("call-search-1", "search"),
                    test_tool_call("call-search-2", "search"),
                    test_tool_call("call-search-3", "search"),
                ],
            },
            SpineMessage::Tool {
                tool_call_id: "call-search-1".to_string(),
                content: serde_json::json!({"ok": true, "results": ["source-a"]}).to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-search-2".to_string(),
                content: serde_json::json!({"ok": true, "results": ["source-b"]}).to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-search-3".to_string(),
                content: serde_json::json!({"ok": true, "results": ["source-c"]}).to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Checking the strongest sources.".to_string()),
                tool_calls: vec![
                    test_tool_call("call-fetch-1", "fetch"),
                    test_tool_call("call-fetch-2", "fetch"),
                    test_tool_call("call-fetch-3", "fetch"),
                ],
            },
            SpineMessage::Tool {
                tool_call_id: "call-fetch-1".to_string(),
                content: serde_json::json!({"ok": true, "text": "evidence one"}).to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-fetch-2".to_string(),
                content: serde_json::json!({"ok": true, "text": "evidence two"}).to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-fetch-3".to_string(),
                content: serde_json::json!({"ok": true, "text": "evidence three"}).to_string(),
            },
        ];

        assert!(incomplete_no_tool_final_response(
            &messages,
            "I need one more pass before the final synthesis."
        ));
    }

    #[test]
    fn incomplete_no_tool_final_response_allows_short_delivery_reference() {
        let messages = vec![
            SpineMessage::Assistant {
                content: Some("Saving the finished document.".to_string()),
                tool_calls: vec![
                    test_tool_call("call-search-1", "search"),
                    test_tool_call("call-fetch-1", "fetch"),
                    test_tool_call("call-pdf-1", "pdf_generate"),
                    test_tool_call("call-fetch-2", "fetch"),
                ],
            },
            SpineMessage::Tool {
                tool_call_id: "call-search-1".to_string(),
                content: serde_json::json!({"ok": true}).to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-fetch-1".to_string(),
                content: serde_json::json!({"ok": true}).to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-pdf-1".to_string(),
                content: serde_json::json!({
                    "ok": true,
                    "artifact": {
                        "download_url": "/api/outputs/report.pdf/download"
                    }
                })
                .to_string(),
            },
            SpineMessage::Tool {
                tool_call_id: "call-fetch-2".to_string(),
                content: serde_json::json!({"ok": true}).to_string(),
            },
        ];

        assert!(!incomplete_no_tool_final_response(
            &messages,
            "Saved the PDF: /api/outputs/report.pdf/download"
        ));
    }

    fn test_tool_call(id: &str, name: &str) -> SpineToolCall {
        SpineToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: serde_json::json!({}),
        }
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
    fn spine_tool_result_stream_content_exposes_sanitized_result_preview() {
        let call = SpineToolCall {
            id: "call-fetch-1".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({ "url": "https://example.com" }),
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
        assert!(
            payload["summary"]
                .as_str()
                .expect("summary should be present")
                .contains("Agent memory report")
        );
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
            ..Default::default()
        };

        let context = structured_chat_request_context_system_message(&hints)
            .expect("structured request context should be present");

        assert!(context.contains("upload-1"));
        assert!(context.contains("active_orbit_id"));
        assert!(context.contains("orbit-1"));
        assert!(context.contains("browser_profile_context"));
        assert!(context.contains("profile-1"));
        assert!(context.contains("alex"));
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

        let context = structured_chat_request_context_system_message(&hints)
            .expect("artifact context should be present");

        assert!(context.contains("recent_actionable_artifacts"));
        assert!(context.contains("app-123"));
        assert!(context.contains("service_manage"));
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
        let fragment_bundle = crate::core::prompt_fragments::default_prompt_fragment_bundle();
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
        assert!(
            telemetry["prompt_fragment_version"]
                .as_str()
                .unwrap_or_default()
                .contains("prompt_fragments_v1")
        );
        assert!(
            telemetry["sections"]["spine.source_grounding_policy"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(
            telemetry["spine_prompt_fragments"]
                .as_array()
                .map(|fragments| fragments
                    .iter()
                    .any(|fragment| fragment["id"] == "spine.final_answer_policy"))
                .unwrap_or(false)
        );
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
        let sanitized = spine_tool_result_value_for_model("resource_rw", "file_write", raw);
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
        assert!(
            sanitized["assistant_instruction"]
                .as_str()
                .unwrap_or_default()
                .contains("answer from this result")
        );
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
        assert!(
            sanitized["message"]
                .as_str()
                .unwrap_or_default()
                .contains("not found")
        );
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
    fn prepared_messages_do_not_leak_tool_call_json_as_dialogue() {
        let messages = vec![
            SpineMessage::System {
                content: "system".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("Let me fetch that.".to_string()),
                tool_calls: vec![SpineToolCall {
                    id: "call_1".to_string(),
                    name: "fetch".to_string(),
                    arguments: serde_json::json!({"url": "https://example.com"}),
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
        assert!(joined.contains("Let me fetch that."));
        assert!(prepared.user_message.contains("Tool result for `call_1`"));
        assert!(!joined.contains("Tool calls requested by the prior model turn"));
        assert!(
            !prepared
                .user_message
                .contains("Tool calls requested by the prior model turn")
        );
        assert!(!joined.contains("\"name\":\"fetch\""));
        assert!(!prepared.user_message.contains("\"name\":\"fetch\""));
    }

    #[test]
    fn resource_file_write_plans_from_structured_fields() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {"path": "notes.txt", "content": "hello"}
        }));
        match plan {
            PrimitivePlan::Actions(actions) => assert_eq!(actions[0].action_name, "file_write"),
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
        assert!(
            kinds
                .iter()
                .any(|kind| kind.as_str() == Some("notification"))
        );

        let kind_description = schema.input_schema["properties"]["kind"]["description"]
            .as_str()
            .expect("kind description should be present");
        assert!(kind_description.contains("notification for user-facing notification delivery"));
        assert!(kind_description.contains("custom_messaging_channel"));
    }

    #[test]
    fn resource_file_write_requires_content_path() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "id": "reports/comparison.html",
            "content": {"content": "<!doctype html><title>Report</title>"}
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("content.path"));
                assert_eq!(extra.unwrap()["field"], "content.path");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_file_write_requires_file_body() {
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
                assert!(reason.contains("file body"));
                assert_eq!(extra.unwrap()["field"], "content.content");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_html_file_create_routes_to_app_service() {
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
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "service_manage");
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(actions[0].arguments["kind"], "static");
                assert_eq!(actions[0].arguments["name"], "Comparison Report");
                assert_eq!(
                    actions[0].arguments["files"]["index.html"],
                    "<!doctype html><html><body>Report</body></html>"
                );
                assert_eq!(
                    actions[0].arguments["metadata"]["original_path"],
                    "reports/comparison.html"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_html_path_without_html_document_stays_file_write() {
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
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "file_write");
                assert_eq!(actions[0].arguments["document_visible"], true);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_markdown_file_create_stays_file_write() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "file",
            "content": {
                "path": "reports/runbook.md",
                "content": "# Runbook\n\nSteps."
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "file_write");
                assert_eq!(actions[0].arguments["document_visible"], true);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn final_response_repeats_tool_call_content_detects_stale_intermediate_text() {
        let messages = vec![
            SpineMessage::User {
                content: "Check whether the provider API works.".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("I will check the provider API now.".to_string()),
                tool_calls: vec![SpineToolCall {
                    id: "call_1".to_string(),
                    name: "resource_rw".to_string(),
                    arguments: serde_json::json!({"kind": "custom_api", "op": "status"}),
                }],
            },
            SpineMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: serde_json::json!({"ok": true, "status": "connected"}).to_string(),
            },
        ];

        assert!(final_response_repeats_tool_call_content(
            &messages,
            "I will check the provider API now."
        ));
        assert!(final_response_repeats_tool_call_content(
            &messages,
            "  i will check the provider api now  "
        ));
    }

    #[test]
    fn final_response_repeats_tool_call_content_allows_new_terminal_answer() {
        let messages = vec![
            SpineMessage::User {
                content: "Check whether the provider API works.".to_string(),
            },
            SpineMessage::Assistant {
                content: Some("I will check the provider API now.".to_string()),
                tool_calls: vec![SpineToolCall {
                    id: "call_1".to_string(),
                    name: "resource_rw".to_string(),
                    arguments: serde_json::json!({"kind": "custom_api", "op": "status"}),
                }],
            },
            SpineMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: serde_json::json!({"ok": true, "status": "connected"}).to_string(),
            },
        ];

        assert!(!final_response_repeats_tool_call_content(
            &messages,
            "The provider API status check completed. It is connected."
        ));
    }

    #[test]
    fn incomplete_no_tool_final_response_detects_unfinished_lead_in() {
        let messages = vec![SpineMessage::User {
            content: "Check whether the saved integration works.".to_string(),
        }];

        assert!(incomplete_no_tool_final_response(
            &messages,
            "Let me run an actual connectivity test:"
        ));
        assert!(!incomplete_no_tool_final_response(
            &messages,
            "The saved integration test failed with HTTP 400."
        ));
    }

    #[test]
    fn final_response_after_no_progress_tool_result_is_forced_to_blocker() {
        let messages = vec![
            SpineMessage::Assistant {
                content: Some("I will update the saved API operation.".to_string()),
                tool_calls: vec![SpineToolCall {
                    id: "call_1".to_string(),
                    name: "resource_rw".to_string(),
                    arguments: serde_json::json!({
                        "kind": "custom_api",
                        "op": "update",
                        "id": "provider-api"
                    }),
                }],
            },
            SpineMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: serde_json::json!({
                    "ok": false,
                    "error": "repeated_no_progress_tool_call",
                    "message": "This exact tool request already completed."
                })
                .to_string(),
            },
        ];

        let replacement = no_progress_final_response(&messages, "I will try a different approach.");

        assert!(replacement.is_some());
        assert!(!replacement.unwrap().contains("I will try"));
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
    fn resource_file_batch_patch_accepts_entry_paths() {
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
            PrimitivePlan::Actions(actions) => assert_eq!(actions[0].action_name, "file_patch"),
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_create_maps_to_deploy_operation() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "GPU pricing comparison",
                "files": {"index.html": "<!doctype html><title>GPU pricing</title>"}
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "service_manage");
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(actions[0].arguments["kind"], "auto");
                assert!(actions[0].arguments["files"].is_object());
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
    fn resource_generic_integration_with_mcp_like_fields_requires_explicit_substrate() {
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
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("structured integration content"));
                let extra = extra.unwrap();
                assert_eq!(extra["kind"], "integration");
                assert_eq!(extra["op"], "install");
                assert!(extra["hint"].as_str().unwrap().contains("mcp_server"));
                assert!(extra["hint"].as_str().unwrap().contains("custom_api"));
            }
            PrimitivePlan::Actions(actions) => {
                panic!(
                    "generic integration install inferred {}",
                    actions[0].action_name
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
    fn resource_custom_messaging_channel_create_requires_channel_definition() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "custom_messaging_channel",
            "content": {
                "message": "Meeting with Mark",
                "channel": "telegram"
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("custom_messaging_channel create requires"));
                assert_eq!(extra.unwrap()["kind"], "custom_messaging_channel");
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
                assert!(
                    extra["hint"]
                        .as_str()
                        .unwrap()
                        .contains("kind=notification")
                );
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
    fn resource_skill_import_maps_to_generic_skill_management() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "install",
            "kind": "skill",
            "content": {
                "url": "https://example.com/skills/SKILL.md",
                "name": "source-checker"
            }
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
    fn direct_notification_with_clock_time_in_request_is_repaired_to_schedule_task() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-27T13:27:00+05:30")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let invocation = PrimitiveActionInvocation {
            action_name: "notify_user".to_string(),
            arguments: serde_json::json!({
                "title": "Meeting with Mark",
                "message": "Meeting with Mark",
                "delivery_channel": "telegram"
            }),
        };

        let repaired = scheduled_notification_invocation_from_direct_notification(
            &invocation,
            "send me a notificaiton in telegram for meeting with mark at 1:30 PM today",
            now,
            Some(chrono_tz::Asia::Kolkata),
        )
        .expect("time-bearing notification request should schedule instead of sending now");

        assert_eq!(repaired.action_name, "schedule_task");
        assert_eq!(repaired.arguments["task"], "Meeting with Mark");
        assert_eq!(repaired.arguments["local_time"], "1:30 PM");
        assert_eq!(repaired.arguments["timezone"], "Asia/Kolkata");
        assert_eq!(repaired.arguments["report_to"], "telegram");
        assert_eq!(repaired.arguments["action"], "notify_user");
        assert_eq!(
            repaired.arguments["action_arguments"]["message"],
            "⏰ Reminder: Meeting with Mark scheduled for today at 1:30 PM IST"
        );
    }

    #[test]
    fn direct_notification_clock_time_repair_preserves_arbitrary_delivery_route() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-27T13:27:00+05:30")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let invocation = PrimitiveActionInvocation {
            action_name: "notify_user".to_string(),
            arguments: serde_json::json!({
                "message": "Meeting with Mark",
                "delivery_channel": "whatsapp"
            }),
        };

        let repaired = scheduled_notification_invocation_from_direct_notification(
            &invocation,
            "remind me on whatsapp about meeting with mark at 1:30 PM",
            now,
            Some(chrono_tz::Asia::Kolkata),
        )
        .expect("clock-time notification request should preserve any delivery route");

        assert_eq!(repaired.action_name, "schedule_task");
        assert_eq!(repaired.arguments["report_to"], "whatsapp");
        assert_eq!(
            repaired.arguments["action_arguments"]["message"],
            "⏰ Reminder: Meeting with Mark scheduled for today at 1:30 PM IST"
        );
    }

    #[test]
    fn direct_notification_without_clock_time_is_not_repaired_to_schedule_task() {
        let invocation = PrimitiveActionInvocation {
            action_name: "notify_user".to_string(),
            arguments: serde_json::json!({
                "message": "Meeting with Mark",
                "delivery_channel": "telegram"
            }),
        };

        let repaired = scheduled_notification_invocation_from_direct_notification(
            &invocation,
            "send me a notificaiton in telegram for meeting with mark",
            chrono::Utc::now(),
            Some(chrono_tz::Asia::Kolkata),
        );

        assert!(repaired.is_none());
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
    fn resource_app_service_content_string_becomes_index_html_file() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Pricing report",
                "content": "<!doctype html><html><body>Report</body></html>"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "service_manage");
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(
                    actions[0].arguments["files"]["index.html"],
                    "<!doctype html><html><body>Report</body></html>"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_plain_text_content_is_not_deploy_material() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Reusable workflow",
                "content": "# Workflow\n\nFetch source, extract table, publish result."
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("deployable app content"));
                assert_eq!(extra.unwrap()["field"], "content.files");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_markdown_files_are_not_deploy_material() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Documentation bundle",
                "files": {
                    "runbook.md": "# Workflow\n\nRepeatable steps."
                }
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("deployable app content"));
                assert_eq!(extra.unwrap()["field"], "content.files");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_structural_html_field_becomes_index_entry() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Generated app",
                "source": "<!doctype html><html><body>App</body></html>"
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "service_manage");
                assert_eq!(
                    actions[0].arguments["files"]["index.html"],
                    "<!doctype html><html><body>App</body></html>"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_html_file_body_without_html_path_gets_entrypoint() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Generated app",
                "files": {
                    "main": "<!doctype html><html><body>App</body></html>"
                }
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].action_name, "service_manage");
                assert_eq!(
                    actions[0].arguments["files"]["index.html"],
                    "<!doctype html><html><body>App</body></html>"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_create_requires_deploy_material() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Empty app"
            }
        }));
        match plan {
            PrimitivePlan::Unsupported { reason, extra } => {
                assert!(reason.contains("deployable app content"));
                assert_eq!(extra.unwrap()["field"], "content.files");
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_normalizes_nested_file_entries() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "name": "Nested files",
                "files": {
                    "index.html": {"content": "<!doctype html><title>Nested</title>"}
                }
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(
                    actions[0].arguments["files"]["index.html"],
                    "<!doctype html><title>Nested</title>"
                );
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn resource_app_service_preserves_explicit_service_kind() {
        let plan = plan_resource_rw(&serde_json::json!({
            "op": "create",
            "kind": "app_service",
            "content": {
                "kind": "static",
                "files": {"index.html": "<!doctype html><title>Report</title>"}
            }
        }));
        match plan {
            PrimitivePlan::Actions(actions) => {
                assert_eq!(actions[0].arguments["operation"], "create");
                assert_eq!(actions[0].arguments["kind"], "static");
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
        };

        let first = tool_call_progress_signature(&first).expect("signature");
        let second = tool_call_progress_signature(&second).expect("signature");

        assert_eq!(first.class, ToolProgressClass::Mutation);
        assert_eq!(first.key, second.key);
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
        let plan = plan_memory_rw(&serde_json::json!({
            "op": "write",
            "explicit_user_request": true,
            "content": {"key": "preferred_model", "value": "fast"}
        }));
        assert!(matches!(
            plan,
            PrimitivePlan::Memory(MemoryPrimitiveOp::Write { .. })
        ));
    }

    #[test]
    fn memory_write_requires_active_memory_management_intent() {
        let plan = plan_memory_rw(&serde_json::json!({
            "op": "write",
            "content": {"key": "personal_fact", "value": "durable user-provided fact"}
        }));
        assert!(matches!(plan, PrimitivePlan::Unsupported { .. }));
    }
}
